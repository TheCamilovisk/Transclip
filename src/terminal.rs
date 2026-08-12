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
use crossterm::style::{Color, ResetColor, SetForegroundColor};
use crossterm::terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode};
use thiserror::Error;

use crate::app::{AppView, StatusStyle, UserCommand};

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

/// The state emoji prefix rendered before the status line (decision D8,
/// functional spec criterion 21): `🟢` Ready, `🔴` Recording, and `⚙️`
/// Transcribing/Cancelling. Presentation-only — `AppView` carries the plain
/// status wording (ADR-14).
fn status_emoji(style: StatusStyle) -> &'static str {
    match style {
        StatusStyle::Ready => "🟢 ",
        StatusStyle::Recording => "🔴 ",
        StatusStyle::Neutral => "⚙️ ",
    }
}

/// The scoped status foreground color, or `None` for the terminal default
/// color (decision D8): Ready is light green (`Color::Green`), Recording is
/// light red (`Color::Red`), and Transcribing/Cancelling applies no color.
/// crossterm's `Color::{Green, Red}` are the light variants (8-bit codes
/// 10/9), matching the light-green/light-red requirement.
fn status_color(style: StatusStyle) -> Option<Color> {
    match style {
        StatusStyle::Ready => Some(Color::Green),
        StatusStyle::Recording => Some(Color::Red),
        StatusStyle::Neutral => None,
    }
}

/// Writes the styled status line (decision D8, ADR-14): the state emoji
/// prefix plus the plain status text, in the state color when one applies,
/// then a terminal color reset immediately after the complete status line so
/// no status color bleeds into any following logical line — the latest
/// transcription, notice, or command hints (functional spec section 11,
/// criterion 21). The neutral `⚙️` style sets no color and therefore needs no
/// reset: it renders in the terminal default.
fn write_status_line(writer: &mut impl Write, status: &str, style: StatusStyle) -> io::Result<()> {
    let color = status_color(style);
    if let Some(color) = color {
        queue!(writer, SetForegroundColor(color))?;
    }
    write_crlf(writer, &format!("{}{}", status_emoji(style), status))?;
    writer.write_all(b"\r\n")?;
    if color.is_some() {
        // Reset before the next logical line (plan step 2); a later resize
        // redraw must never inherit a stale status color (decision D8).
        queue!(writer, ResetColor)?;
    }
    Ok(())
}

/// Serializes one complete view into `writer` as a single in-place redraw
/// (decision D7): move to the interface origin, clear stale content from the
/// prior view, write every logical line through `write_crlf` (architecture
/// section 21), and flush. The current view is written exactly once — a
/// normal transition never appends a second interface block (functional
/// spec section 11, acceptance criterion 20). Only the status line is styled
/// (decision D8); the latest transcription, notices, and command hints are
/// written unstyled in the terminal default color.
fn render_to(writer: &mut impl Write, view: &AppView) -> io::Result<()> {
    // `MoveTo` is 0-based, so (0, 0) is the top-left cell; clearing from the
    // cursor down after moving to the origin erases the prior view region.
    queue!(writer, MoveTo(0, 0), Clear(ClearType::FromCursorDown))?;
    write_status_line(writer, &view.status, view.status_style)?;
    // Every remaining logical line (latest transcription, notice, commands)
    // is neutral — plain text in the terminal default color.
    for line in view.lines().into_iter().skip(1) {
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

    /// The status color escapes produced by crossterm 0.29 (decision D8):
    /// `SetForegroundColor(Color::Green)` → `\x1b[38;5;10m` (light green),
    /// `SetForegroundColor(Color::Red)` → `\x1b[38;5;9m` (light red), and
    /// `ResetColor` → `\x1b[0m`. Verified against the crossterm 0.29.0
    /// source (`style/types/colored.rs`, `style.rs`).
    const LIGHT_GREEN: &str = "\u{1b}[38;5;10m";
    const LIGHT_RED: &str = "\u{1b}[38;5;9m";
    const COLOR_RESET: &str = "\u{1b}[0m";

    /// Crossterm memoizes color support from the `NO_COLOR` environment
    /// variable on first use; force colors on so the byte-level style
    /// assertions below are deterministic in any environment (decision D8).
    fn force_colors() {
        crossterm::style::Colored::set_ansi_color_disabled(false);
    }

    #[test]
    fn render_to_clears_stale_content_then_writes_the_view_once() {
        force_colors();
        let mut out = Vec::new();
        let view = AppView {
            status: "Ready to record".to_string(),
            status_style: StatusStyle::Ready,
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
            format!(
                "{LIGHT_GREEN}🟢 Ready to record\r\n{COLOR_RESET}\r\nhello world\r\n\r\nCopied to clipboard.\r\n\r\nCtrl+R  Start recording\r\n"
            )
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
        force_colors();
        // First render: a successful cycle's fixed view.
        let mut out = Vec::new();
        render_to(
            &mut out,
            &AppView {
                status: "Ready to record".to_string(),
                status_style: StatusStyle::Ready,
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
                status_style: StatusStyle::Recording,
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
            format!(
                "{LIGHT_RED}🔴 Recording...\r\n{COLOR_RESET}\r\nfirst result\r\n\r\nCopied to clipboard.\r\n\r\nCtrl+R  Finish\r\nEsc     Cancel\r\n"
            )
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
        // (architecture section 21, plan step 5). The neutral `⚙️` status
        // carries no color escapes (decision D8).
        let mut out = Vec::new();
        render_to(
            &mut out,
            &AppView {
                status: "Transcribing...".to_string(),
                status_style: StatusStyle::Neutral,
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
            "⚙️ Transcribing...\r\n\r\nfirst\r\nsecond line\r\n\r\nEsc     Cancel\r\n"
        );
        assert!(
            !body.contains('\u{1b}'),
            "the neutral status applies no non-default color escape"
        );
    }

    // ---- Scoped state styling (decision D8, plan steps 1-2) ----

    #[test]
    fn ready_status_is_styled_light_green_with_emoji_and_resets() {
        force_colors();
        let mut out = Vec::new();
        render_to(
            &mut out,
            &AppView {
                status: "Ready to record".to_string(),
                status_style: StatusStyle::Ready,
                latest_transcription: None,
                notice: None,
                commands: "Ctrl+R  Start recording".to_string(),
            },
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let body = &text[REDRAW_PREFIX.len()..];
        assert!(
            body.contains("🟢"),
            "the Ready status must show the 🟢 emoji"
        );
        assert!(
            body.starts_with(&format!(
                "{LIGHT_GREEN}🟢 Ready to record\r\n{COLOR_RESET}\r\n"
            )),
            "Ready is light green and resets before the next line: {body:?}"
        );
    }

    #[test]
    fn recording_status_is_styled_light_red_and_resets_before_content() {
        force_colors();
        let mut out = Vec::new();
        render_to(
            &mut out,
            &AppView {
                status: "Recording...".to_string(),
                status_style: StatusStyle::Recording,
                latest_transcription: Some("mid text".to_string()),
                notice: Some("Copied to clipboard.".to_string()),
                commands: "Ctrl+R  Finish\nEsc     Cancel".to_string(),
            },
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let body = &text[REDRAW_PREFIX.len()..];
        assert!(
            body.contains("🔴"),
            "the Recording status must show the 🔴 emoji"
        );
        assert_eq!(
            body,
            format!(
                "{LIGHT_RED}🔴 Recording...\r\n{COLOR_RESET}\r\nmid text\r\n\r\nCopied to clipboard.\r\n\r\nCtrl+R  Finish\r\nEsc     Cancel\r\n"
            ),
            "the light-red status resets before the transcription, notice, and command content"
        );
    }

    #[test]
    fn transcribing_and_cancelling_statuses_are_neutral_gear() {
        // Both Transcribing phases render `⚙️` with no non-default state
        // color: no `SetForegroundColor`/`ResetColor` escape at all after
        // the redraw prefix (decision D8).
        for status in ["Transcribing...", "Cancelling transcription..."] {
            let mut out = Vec::new();
            render_to(
                &mut out,
                &AppView {
                    status: status.to_string(),
                    status_style: StatusStyle::Neutral,
                    latest_transcription: None,
                    notice: None,
                    commands: "Esc     Cancel".to_string(),
                },
            )
            .unwrap();
            let text = String::from_utf8(out).unwrap();
            let body = &text[REDRAW_PREFIX.len()..];
            assert!(
                body.contains("⚙️"),
                "the {status} status must show the ⚙️ emoji"
            );
            assert_eq!(
                body,
                &format!("⚙️ {status}\r\n\r\nEsc     Cancel\r\n"),
                "{status} is rendered with ⚙️ in the terminal default color"
            );
            assert!(
                !body.contains('\u{1b}'),
                "no non-default state color may be applied to {status}"
            );
        }
    }

    #[test]
    fn only_the_status_line_occurs_between_color_set_and_reset() {
        force_colors();
        let mut out = Vec::new();
        render_to(
            &mut out,
            &AppView {
                status: "Ready to record".to_string(),
                status_style: StatusStyle::Ready,
                latest_transcription: Some("hello world".to_string()),
                notice: Some("Copied to clipboard.".to_string()),
                commands: "Ctrl+R  Start recording".to_string(),
            },
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let body = &text[REDRAW_PREFIX.len()..];
        let start = body.find(LIGHT_GREEN).expect("status color is set");
        let end = body.find(COLOR_RESET).expect("status color is reset");
        let styled = &body[start + LIGHT_GREEN.len()..end];
        assert_eq!(
            styled, "🟢 Ready to record\r\n",
            "only the status line is styled"
        );
        // Neither the transcription, the notice, nor the command hints may
        // occur between a state color set and its reset (plan step 4).
        for neutral in ["hello world", "Copied to clipboard.", "Ctrl+R"] {
            assert!(
                !styled.contains(neutral),
                "{neutral} must not appear inside the styled span"
            );
        }
        assert!(
            body[end + COLOR_RESET.len()..].contains("hello world"),
            "the transcription follows the reset, in the terminal default color"
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
