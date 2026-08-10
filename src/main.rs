//! Transclip — terminal voice transcriber.
//!
//! Slice 1: startup and model readiness. `main` performs startup, dependency
//! initialization (model provisioning and loading), terminal initialization,
//! app-loop invocation, final cleanup, and process-level error reporting.
//! It contains minimal application logic (architecture section 31).

mod app;
mod audio;
mod clipboard;
mod recorder;
mod terminal;
mod transcriber;

use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc;

use anyhow::Context;
use app::{App, AppEvent, TranscriptionJob};
use clipboard::{ArboardClipboard, Clipboard};
use recorder::{CpalRecorder, Recorder};
use transcriber::{
    Downloader, ModelRelease, WhisperTranscriber, ensure_model, load_model, model_cache_dir,
    spawn_worker,
};

/// Provisions and loads the pinned model. Any failure here prevents the
/// application loop from ever starting (functional spec 15.5, architecture
/// section 14). Slice 4 hands the loaded context to the long-lived
/// transcription worker, which alone runs inference (decision D2).
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
    let model = startup(
        &cache_dir,
        &transcriber::BASE_MODEL,
        &transcriber::HttpDownloader,
    )?;

    // The long-lived transcription worker (architecture sections 13-15,
    // ADR-09/10; decision D5) starts before any interactive state and reports
    // startup success/failure through its handshake: the loaded model is
    // handed to the worker thread, which alone runs inference. The job
    // channel is bounded to one job so the worker never has a backlog; the
    // event channel carries exactly one terminal outcome per accepted job.
    let (job_tx, job_rx) = mpsc::sync_channel::<TranscriptionJob>(1);
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();
    let worker = spawn_worker(
        move || {
            WhisperTranscriber::new(model)
                .map(|transcriber| Box::new(transcriber) as Box<dyn transcriber::Transcriber>)
        },
        job_rx,
        event_tx.clone(),
    )
    .context("transcription worker failed to start")?;

    // The recorder's stream-error sink forwards `RecordingFailed` onto the
    // event channel from the CPAL callback thread (architecture section 32:
    // the recorder itself never depends on app types).
    let recorder: Box<dyn Recorder> = Box::new(CpalRecorder::new({
        let event_tx = event_tx.clone();
        move |message: String| {
            let _ = event_tx.send(AppEvent::RecordingFailed(message));
        }
    }));

    // The clipboard runs on the main thread and is constructed lazily inside
    // `ArboardClipboard` at the first copy, then kept alive for the session
    // (on X11 arboard hosts the selection in the app and hands it to a
    // clipboard manager when the instance drops — see decision D3). Deferring
    // construction means a missing/headless clipboard service can never
    // prevent startup: the failure surfaces as a per-copy warning on the
    // accepted completion path (functional spec 15.4).
    let clipboard: Box<dyn Clipboard> = Box::new(ArboardClipboard::new());

    let mut app = App::new(recorder, clipboard, job_tx);
    let mut renderer = terminal::TerminalRenderer;

    let mut terminal_guard =
        terminal::TerminalGuard::enter().context("terminal initialization failed")?;
    let result = app::run(&mut app, event_rx, &mut renderer);
    let _ = terminal_guard.restore();

    // Shutdown (decision D5): ask an in-flight inference to abort, then wait
    // briefly for the worker to stop. Dropping `App` closes the job channel
    // (it owns the only job sender) and the event receiver is gone with
    // `run`, so the worker exits its loop; on timeout the process exits and
    // teardown terminates the worker thread.
    app.shutdown();
    drop(app);
    let _ = worker.join_with_timeout(transcriber::WORKER_JOIN_TIMEOUT);
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
