//! Transclip — terminal voice transcriber.
//!
//! Slice 1: startup and model readiness. `main` performs startup, dependency
//! initialization (model provisioning and loading), terminal initialization,
//! app-loop invocation, final cleanup, and process-level error reporting.
//! It contains minimal application logic (architecture section 31).

mod app;
mod recorder;
mod terminal;
mod transcriber;

use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc;

use anyhow::Context;
use app::{App, AppEvent, TranscriptionJob};
use recorder::{Recorder, UnavailableRecorder};
use transcriber::{Downloader, ModelRelease, ensure_model, load_model, model_cache_dir};

/// Provisions and loads the pinned model. Any failure here prevents the
/// application loop from ever starting (functional spec 15.5, architecture
/// section 14). The returned context is held alive for the session; slice 5
/// hands ownership to the long-lived transcription worker (decision D2).
fn startup(
    cache_dir: &Path,
    release: &ModelRelease,
    downloader: &dyn Downloader,
) -> anyhow::Result<whisper_rs::WhisperContext> {
    let (model_path, outcome) =
        ensure_model(cache_dir, release, downloader).context("model provisioning failed")?;
    if outcome == transcriber::ProvisionOutcome::Downloaded {
        eprintln!("transclip: Whisper model downloaded and verified");
    }
    load_model(&model_path).context("Whisper model failed to load")
}

/// Runs the full startup sequence and the interactive shell.
///
/// Ordering guarantees:
/// - The model is provisioned and loaded before any interactive state; any
///   failure exits non-zero without enabling recording.
/// - Terminal raw mode is entered only after model readiness, so a failed
///   download or load never leaves the terminal partially configured.
fn run() -> anyhow::Result<()> {
    // Decision D3: refuse unsupported runtimes with a clear startup error.
    if std::env::consts::OS != "linux" {
        return Err(transcriber::ModelError::UnsupportedRuntime {
            os: std::env::consts::OS,
        }
        .into());
    }

    let cache_dir = model_cache_dir(&transcriber::data_dir()?);
    let _model = startup(
        &cache_dir,
        &transcriber::BASE_MODEL,
        &transcriber::HttpDownloader,
    )?;

    // Slice 2 wires the controller boundaries. The recorder is a placeholder
    // until slice 3 attaches CPAL (it reports a clear error, so the Ready
    // shell and the recorder-error path run end to end). The worker threads
    // arrive in slice 4; the channels are created here so the controller
    // boundaries stay fixed: `job_tx` feeds the future worker, `event_rx`
    // feeds the loop, and `_event_tx` is held by main until the worker
    // exists.
    let recorder: Box<dyn Recorder> = Box::new(UnavailableRecorder);
    let (job_tx, _job_rx) = mpsc::channel::<TranscriptionJob>();
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();
    let _event_tx = event_tx;

    let mut app = App::new(recorder, job_tx);
    let mut renderer = terminal::TerminalRenderer;

    let mut terminal_guard =
        terminal::TerminalGuard::enter().context("terminal initialization failed")?;
    let result = app::run(&mut app, event_rx, &mut renderer);
    let _ = terminal_guard.restore();
    result.map_err(Into::into)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest as _;

    /// Fixture release whose sha256 is derived from the payload bytes.
    fn fixture_release(payload: &[u8]) -> ModelRelease {
        ModelRelease {
            filename: "fixture.bin",
            url: "https://example.invalid/fixture.bin",
            sha256: Box::leak(hex::encode(sha2::Sha256::digest(payload)).into_boxed_str()),
        }
    }

    #[test]
    fn startup_fails_when_model_cannot_be_loaded() {
        // A download that yields a valid-checksum but non-loadable artifact
        // must fail startup (and thereby prevent the application loop).
        let temp = tempfile::tempdir().unwrap();
        let payload = b"valid checksum, not a ggml model".to_vec();

        let err = startup(
            temp.path(),
            &fixture_release(&payload),
            &FakeDownloader(payload),
        )
        .expect_err("garbage model must fail startup");
        assert!(format!("{err:#}").contains("failed to load Whisper model from"));
    }

    #[test]
    fn startup_fails_on_checksum_mismatch_before_loading() {
        let temp = tempfile::tempdir().unwrap();

        let err = startup(
            temp.path(),
            &fixture_release(b"expected"),
            &FakeDownloader(b"tampered".to_vec()),
        )
        .expect_err("checksum mismatch must fail startup");
        assert!(format!("{err:#}").contains("SHA-256 verification failed"));
    }

    struct FakeDownloader(Vec<u8>);

    impl Downloader for FakeDownloader {
        fn download_to(
            &self,
            _url: &str,
            dest: &Path,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            std::fs::write(dest, &self.0)?;
            Ok(())
        }
    }
}
