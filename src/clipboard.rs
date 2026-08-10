//! Clipboard boundary and arboard-backed implementation (architecture
//! sections 3.5, 18; decision D3 arboard policy).
//!
//! Slice 5 adds the smallest practical clipboard boundary — one function,
//! `copy_text` — and invokes it exclusively on the accepted final
//! transcription path. The controller depends only on the [`Clipboard`] trait
//! so app tests inject a fake and never touch the developer's real clipboard
//! (architecture section 45). Backend errors surface as a non-fatal warning:
//! a missing/headless clipboard service must not invalidate a successful
//! transcription (functional spec 15.4) or prevent future recording cycles.
//!
//! [`ArboardClipboard`] constructs one `arboard::Clipboard` lazily on the
//! first copy and keeps it alive for the session, all on the main thread.
//! Keeping a single persistent instance matters on X11: arboard hosts the
//! selection inside the app and hands it to a clipboard manager when the last
//! instance drops, so constructing per copy would race managers and can lose
//! the content. Deferring the first construction to copy time means an
//! unavailable clipboard service surfaces exactly where the spec wants it —
//! as a copy warning — rather than failing startup.

use std::fmt;

/// A clipboard failure, reported to the controller as an opaque message
/// (same pattern as `RecorderError`). The message describes the backend
/// problem and never includes the transcription text being copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardError(pub String);

impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<arboard::Error> for ClipboardError {
    fn from(err: arboard::Error) -> Self {
        // arboard's `Display` is a short human-readable description without
        // payload; the copied text is never part of it.
        ClipboardError(format!("{err}"))
    }
}

/// Boundary the controller drives to place text on the system clipboard
/// (architecture section 18). The clipboard is invoked only on an accepted
/// successful transcription; it never decides application state.
pub trait Clipboard {
    /// Copies `text` to the clipboard. The caller renders the transcription
    /// and reports success/warning; a copy failure is not a transcription
    /// failure (functional spec 15.4, decision R4).
    fn copy_text(&mut self, text: &str) -> Result<(), ClipboardError>;
}

/// Real clipboard backed by arboard (X11 via x11rb, Wayland via
/// `wl-clipboard-rs` when `WAYLAND_DISPLAY` is set; decision D3).
///
/// One `arboard::Clipboard` is constructed lazily on the first copy and kept
/// alive for the session. Keeping a single persistent instance matters on
/// X11: arboard hosts the selection inside the app and, when the last
/// `Clipboard` instance drops, hands the content to a clipboard manager and
/// tears the serving window down — constructing per copy would race managers
/// and can lose the content. Deferring the first construction keeps a
/// headless session fully usable: the backend error surfaces exactly as a
/// per-copy warning instead of failing startup (functional spec 15.4).
pub struct ArboardClipboard {
    inner: Option<arboard::Clipboard>,
}

impl ArboardClipboard {
    pub fn new() -> Self {
        Self { inner: None }
    }
}

impl Clipboard for ArboardClipboard {
    fn copy_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        if self.inner.is_none() {
            self.inner = Some(arboard::Clipboard::new()?);
        }
        self.inner
            .as_mut()
            .expect("clipboard constructed just above")
            .set_text(text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_conversion_preserves_backend_reason_without_text() {
        // The copied text is never part of the error message: arboard's
        // `Display` describes the backend problem only (plan step 6).
        let err = arboard::Error::Unknown {
            description: "no clipboard service reachable".to_string(),
        };
        let converted = ClipboardError::from(err);
        assert!(converted.0.contains("no clipboard service reachable"));
        assert!(!converted.0.contains("secret transcription text"));
    }
}
