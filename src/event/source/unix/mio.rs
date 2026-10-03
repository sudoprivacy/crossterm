use std::{collections::VecDeque, io, time::Duration};

use mio::{Events, Interest, Poll, Token, unix::SourceFd};
use signal_hook_mio::v1_0::Signals;

#[cfg(feature = "event-stream")]
use crate::event::sys::Waker;
use crate::event::{
    Event, internal::InternalEvent, source::EventSource, sys::parse::Parser, timeout::PollTimeout,
};
use crate::terminal::sys::file_descriptor::{FileDesc, tty_fd};

// Tokens to identify file descriptor
const TTY_TOKEN: Token = Token(0);
const SIGNAL_TOKEN: Token = Token(1);
#[cfg(feature = "event-stream")]
const WAKE_TOKEN: Token = Token(2);

// I (@zrzka) wasn't able to read more than 1_022 bytes when testing
// reading on macOS/Linux -> we don't need bigger buffer and 1k of bytes
// is enough.
const TTY_BUFFER_SIZE: usize = 1_024;

pub(crate) struct UnixInternalEventSource {
    poll: Poll,
    events: Events,
    pending_tokens: VecDeque<Token>,
    parser: Parser,
    tty_buffer: [u8; TTY_BUFFER_SIZE],
    tty_fd: FileDesc<'static>,
    signals: Signals,
    #[cfg(feature = "event-stream")]
    waker: Waker,
}

impl UnixInternalEventSource {
    pub fn new() -> io::Result<Self> {
        UnixInternalEventSource::from_file_descriptor(tty_fd()?)
    }

    pub(crate) fn from_file_descriptor(input_fd: FileDesc<'static>) -> io::Result<Self> {
        let poll = Poll::new()?;
        let registry = poll.registry();

        let tty_raw_fd = input_fd.raw_fd();
        let mut tty_ev = SourceFd(&tty_raw_fd);
        registry.register(&mut tty_ev, TTY_TOKEN, Interest::READABLE)?;

        let mut signals = Signals::new([signal_hook::consts::SIGWINCH])?;
        registry.register(&mut signals, SIGNAL_TOKEN, Interest::READABLE)?;

        #[cfg(feature = "event-stream")]
        let waker = Waker::new(registry, WAKE_TOKEN)?;

        Ok(UnixInternalEventSource {
            poll,
            events: Events::with_capacity(3),
            pending_tokens: VecDeque::with_capacity(3),
            parser: Parser::default(),
            tty_buffer: [0u8; TTY_BUFFER_SIZE],
            tty_fd: input_fd,
            signals,
            #[cfg(feature = "event-stream")]
            waker,
        })
    }
}

impl EventSource for UnixInternalEventSource {
    fn try_read(&mut self, timeout: Option<Duration>) -> io::Result<Option<InternalEvent>> {
        if let Some(event) = self.parser.next() {
            return Ok(Some(event));
        }

        let timeout = PollTimeout::new(timeout);

        loop {
            // Readiness is edge-triggered. Returning an input, resize or wake
            // event must not discard the other tokens delivered by the same poll.
            if self.pending_tokens.is_empty() {
                if let Err(error) = self.poll.poll(&mut self.events, timeout.leftover()) {
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if self.events.is_empty() {
                    return Ok(None);
                }
                self.pending_tokens
                    .extend(self.events.iter().map(|event| event.token()));
            }

            while let Some(token) = self.pending_tokens.front().copied() {
                match token {
                    TTY_TOKEN => {
                        loop {
                            // The terminal descriptor can be blocking. Check available
                            // bytes before draining a retained readiness notification;
                            // changing O_NONBLOCK would also affect inherited stdin.
                            // SAFETY: tty_fd owns or borrows this valid descriptor for
                            // at least as long as this temporary borrow.
                            let fd =
                                unsafe { rustix::fd::BorrowedFd::borrow_raw(self.tty_fd.raw_fd()) };
                            if rustix::io::ioctl_fionread(fd)? == 0 {
                                self.pending_tokens.pop_front();
                                break;
                            }
                            match self.tty_fd.read(&mut self.tty_buffer) {
                                Ok(0) => {
                                    self.pending_tokens.pop_front();
                                    break;
                                }
                                Ok(read_count) => {
                                    self.parser.advance(
                                        &self.tty_buffer[..read_count],
                                        read_count == TTY_BUFFER_SIZE,
                                    );
                                }
                                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                    // Retain TTY readiness across returned parser events,
                                    // until the descriptor has actually been drained.
                                    self.pending_tokens.pop_front();
                                    break;
                                }
                                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                    continue;
                                }
                                Err(error) => return Err(error),
                            };

                            if let Some(event) = self.parser.next() {
                                return Ok(Some(event));
                            }
                        }
                    }
                    SIGNAL_TOKEN => {
                        self.pending_tokens.pop_front();
                        if self.signals.pending().next() == Some(signal_hook::consts::SIGWINCH) {
                            // TODO Should we remove tput?
                            //
                            // This can take a really long time, because terminal::size can
                            // launch new process (tput) and then it parses its output. It's
                            // not a really long time from the absolute time point of view, but
                            // it's a really long time from an async executor's point of view.
                            let new_size = crate::terminal::size()?;
                            return Ok(Some(InternalEvent::Event(Event::Resize(
                                new_size.0, new_size.1,
                            ))));
                        }
                    }
                    #[cfg(feature = "event-stream")]
                    WAKE_TOKEN => {
                        self.pending_tokens.pop_front();
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Interrupted,
                            "Poll operation was woken up by `Waker::wake`",
                        ));
                    }
                    _ => unreachable!("Synchronize Evented handle registration & token handling"),
                }
            }

            // Processing above can take some time, check if timeout expired
            if timeout.elapsed() {
                return Ok(None);
            }
        }
    }

    #[cfg(feature = "event-stream")]
    fn waker(&self) -> Waker {
        self.waker.clone()
    }
}

#[cfg(all(test, feature = "event-stream"))]
mod tests {
    use super::*;
    use crate::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    fn input_pair() -> (UnixInternalEventSource, UnixStream) {
        let (reader, writer) = UnixStream::pair().unwrap();
        #[cfg(feature = "libc")]
        let fd = {
            use std::os::fd::IntoRawFd;
            FileDesc::new(reader.into_raw_fd(), true)
        };
        #[cfg(not(feature = "libc"))]
        let fd = FileDesc::Owned(reader.into());
        (
            UnixInternalEventSource::from_file_descriptor(fd).unwrap(),
            writer,
        )
    }

    #[test]
    fn readiness_batch_keeps_input_and_wake() {
        let (mut source, mut writer) = input_pair();
        source.waker().wake().unwrap();
        writer.write_all(b"\x15").unwrap();
        let mut saw_wake = false;
        let mut saw_key = false;
        for _ in 0..2 {
            match source.try_read(Some(Duration::from_millis(100))) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => saw_wake = true,
                Ok(Some(InternalEvent::Event(Event::Key(key)))) => {
                    assert_eq!(
                        key,
                        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
                    );
                    saw_key = true;
                }
                other => panic!("a readiness notification was lost: {other:?}"),
            }
        }
        assert!(saw_wake && saw_key);
    }

    #[test]
    fn drained_blocking_input_respects_timeout() {
        let (mut source, mut writer) = input_pair();
        writer.write_all(b"x").unwrap();
        assert!(matches!(
            source.try_read(Some(Duration::from_millis(100))).unwrap(),
            Some(InternalEvent::Event(Event::Key(_)))
        ));
        // Release a broken blocking reader so this regression fails instead of
        // hanging the test process indefinitely.
        let next_write = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            writer.write_all(b"y").unwrap();
        });
        let result = source.try_read(Some(Duration::from_millis(5)));
        next_write.join().unwrap();
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn readiness_is_drained_past_one_input_buffer() {
        let (mut source, mut writer) = input_pair();
        let bytes = vec![b'x'; TTY_BUFFER_SIZE * 2 + 17];
        writer.write_all(&bytes).unwrap();
        for index in 0..bytes.len() {
            assert_eq!(
                source.try_read(Some(Duration::from_millis(100))).unwrap(),
                Some(InternalEvent::Event(Event::Key(KeyEvent::new(
                    KeyCode::Char('x'),
                    KeyModifiers::NONE
                )))),
                "input at index {index} must not need another write to wake the reader"
            );
        }
    }
}
