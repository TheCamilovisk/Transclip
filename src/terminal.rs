//! Terminal setup, restoration, input, and status rendering.
//!
//! The RAII `TerminalGuard` enables the per-key input mode needed by the
//! application and restores raw mode and cursor visibility on drop
//! (architecture sections 20, 24).

use std::io;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("failed to initialize terminal (is stdin a terminal?): {0}")]
    Init(#[source] io::Error),
    #[error("failed to read keyboard input: {0}")]
    Read(#[source] io::Error),
    #[error("failed to write terminal output: {0}")]
    Write(#[source] io::Error),
    #[error("failed to restore terminal state: {0}")]
    Restore(#[source] io::Error),
}

/// RAII guard that restores the terminal state on drop (architecture section 24).
///
/// Construction fails cleanly when stdin is not a terminal, before any partial
/// interactive state is entered (slice 1 step 8).
pub struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    /// Enables raw mode (per-key input without waiting for newline) and hides
    /// the cursor.
    pub fn enter() -> Result<Self, TerminalError> {
        enable_raw_mode().map_err(TerminalError::Init)?;
        let mut stdout = io::stdout();
        if let Err(e) = execute!(stdout, Hide) {
            let _ = disable_raw_mode();
            return Err(TerminalError::Init(e));
        }
        Ok(Self { active: true })
    }

    /// Restores raw mode and cursor visibility. Idempotent: subsequent calls
    /// are no-ops, and the destructor never panics.
    pub fn restore(&mut self) -> Result<(), TerminalError> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        disable_raw_mode().map_err(TerminalError::Restore)?;
        execute!(io::stdout(), Show).map_err(TerminalError::Restore)?;
        Ok(())
    }

    #[cfg(test)]
    fn for_test(active: bool) -> Self {
        Self { active }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// The Ready status block (functional spec 4.1, architecture section 22).
pub fn render_ready() -> String {
    "Ready to record\n\nCtrl+R  Start recording\n".to_string()
}

/// Reads the next key press, ignoring non-key events (resize, focus, paste).
pub fn read_key() -> Result<KeyEvent, TerminalError> {
    loop {
        match event::read().map_err(TerminalError::Read)? {
            Event::Key(key) => return Ok(key),
            _ => continue,
        }
    }
}

/// True for `Ctrl+C`, the slice-1 exit key. Slice 2 replaces this with the
/// real command mapping (`Ctrl+R` → toggle recording, `Esc` → cancel).
pub fn is_exit_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_ready_matches_spec() {
        assert_eq!(
            render_ready(),
            "Ready to record\n\nCtrl+R  Start recording\n"
        );
    }

    #[test]
    fn ctrl_c_is_an_exit_key() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(is_exit_key(key));
        let plain = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(!is_exit_key(plain));
        let ctrl_r = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert!(!is_exit_key(ctrl_r));
    }

    #[test]
    fn restore_is_idempotent_and_noop_when_inactive() {
        // An inactive guard must not touch the terminal at all: restore
        // succeeds without any crossterm call (no TTY required).
        let mut guard = TerminalGuard::for_test(false);
        assert!(guard.restore().is_ok());
        assert!(guard.restore().is_ok());
    }

    #[test]
    fn enter_fails_cleanly_without_a_tty() {
        use std::io::IsTerminal;

        // Unit tests run without a real terminal; `enter` must fail cleanly
        // rather than panic or partially enter interactive mode.
        if io::stdin().is_terminal() {
            return; // would require a real TTY to assert success
        }
        assert!(matches!(
            TerminalGuard::enter(),
            Err(TerminalError::Init(_))
        ));
    }
}
