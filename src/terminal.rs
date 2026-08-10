//! Terminal setup, restoration, input, and presentation (architecture
//! sections 19-24).
//!
//! The RAII `TerminalGuard` enables the per-key input mode needed by the
//! application and restores raw mode and cursor visibility on drop. The
//! input layer polls with a bounded timeout and maps physical keys to
//! application commands (architecture section 20); the `Renderer` boundary
//! writes only what the controller reports — it owns no business behavior
//! (architecture section 21).

use std::io::{self, Write};
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use thiserror::Error;

use crate::app::UserCommand;

/// Bounded terminal-input poll timeout so worker events are drained promptly
/// and inference never blocks the loop (functional spec 10, architecture
/// section 8, decision D6).
pub const POLL_TIMEOUT: Duration = Duration::from_millis(100);

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
/// interactive state is entered.
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

/// Presentation boundary: the controller produces display data (a new status
/// block and/or output lines) and the renderer writes it. Rendering is
/// append-only — status blocks and lines accumulate in the persistent
/// terminal history (architecture section 23, decision D6).
pub trait Renderer {
    /// Writes the new status block (when `status_block` is `Some`) and
    /// appends `lines` to the persistent output area.
    fn render(&mut self, status_block: Option<&str>, lines: &[String]) -> io::Result<()>;
}

/// Writes render requests to stdout (append-only line rendering, D6).
pub struct TerminalRenderer;

impl Renderer for TerminalRenderer {
    fn render(&mut self, status_block: Option<&str>, lines: &[String]) -> io::Result<()> {
        let mut stdout = io::stdout();
        if let Some(block) = status_block {
            stdout.write_all(block.as_bytes())?;
        }
        for line in lines {
            stdout.write_all(line.as_bytes())?;
            stdout.write_all(b"\n")?;
        }
        stdout.flush()?;
        Ok(())
    }
}

/// True when at least one terminal event is available within `timeout`.
pub fn poll_key(timeout: Duration) -> Result<bool, TerminalError> {
    event::poll(timeout).map_err(TerminalError::Read)
}

/// Reads a single terminal event. Returns the key event, or `None` for
/// non-key events (resize, focus, paste, mouse), which are ignored
/// (decision D6). Never blocks for more than one queued event.
pub fn read_event() -> Result<Option<KeyEvent>, TerminalError> {
    match event::read().map_err(TerminalError::Read)? {
        Event::Key(key) => Ok(Some(key)),
        _ => Ok(None),
    }
}

/// Maps a physical key event to a focused-terminal command (functional spec
/// section 8, architecture section 20, decision D6):
/// - only `Press` events produce commands (`Repeat`/`Release` are ignored,
///   which prevents hold-to-toggle);
/// - `Ctrl+R` requires exactly the CONTROL modifier on `Char('r')`;
/// - `Esc` maps regardless of modifiers;
/// - every other key is ignored.
pub fn map_key(key: KeyEvent) -> Option<UserCommand> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    match key.code {
        KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
            Some(UserCommand::ToggleRecording)
        }
        KeyCode::Esc => Some(UserCommand::Cancel),
        _ => None,
    }
}

/// True for `Ctrl+C`, the exit key: with raw mode enabled it arrives as a key
/// event, not SIGINT, and initiates the normal shutdown path (architecture
/// section 29, decision D6).
pub fn is_exit_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl_r(kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(KeyCode::Char('r'), KeyModifiers::CONTROL, kind)
    }

    fn esc(kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(KeyCode::Esc, KeyModifiers::NONE, kind)
    }

    #[test]
    fn ctrl_r_maps_to_toggle_recording() {
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(UserCommand::ToggleRecording)
        );
    }

    #[test]
    fn ctrl_r_with_extra_modifiers_is_ignored() {
        let shifted = KeyEvent::new(
            KeyCode::Char('R'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(map_key(shifted), None);
    }

    #[test]
    fn esc_maps_to_cancel() {
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(UserCommand::Cancel)
        );
        // Esc with a modifier still cancels (decision D6).
        let alt_esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT);
        assert_eq!(map_key(alt_esc), Some(UserCommand::Cancel));
    }

    #[test]
    fn repeat_and_release_keys_are_ignored() {
        assert_eq!(map_key(ctrl_r(KeyEventKind::Repeat)), None);
        assert_eq!(map_key(ctrl_r(KeyEventKind::Release)), None);
        assert_eq!(map_key(esc(KeyEventKind::Repeat)), None);
        assert_eq!(map_key(esc(KeyEventKind::Release)), None);
    }

    #[test]
    fn unsupported_keys_are_ignored() {
        for key in [
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
            // Ctrl+C is the exit key, not a command.
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            assert_eq!(
                map_key(key),
                None,
                "unsupported key must be ignored: {key:?}"
            );
        }
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
