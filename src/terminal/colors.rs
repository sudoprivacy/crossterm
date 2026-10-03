use std::io::{self, Write};
use std::time::{Duration, Instant};

use crate::event::filter::Filter;
use crate::event::internal::{InternalEvent, lock_event_reader};
use crate::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled};

/// The terminal's default foreground and background, as 8-bit RGB values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultColors {
    /// Default text color.
    pub foreground: (u8, u8, u8),
    /// Default terminal canvas color.
    pub background: (u8, u8, u8),
}

struct ColorFilter;

impl Filter for ColorFilter {
    fn eval(&self, event: &InternalEvent) -> bool {
        matches!(event, InternalEvent::TerminalColor(..))
    }
}

struct RawModeGuard(bool);

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.0 {
            let _ = disable_raw_mode();
        }
    }
}

/// Query OSC 10/11 once under a shared timeout, preserving unrelated input.
///
/// Call before starting an event stream or another polling thread. Unsupported
/// terminals return `None`; keys and pastes received during the query remain in
/// the same event queue for the subsequent reader. Late replies remain internal
/// events and cannot appear as typed text. Raw mode is restored on every exit.
pub fn default_colors(timeout: Duration) -> io::Result<Option<DefaultColors>> {
    let restore_raw = !is_raw_mode_enabled()?;
    if restore_raw {
        enable_raw_mode()?;
    }
    let _guard = RawModeGuard(restore_raw);
    let mut reader = lock_event_reader();
    while reader.try_read(&ColorFilter).is_some() {}
    let mut stdout = io::stdout().lock();
    stdout.write_all(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\")?;
    stdout.flush()?;
    drop(stdout);
    let deadline = Instant::now() + timeout;
    let mut foreground = None;
    let mut background = None;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        if !reader.poll(Some(remaining), &ColorFilter)? {
            break;
        }
        match reader.try_read(&ColorFilter) {
            Some(InternalEvent::TerminalColor(10, rgb)) => foreground = Some(rgb),
            Some(InternalEvent::TerminalColor(11, rgb)) => background = Some(rgb),
            _ => {}
        }
        if let (Some(foreground), Some(background)) = (foreground, background) {
            return Ok(Some(DefaultColors {
                foreground,
                background,
            }));
        }
    }
    Ok(None)
}
