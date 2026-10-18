//! Exercise palette replies while an async input reader releases its wakeup.
use std::{
    io::{self, Write},
    thread,
    time::Duration,
};

use crossterm::{event::EventStream, terminal};

fn main() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let result = (|| {
        // The stream is never polled and therefore cannot consume the reply.
        // Its real shutdown wakeup is independent of terminal response timing.
        for _ in 0..12 {
            let stream = EventStream::new();
            let shutdown = thread::spawn(move || {
                thread::sleep(Duration::from_millis(10));
                drop(stream);
            });
            let colors = terminal::default_colors(Duration::from_millis(250));
            shutdown.join().expect("reader shutdown");
            let colors = colors?.expect("palette reply within the original deadline");
            assert_eq!(colors.foreground, (0, 0, 0));
            assert_eq!(colors.background, (255, 255, 255));
        }
        writeln!(io::stdout(), "PALETTE_HANDOFF_PASS")
    })();
    terminal::disable_raw_mode()?;
    result
}
