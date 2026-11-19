use std::{
    io::{self, Error, Write},
    time::{Duration, Instant},
};

use crate::{
    event::{
        filter::CursorPositionFilter,
        internal::{self, InternalEvent},
    },
    terminal::{disable_raw_mode, enable_raw_mode, sys::is_raw_mode_enabled},
};

/// Returns the cursor position (column, row).
///
/// The top left cell is represented as `(0, 0)`.
///
/// On unix systems, this function will block and possibly time out while
/// [`crossterm::event::read`](crate::event::read) or [`crossterm::event::poll`](crate::event::poll) are being called.
pub fn position() -> io::Result<(u16, u16)> {
    position_within(Duration::from_millis(2000))?.ok_or_else(|| {
        Error::other("The cursor position could not be read within a normal duration")
    })
}

/// Returns the cursor position (column, row), or `None` when the terminal does
/// not answer within `timeout`.
///
/// For probes that must not hold up a program's start: a terminal without
/// cursor position reports costs `timeout`, not two seconds. Like
/// [`position`], input that arrives meanwhile stays queued for the next
/// [`crate::event::read`].
pub fn position_within(timeout: Duration) -> io::Result<Option<(u16, u16)>> {
    if is_raw_mode_enabled() {
        read_position_raw(timeout)
    } else {
        enable_raw_mode()?;
        let pos = read_position_raw(timeout);
        disable_raw_mode()?;
        pos
    }
}

fn read_position_raw(timeout: Duration) -> io::Result<Option<(u16, u16)>> {
    // Discard any buffered cursor-position replies from earlier `ESC[6n` requests so the
    // position returned below corresponds to the fresh request we are about to send.
    // Poll with a zero timeout to drain only already-available events without blocking.
    while let Ok(true) = internal::poll(Some(Duration::ZERO), &CursorPositionFilter) {
        let _ = internal::read(&CursorPositionFilter);
    }

    // Use `ESC [ 6 n` to and retrieve the cursor position.
    let mut stdout = io::stdout();
    stdout.write_all(b"\x1B[6n")?;
    stdout.flush()?;

    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        match internal::poll(Some(remaining), &CursorPositionFilter) {
            Ok(true) => {
                if let Ok(InternalEvent::CursorPosition(x, y)) =
                    internal::read(&CursorPositionFilter)
                {
                    return Ok(Some((x, y)));
                }
            }
            // Dropping EventStream wakes the shared reader. That wakeup may
            // arrive in this poll; it does not mean the DSR deadline expired.
            Ok(false) => {}
            Err(error) => return Err(error),
        }
    }
}
