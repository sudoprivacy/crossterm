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
    #[cfg(windows)]
    let _ = crate::ansi_support::supports_ansi();
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
    #[cfg(windows)]
    return Ok(native_console_colors());
    #[cfg(not(windows))]
    Ok(None)
}

#[cfg(windows)]
fn native_console_colors() -> Option<DefaultColors> {
    use crossterm_winapi::Handle;
    use winapi::um::wincon::{
        CONSOLE_SCREEN_BUFFER_INFOEX, GetConsoleScreenBufferInfoEx, GetConsoleWindow,
    };
    use winapi::um::winuser::GetClassNameW;

    // ConPTY's color table belongs to its backing console, not to the visible
    // terminal theme. Only a native console window can use this fallback.
    let mut class = [0_u16; 32];
    // SAFETY: GetClassNameW writes at most the supplied buffer length; the
    // window handle is read-only and need not be owned by this function.
    let length = unsafe { GetClassNameW(GetConsoleWindow(), class.as_mut_ptr(), 32) };
    let length = usize::try_from(length).ok()?;
    if String::from_utf16_lossy(&class[..length]) != "ConsoleWindowClass" {
        return None;
    }
    let handle = Handle::output_handle().ok()?;
    // SAFETY: The POD structure admits zero initialization; cbSize is set
    // before the API writes to the valid, exclusively borrowed output buffer.
    let mut info: CONSOLE_SCREEN_BUFFER_INFOEX = unsafe { std::mem::zeroed() };
    info.cbSize = u32::try_from(std::mem::size_of_val(&info)).ok()?;
    if unsafe { GetConsoleScreenBufferInfoEx(*handle, &mut info) } == 0 {
        return None;
    }
    let rgb = |index: u16| {
        let bytes = info.ColorTable[usize::from(index)].to_le_bytes();
        (bytes[0], bytes[1], bytes[2])
    };
    Some(DefaultColors {
        foreground: rgb(info.wAttributes & 15),
        background: rgb((info.wAttributes >> 4) & 15),
    })
}
