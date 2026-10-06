//! Exercise cursor queries immediately after handing input back from EventStream.
use std::{
    io::{self, Write},
    thread,
    time::Duration,
};

use crossterm::{cursor, event::EventStream, execute, terminal};
use futures::{FutureExt, StreamExt};

fn main() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let result = (|| {
        for _ in 0..100 {
            let mut stream = EventStream::new();
            let _ = stream.next().now_or_never();
            drop(stream);
            execute!(io::stdout(), cursor::MoveTo(17, 9))?;
            assert_eq!(cursor::position()?, (17, 9));
        }
        // A reader shutdown can wake a cursor poll that has already started.
        // This stream is never polled, so it does not compete for input; only
        // its real shutdown wakeup overlaps the terminal's delayed DSR reply.
        for _ in 0..20 {
            let stream = EventStream::new();
            let shutdown = thread::spawn(move || {
                thread::sleep(Duration::from_millis(10));
                drop(stream);
            });
            execute!(io::stdout(), cursor::MoveTo(17, 9))?;
            let position = cursor::position();
            shutdown.join().expect("reader shutdown");
            assert_eq!(position?, (17, 9));
        }
        writeln!(io::stdout(), "CURSOR_HANDOFF_PASS")
    })();
    terminal::disable_raw_mode()?;
    result
}
