//! Recorder boundary (architecture sections 10-12, 32-33).
//!
//! Slice 2 establishes the trait the application controller drives and the
//! audio type the recorder returns; slice 3 attaches the CPAL-backed
//! implementation. Infrastructure returns values — it never mutates
//! application state or renders directly.

use std::fmt;

/// Captured in-memory audio handed from the recorder to the transcriber
/// (architecture section 11). Lives here because it is the recorder
/// boundary's output; the shared-contract shape is preserved.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// A recorder failure, reported to the controller as an opaque message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderError(pub String);

impl fmt::Display for RecorderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Boundary the controller drives to capture microphone audio
/// (architecture sections 10, 33).
pub trait Recorder {
    /// Opens the microphone and starts accumulating samples. Errors leave
    /// the recorder unusable-but-idle; the controller stays in `Ready`
    /// (functional spec 15.1).
    fn start(&mut self) -> Result<(), RecorderError>;

    /// Stops capture and returns the accumulated audio for transcription
    /// (functional spec 4.2).
    fn stop(&mut self) -> Result<RecordedAudio, RecorderError>;

    /// Stops capture and discards the accumulated audio (functional spec
    /// 4.2); no audio is ever submitted after a cancel.
    fn cancel(&mut self);
}

/// Slice-2 placeholder: reports a clear error until the CPAL-backed
/// implementation lands in slice 3. This lets the Ready shell and the
/// controller's recorder-error path run end to end without hardware.
pub struct UnavailableRecorder;

impl Recorder for UnavailableRecorder {
    fn start(&mut self) -> Result<(), RecorderError> {
        Err(RecorderError(
            "microphone capture is not available in this build".to_string(),
        ))
    }

    fn stop(&mut self) -> Result<RecordedAudio, RecorderError> {
        Err(RecorderError("no active recording".to_string()))
    }

    fn cancel(&mut self) {}
}
