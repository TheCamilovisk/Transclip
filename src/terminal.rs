//! Terminal setup, restoration, input, and presentation (architecture
//! sections 19-24).
//!
//! The RAII `TerminalGuard` enables the per-key input mode needed by the
//! application and restores raw mode and cursor visibility on drop. The
//! input layer polls with a bounded timeout and maps physical keys to
//! application commands (architecture section 20); the `Renderer` boundary
//! redraws the complete fixed view the controller owns — it has no business
//! behavior (architecture sections 21, 23, decision D7).

use std::io::{self, Write};
use std::time::Duration;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::queue;
use crossterm::terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode};
use thiserror::Error;

use crate::app::{AppView, UserCommand};

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

/// Presentation boundary (architecture sections 19, 21, 23; decision D7):
/// the controller owns the display data ([`AppView`]) and the renderer
/// redraws the complete fixed view in place — no append-only history.
pub trait Renderer {
    /// Redraws the complete fixed view in place: moves to the interface
    /// origin, clears stale content from the prior view, writes the current
    /// view, and flushes (decision D7).
    fn render(&mut self, view: &AppView) -> io::Result<()>;
}

/// Writes render requests to stdout (in-place fixed-view redraw, D7).
pub struct TerminalRenderer;

impl Renderer for TerminalRenderer {
    fn render(&mut self, view: &AppView) -> io::Result<()> {
        render_to(&mut io::stdout(), view)
    }
}

/// Writes `text` to `writer` with every logical line feed serialized as `\r\n`
/// (architecture section 21). Raw mode disables the terminal's output
/// post-processing — crossterm's `cfmakeraw` clears `OPOST` (decision D6) —
/// so a bare `\n` would move the cursor down without returning it to column
/// zero; the renderer must emit the carriage return itself. A `\n` already
/// preceded by `\r` is left untouched so an existing `\r\n` is never doubled.
fn write_crlf(writer: &mut impl Write, text: &str) -> io::Result<()> {
    let bytes = text.as_bytes();
    let mut chunk_start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' && (index == 0 || bytes[index - 1] != b'\r') {
            writer.write_all(&bytes[chunk_start..index])?;
            writer.write_all(b"\r\n")?;
            chunk_start = index + 1;
        }
    }
    writer.write_all(&bytes[chunk_start..])
}

/// Serializes one complete view into `writer` as a single in-place redraw
/// (decision D7): move to the interface origin, clear stale content from the
/// prior view, write every logical line through `write_crlf` (architecture
/// section 21), and flush. The current view is written exactly once — a
/// normal transition never appends a second interface block (functional
/// spec section 11, acceptance criterion 20).
fn render_to(writer: &mut impl Write, view: &AppView) -> io::Result<()> {
    // `MoveTo` is 0-based, so (0, 0) is the top-left cell; clearing from the
    // cursor down after moving to the origin erases the prior view region.
    queue!(writer, MoveTo(0, 0), Clear(ClearType::FromCursorDown))?;
    for line in view.lines() {
        write_crlf(writer, &line)?;
        writer.write_all(b"\r\n")?;
    }
    writer.flush()?;
    Ok(())
}

/// True when at least one terminal event is available within `timeout`.
pub fn poll_key(timeout: Duration) -> Result<bool, TerminalError> {
    event::poll(timeout).map_err(TerminalError::Read)
}

/// Terminal input forwarded to the application loop (decision D7): key
/// events for command mapping and resize events that request an in-place
/// redraw of the current view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Key(KeyEvent),
    Resize,
}

/// Reads a single terminal event. Returns the key or resize event, or `None`
/// for non-key events (focus, paste, mouse), which are ignored (decision
/// D6). Never blocks for more than one queued event.
pub fn read_event() -> Result<Option<InputEvent>, TerminalError> {
    match event::read().map_err(TerminalError::Read)? {
        Event::Key(key) => Ok(Some(InputEvent::Key(key))),
        Event::Resize(_, _) => Ok(Some(InputEvent::Resize)),
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
    fn write_crlf_serializes_every_logical_line_feed() {
        let mut out = Vec::new();
        // A multiline status block with embedded blank lines and a trailing
        // terminator (architecture section 22/23).
        write_crlf(&mut out, "Ready to record\n\nCtrl+R  Start recording\n").unwrap();
        assert_eq!(out, b"Ready to record\r\n\r\nCtrl+R  Start recording\r\n");
        assert!(
            out.windows(2).all(|w| !(w[1] == b'\n' && w[0] != b'\r')),
            "no bare line feed bytes: every \\n must be preceded by \\r"
        );
    }

    #[test]
    fn write_crlf_preserves_existing_carriage_returns() {
        // An existing `\r\n` is not doubled; a bare `\n` after it is still
        // serialized.
        let mut out = Vec::new();
        write_crlf(&mut out, "a\r\nb\n").unwrap();
        assert_eq!(out, b"a\r\nb\r\n");
    }

    /// The ANSI redraw prefix produced by the single in-place redraw
    /// operation: move to the interface origin (`MoveTo(0, 0)` → `\x1b[1;1H`)
    /// then clear the previously occupied region from the cursor down
    /// (`ClearType::FromCursorDown` → `\x1b[J`). Pinned with crossterm 0.29.
    const REDRAW_PREFIX: &str = "\u{1b}[1;1H\u{1b}[J";

    #[test]
    fn render_to_clears_stale_content_then_writes_the_view_once() {
        let mut out = Vec::new();
        let view = AppView {
            status: "Ready to record".to_string(),
            latest_transcription: Some("hello world".to_string()),
            notice: Some("Copied to clipboard.".to_string()),
            commands: "Ctrl+R  Start recording".to_string(),
        };
        render_to(&mut out, &view).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.starts_with(REDRAW_PREFIX),
            "one in-place redraw must clear stale content first: {text:?}"
        );
        let body = &text[REDRAW_PREFIX.len()..];
        assert_eq!(
            body,
            "Ready to record\r\n\r\nhello world\r\n\r\nCopied to clipboard.\r\n\r\nCtrl+R  Start recording\r\n"
        );
        assert!(
            body.as_bytes()
                .windows(2)
                .all(|w| !(w[1] == b'\n' && w[0] != b'\r')),
            "every logical line feed must be serialized as CRLF"
        );
    }

    #[test]
    fn a_redraw_replaces_the_prior_view_without_duplicating_it() {
        // First render: a successful cycle's fixed view.
        let mut out = Vec::new();
        render_to(
            &mut out,
            &AppView {
                status: "Ready to record".to_string(),
                latest_transcription: Some("first result".to_string()),
                notice: Some("Copied to clipboard.".to_string()),
                commands: "Ctrl+R  Start recording".to_string(),
            },
        )
        .unwrap();
        // Second render (e.g. a resize or a state transition): the current
        // view with a multi-line commands block.
        render_to(
            &mut out,
            &AppView {
                status: "Recording...".to_string(),
                latest_transcription: Some("first result".to_string()),
                notice: Some("Copied to clipboard.".to_string()),
                commands: "Ctrl+R  Finish\nEsc     Cancel".to_string(),
            },
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        // The second redraw starts by clearing, then writes the complete
        // current view exactly once — nothing from the prior view is
        // repeated after the clear (functional spec 11, criterion 20).
        let second_start = text.rfind(REDRAW_PREFIX).unwrap();
        let second_body = &text[second_start + REDRAW_PREFIX.len()..];
        assert_eq!(
            second_body,
            "Recording...\r\n\r\nfirst result\r\n\r\nCopied to clipboard.\r\n\r\nCtrl+R  Finish\r\nEsc     Cancel\r\n"
        );
        assert!(
            !second_body.contains("Ready to record"),
            "the stale status must not be repeated after the redraw"
        );
        assert!(
            second_body
                .as_bytes()
                .windows(2)
                .all(|w| !(w[1] == b'\n' && w[0] != b'\r')),
            "no bare line feed bytes anywhere in the render stream"
        );
    }

    #[test]
    fn render_to_handles_embedded_newlines_with_crlf() {
        // The fixed view may carry embedded logical newlines (multi-line
        // commands, multi-line notices); every one is serialized as CRLF
        // (architecture section 21, plan step 5).
        let mut out = Vec::new();
        render_to(
            &mut out,
            &AppView {
                status: "Transcribing...".to_string(),
                latest_transcription: None,
                notice: Some("first\nsecond line".to_string()),
                commands: "Esc     Cancel".to_string(),
            },
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let body = &text[REDRAW_PREFIX.len()..];
        assert_eq!(
            body,
            "Transcribing...\r\n\r\nfirst\r\nsecond line\r\n\r\nEsc     Cancel\r\n"
        );
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
