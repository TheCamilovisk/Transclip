//! Recorder boundary and CPAL-backed implementation (architecture sections
//! 10-12, 32-33; decision D4).
//!
//! Slice 2 established the `Recorder` trait the controller drives and the
//! audio type the recorder returns; slice 3 attaches the CPAL implementation.
//! The controller's `RecordingFailed` path (functional spec 15.2) is served by
//! an injected error sink: the CPAL error callback only forwards a message and
//! never touches the stream (dropping the stream from inside its own callback
//! thread would self-join); the controller calls `cancel` on the main loop,
//! which releases the stream there.
//!
//! The capture buffer and the normalization math live in testable, hardware-
//! free units ([`CaptureBuffer`] here, [`crate::audio`] for DSP); CPAL wiring
//! itself is covered by manual acceptance.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::audio;

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
    /// (functional spec 4.2). Only a successful stop may create a job.
    fn stop(&mut self) -> Result<RecordedAudio, RecorderError>;

    /// Stops capture and discards the accumulated audio (functional spec
    /// 4.2); no audio is ever submitted after a cancel.
    fn cancel(&mut self);
}

/// The sample formats this recorder can capture (decision D4): every format
/// addressable through `cpal::Data::as_slice` with a numeric sample type.
/// DSD (non-PCM) and 64-bit integer formats are rejected at `start`, leaving
/// the controller Ready with a clear error.
fn is_supported_format(format: cpal::SampleFormat) -> bool {
    matches!(
        format,
        cpal::SampleFormat::I8
            | cpal::SampleFormat::I16
            | cpal::SampleFormat::I24
            | cpal::SampleFormat::I32
            | cpal::SampleFormat::U8
            | cpal::SampleFormat::U16
            | cpal::SampleFormat::U24
            | cpal::SampleFormat::U32
            | cpal::SampleFormat::F32
            | cpal::SampleFormat::F64
    )
}

/// Converts one raw CPAL buffer to interleaved `f32` (decision D4). The
/// format is validated at `start`, so the wildcard arm (DSD, 64-bit
/// integers) is unreachable in practice; it yields no samples defensively
/// rather than panicking on the audio thread.
fn data_to_f32(data: &cpal::Data) -> Vec<f32> {
    use cpal::SampleFormat::{F32, F64, I8, I16, I24, I32, U8, U16, U24, U32};
    match data.sample_format() {
        F32 => data
            .as_slice::<f32>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        F64 => data
            .as_slice::<f64>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        I8 => data
            .as_slice::<i8>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        I16 => data
            .as_slice::<i16>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        I24 => data
            .as_slice::<cpal::I24>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        I32 => data
            .as_slice::<i32>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        U8 => data
            .as_slice::<u8>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        U16 => data
            .as_slice::<u16>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        U24 => data
            .as_slice::<cpal::U24>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        U32 => data
            .as_slice::<u32>()
            .map(audio::convert_to_f32)
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Synchronized accumulation of one recording's samples (decision D4). The
/// CPAL callback appends interleaved `f32` samples; `finish` normalizes them
/// (downmix to mono, resample to 16 kHz) outside the callback, and `clear`
/// discards them. All operations are idempotent-safe: a recording is either
/// finished exactly once or cleared.
///
/// `Default` is only used as the placeholder left behind when `stop` takes
/// the captured samples out of the shared mutex; it is never finished.
#[derive(Default)]
struct CaptureBuffer {
    /// Interleaved samples in the device's capture format, converted to f32.
    samples: Vec<f32>,
    /// Channel count of the capture stream.
    channels: u16,
    /// Capture sample rate, Hz.
    sample_rate: u32,
}

impl CaptureBuffer {
    fn append(&mut self, interleaved: &[f32]) {
        self.samples.extend_from_slice(interleaved);
    }

    /// Consumes the capture, normalizes it, and returns Whisper-ready audio.
    /// A completely empty capture (the stream never delivered data — e.g. a
    /// near-instant toggle) is an error, so no job is ever submitted for it
    /// (decision D4). Non-empty silent audio passes through: whisper.cpp
    /// returns empty text for silence rather than failing.
    fn finish(self) -> Result<RecordedAudio, RecorderError> {
        if self.samples.is_empty() {
            return Err(RecorderError("no audio was captured".to_string()));
        }
        let mono = audio::downmix_to_mono(&self.samples, self.channels);
        let samples = audio::resample(&mono, self.sample_rate, audio::WHISPER_SAMPLE_RATE);
        Ok(RecordedAudio {
            samples,
            sample_rate: audio::WHISPER_SAMPLE_RATE,
        })
    }
}

/// How long `build_input_stream_raw` waits for the backend to initialize
/// (decision D4): a hung device fails `start` instead of blocking the
/// terminal loop, keeping `Ctrl+R`/`Esc` responsive (functional spec 10).
const STREAM_INIT_TIMEOUT: Duration = Duration::from_secs(2);

/// CPAL-backed recorder using the default input device (decision D4).
///
/// The stream is owned only while a recording is active; `stop` and `cancel`
/// drop it first (CPAL's `Drop` joins the ALSA callback thread, so no
/// callback can write afterwards) and then take or clear the shared buffer.
/// Repeated stop/cancel/error cleanup is safe: after the first cleanup the
/// recorder is idle and further calls are no-ops or clear errors.
pub struct CpalRecorder {
    /// Forwards stream failures to the controller as `AppEvent::RecordingFailed`.
    on_error: Arc<dyn Fn(String) + Send + Sync>,
    stream: Option<cpal::Stream>,
    buffer: Option<Arc<Mutex<CaptureBuffer>>>,
}

impl CpalRecorder {
    /// `on_error` receives a human-readable stream-failure message. It must
    /// only forward the message (e.g. onto the app event channel) and never
    /// block on the recorder itself.
    pub fn new(on_error: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            on_error: Arc::new(on_error),
            stream: None,
            buffer: None,
        }
    }
}

impl Recorder for CpalRecorder {
    fn start(&mut self) -> Result<(), RecorderError> {
        if self.stream.is_some() {
            return Err(RecorderError("recording already active".to_string()));
        }
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| RecorderError("no microphone found".to_string()))?;
        let config = select_config(&device)?;
        let channels = config.channels();
        let sample_rate = config.sample_rate();
        let sample_format = config.sample_format();

        let buffer = Arc::new(Mutex::new(CaptureBuffer {
            samples: Vec::new(),
            channels,
            sample_rate,
        }));

        // Data callback: minimal work only — convert to f32 and append
        // (architecture section 9; plan step 3). No downmix, resampling, or
        // inference happens here.
        let capture = buffer.clone();
        let data_callback = move |data: &cpal::Data, _: &cpal::InputCallbackInfo| {
            let samples = data_to_f32(data);
            if !samples.is_empty() {
                if let Ok(mut capture) = capture.lock() {
                    capture.append(&samples);
                }
            }
        };

        // Error callback: forward and return — never touch the stream from
        // the audio thread (would self-join on drop; decision D4).
        let on_error = self.on_error.clone();
        let error_callback = move |err: cpal::Error| on_error(err.to_string());

        let stream = device
            .build_input_stream_raw(
                config.config(),
                sample_format,
                data_callback,
                error_callback,
                Some(STREAM_INIT_TIMEOUT),
            )
            .map_err(|e| RecorderError(format!("unable to open microphone stream: {e}")))?;
        // On failure the stream is dropped here, releasing the buffer; no
        // residual state remains (plan step 5).
        stream
            .play()
            .map_err(|e| RecorderError(format!("unable to start microphone stream: {e}")))?;

        self.stream = Some(stream);
        self.buffer = Some(buffer);
        Ok(())
    }

    fn stop(&mut self) -> Result<RecordedAudio, RecorderError> {
        // Drop the stream first: CPAL joins the callback thread, so no
        // callback can append after this point. Then take the buffer.
        self.stream = None;
        let buffer = match self.buffer.take() {
            Some(buffer) => buffer,
            None => return Err(RecorderError("no active recording".to_string())),
        };
        let captured = std::mem::take(&mut *buffer.lock().unwrap_or_else(|p| p.into_inner()));
        captured.finish()
    }

    fn cancel(&mut self) {
        // Dropping the stream joins the callback thread; dropping the Arc
        // frees the captured samples. Idempotent: repeated cancels are
        // no-ops, and nothing is ever submitted after a cancel.
        self.stream = None;
        self.buffer = None;
    }
}

/// Picks the capture configuration (decision D4): the device's default input
/// config when its sample format is supported; otherwise the best supported
/// config by CPAL's default heuristics (48 kHz, then 44.1 kHz, then max
/// rate). Any channel count ≥ 1 is accepted and downmixed at `finish`.
fn select_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig, RecorderError> {
    match device.default_input_config() {
        Ok(config) if is_supported_format(config.sample_format()) => return Ok(config),
        _ => {}
    }
    let best = device
        .supported_input_configs()
        .map_err(|e| RecorderError(format!("unable to query input formats: {e}")))?
        .filter(|range| is_supported_format(range.sample_format()))
        .max_by(|a, b| a.cmp_default_heuristics(b))
        .ok_or_else(|| RecorderError("no supported microphone input format".to_string()))?;
    Ok(best
        .try_with_standard_sample_rate()
        .unwrap_or_else(|| best.with_max_sample_rate()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh 48 kHz stereo capture buffer.
    fn buffer() -> CaptureBuffer {
        CaptureBuffer {
            samples: Vec::new(),
            channels: 2,
            sample_rate: 48_000,
        }
    }

    // ---- Buffer accumulation and transfer at stop (plan step 3) ----

    #[test]
    fn append_accumulates_and_finish_transfers_normalized_audio() {
        let mut b = buffer();
        // 6 stereo frames of DC 0.5: interleaved L/R = 0.5, 0.5 repeated.
        for _ in 0..3 {
            b.append(&[0.5, 0.5, 0.5, 0.5]);
        }

        let audio = b.finish().expect("non-empty capture finishes");
        // 6 frames downmix to mono 0.5; resample 48k -> 16k yields
        // floor(6 * 16000 / 48000) = 2 samples.
        assert_eq!(audio.sample_rate, audio::WHISPER_SAMPLE_RATE);
        assert_eq!(audio.samples.len(), 2);
        assert!(audio.samples.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn finish_preserves_signal_through_downmix_and_resample() {
        // Ramp: frame i holds L = i, R = i. Downmix -> i. Resample 2k->2k
        // (equal rate) -> unchanged. Use matching rates to isolate downmix.
        let mut b = CaptureBuffer {
            samples: Vec::new(),
            channels: 2,
            sample_rate: 16_000,
        };
        b.append(&[0.0, 0.0, 1.0, 1.0, 2.0, 2.0]);
        let audio = b.finish().unwrap();
        assert_eq!(audio.samples, vec![0.0, 1.0, 2.0]);
    }

    // ---- Cancel discards (plan step 4, functional spec 4.2) ----

    #[test]
    fn cleared_buffer_is_empty_and_cannot_produce_audio() {
        let mut b = buffer();
        b.append(&[0.25; 48]);
        b.samples.clear(); // what cancel does: drop the samples
        let err = b.finish().expect_err("cleared capture must not finish");
        assert_eq!(err.0, "no audio was captured");
    }

    #[test]
    fn empty_finish_errors_and_never_submits() {
        let err = buffer().finish().expect_err("empty capture errors");
        assert_eq!(err.0, "no audio was captured");
    }

    // ---- Repeated cleanup safety (plan step 4) ----

    #[test]
    fn finish_consumes_and_empty_capture_errors_cleanly() {
        let mut b = buffer();
        b.append(&[0.1, 0.1]);
        assert!(b.finish().is_ok());
        // A fresh capture that never received samples errors; nothing is
        // produced, so cleanup is stable across repeated cycles.
        let err = buffer().finish().expect_err("empty capture errors");
        assert_eq!(err.0, "no audio was captured");
    }

    #[test]
    fn repeated_append_is_accumulative_not_duplicating() {
        let mut b = buffer();
        b.append(&[1.0, 2.0]);
        b.append(&[3.0, 4.0]);
        assert_eq!(b.samples, vec![1.0, 2.0, 3.0, 4.0]);
    }

    // ---- Start/config policy (decision D4) ----

    #[test]
    fn supported_formats_exclude_dsd_and_64bit_integers() {
        for format in [
            cpal::SampleFormat::I8,
            cpal::SampleFormat::I16,
            cpal::SampleFormat::I24,
            cpal::SampleFormat::I32,
            cpal::SampleFormat::U8,
            cpal::SampleFormat::U16,
            cpal::SampleFormat::U24,
            cpal::SampleFormat::U32,
            cpal::SampleFormat::F32,
            cpal::SampleFormat::F64,
        ] {
            assert!(is_supported_format(format), "{format:?} must be supported");
        }
        for format in [
            cpal::SampleFormat::DsdU8,
            cpal::SampleFormat::DsdU16,
            cpal::SampleFormat::DsdU32,
        ] {
            assert!(!is_supported_format(format), "{format:?} must be rejected");
        }
    }

    #[test]
    fn data_to_f32_rejects_dsd_buffers_defensively() {
        // A DSD buffer cannot be produced by this recorder (start rejects the
        // format), but the callback must return no samples rather than panic.
        // Build one through the raw constructor to prove the wildcard arm.
        let mut bytes = [0x69u8; 4];
        let data = unsafe {
            cpal::Data::from_parts(bytes.as_mut_ptr().cast(), 4, cpal::SampleFormat::DsdU8)
        };
        assert_eq!(data.sample_format(), cpal::SampleFormat::DsdU8);
        assert!(data_to_f32(&data).is_empty());
    }
}
