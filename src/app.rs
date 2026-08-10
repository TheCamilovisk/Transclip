//! Application controller.
//!
//! Slice 1 establishes only the Ready shell: the status block is rendered and
//! the loop waits for `Ctrl+C` to exit. All other keys are ignored. The
//! focused terminal command loop (`Ctrl+R` / `Esc`, mode transitions) arrives
//! with slice 2; rendering stays compatible with the persistent output area
//! required by later slices (architecture section 23).

use std::io::{self, Write};

use crate::terminal::{self, TerminalError};

/// Runs the interactive shell. Returns `Ok` after the user exits with
/// `Ctrl+C` (slice-1 escape hatch).
pub fn run() -> Result<(), TerminalError> {
    let mut stdout = io::stdout();
    write!(stdout, "{}", terminal::render_ready()).map_err(TerminalError::Write)?;
    stdout.flush().map_err(TerminalError::Write)?;

    loop {
        let key = terminal::read_key()?;
        if terminal::is_exit_key(key) {
            return Ok(());
        }
        // Slice 1: no commands are available yet.
    }
}
