//! Query the cursor with a deadline: an answer is returned, silence costs only
//! the deadline, and input typed while waiting stays queued.
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute, terminal,
};

fn main() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let result = (|| {
        execute!(io::stdout(), cursor::MoveTo(17, 9))?;
        assert_eq!(
            cursor::position_within(Duration::from_millis(2000))?,
            Some((17, 9))
        );
        // The fixture leaves this query unanswered and types `k` instead.
        let started = Instant::now();
        assert_eq!(cursor::position_within(Duration::from_millis(250))?, None);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "an unanswered query waits for its deadline only: {:?}",
            started.elapsed()
        );
        match event::read()? {
            Event::Key(key) if key.code == KeyCode::Char('k') => {}
            other => panic!("the key typed during the query was lost: {other:?}"),
        }
        writeln!(io::stdout(), "CURSOR_WITHIN_PASS")
    })();
    terminal::disable_raw_mode()?;
    result
}
