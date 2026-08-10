//! Whisper model provisioning, loading, and the long-lived transcription
//! worker.
//!
//! The pinned model artifact is release metadata, not user configuration
//! (architecture section 42). It is downloaded once, SHA-256 verified, and
//! atomically installed into the Linux user data directory before the first
//! usable session (functional spec 2.1, 15.5; architecture section 14).
//! A cached artifact is re-verified at every process start and re-downloaded
//! when invalid or corrupt.
//!
//! Slice 4 adds the transcription worker (architecture sections 13-15,
//! ADR-09/10; decision D5): one long-lived worker thread owns the loaded
//! model and processes one job at a time, so inference never runs on the
//! terminal event loop and the model is never reloaded between cycles. The
//! worker starts before Ready and reports startup success/failure through a
//! handshake.
//!
//! Provenance (decision D1):
//! - Artifact: `ggml-base.bin`, multilingual Whisper `base` (float32).
//! - Source: https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
//! - Size: 147,951,465 bytes.
//! - SHA-256 verified by full download on 2026-08-10; matches the HF
//!   `x-linked-etag` for that artifact.

use std::any::Any;
use std::fs;
use std::io::{self, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use thiserror::Error;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::app::{AppEvent, TranscriptionJob};
use crate::recorder::RecordedAudio;

/// Pinned model release metadata (decision D1).
pub struct ModelRelease {
    /// Artifact filename within the cache directory.
    pub filename: &'static str,
    /// HTTPS source URL of the artifact.
    pub url: &'static str,
    /// Expected SHA-256 of the artifact, lowercase hex.
    pub sha256: &'static str,
}

/// The pinned multilingual Whisper `base` artifact.
pub const BASE_MODEL: ModelRelease = ModelRelease {
    filename: "ggml-base.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
    sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
};

/// Outcome of `ensure_model`, used for provisioning feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionOutcome {
    /// A verified artifact was already cached and reused.
    Reused,
    /// The artifact was downloaded, verified, and installed (or replaced).
    Downloaded,
}

/// Abstraction over the artifact downloader so tests can run offline.
pub trait Downloader {
    /// Streams the artifact at `url` into `dest`. Must not leave a usable file
    /// behind on failure.
    fn download_to(
        &self,
        url: &str,
        dest: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// HTTPS downloader backed by ureq (rustls, no OpenSSL — decision D3).
pub struct HttpDownloader;

impl Downloader for HttpDownloader {
    fn download_to(
        &self,
        url: &str,
        dest: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut response = ureq::get(url)
            .header(
                "User-Agent",
                concat!("transclip/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut file = fs::File::create(dest)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        io::copy(&mut response.body_mut().as_reader(), &mut file)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("no Linux data directory is available (is $XDG_DATA_HOME or $HOME set?)")]
    NoDataDirectory,
    #[error("failed to create model cache directory {path}: {source}")]
    CreateCacheDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to download model from {url}: {source}")]
    Download {
        url: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("failed to read {path} for verification: {source}")]
    ReadForVerify {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "SHA-256 verification failed for {path}: expected {expected}, got {actual};\n\
         the download or cached artifact is corrupt. It has been removed; retry to re-download."
    )]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("failed to install verified model at {path}: {source}")]
    Install {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to load Whisper model from {path}: {source}")]
    Load {
        path: PathBuf,
        #[source]
        source: whisper_rs::WhisperError,
    },
    #[error("unsupported runtime: {os}; Transclip requires Linux (decision D3)")]
    UnsupportedRuntime { os: &'static str },
    #[error("failed to spawn transcription worker: {source}")]
    WorkerSpawn {
        #[source]
        source: io::Error,
    },
    #[error("transcription worker failed to start: {message}")]
    WorkerStartup { message: String },
}

/// The Linux user data directory per XDG (decision D3): `$XDG_DATA_HOME`, else
/// `~/.local/share`.
pub fn data_dir() -> Result<PathBuf, ModelError> {
    dirs::data_dir().ok_or(ModelError::NoDataDirectory)
}

/// Model cache directory: `<data dir>/transclip/models`.
pub fn model_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("transclip").join("models")
}

/// Provisions the pinned model artifact, returning its verified path.
///
/// Reuses a cached artifact only when its SHA-256 matches the release
/// metadata. Otherwise downloads to a temporary `.part` file inside the cache
/// directory, verifies it, and atomically installs it with `fs::rename`.
/// Temporary files are removed on any failure (decision D1).
pub fn ensure_model(
    cache_dir: &Path,
    release: &ModelRelease,
    downloader: &dyn Downloader,
) -> Result<(PathBuf, ProvisionOutcome), ModelError> {
    let model_path = cache_dir.join(release.filename);

    if model_path.exists() {
        match sha256_of(&model_path) {
            Ok(actual) if actual == release.sha256 => {
                return Ok((model_path, ProvisionOutcome::Reused));
            }
            Ok(_) => {
                // Corrupt or stale artifact: delete it and re-download below.
                fs::remove_file(&model_path).map_err(|source| ModelError::Install {
                    path: model_path.clone(),
                    source,
                })?;
            }
            Err(err) => return Err(err),
        }
    }

    // Standard XDG data-dir permissions (0755 dirs, 0644 files under umask);
    // the cache is single-user (decision D1).
    fs::create_dir_all(cache_dir).map_err(|source| ModelError::CreateCacheDir {
        path: cache_dir.to_path_buf(),
        source,
    })?;

    let temp_path = cache_dir.join(format!("{}.part", release.filename));
    // Remove a stale partial download left by an interrupted previous run.
    let _ = fs::remove_file(&temp_path);

    downloader
        .download_to(release.url, &temp_path)
        .map_err(|source| ModelError::Download {
            url: release.url.to_string(),
            source,
        })?;

    let actual = sha256_of(&temp_path)?;
    if actual != release.sha256 {
        let _ = fs::remove_file(&temp_path);
        return Err(ModelError::ChecksumMismatch {
            path: temp_path,
            expected: release.sha256.to_string(),
            actual,
        });
    }

    // Atomic on the same filesystem: the verified artifact only appears at
    // `model_path` once the checksum is known to match.
    fs::rename(&temp_path, &model_path).map_err(|source| ModelError::Install {
        path: model_path.clone(),
        source,
    })?;
    Ok((model_path, ProvisionOutcome::Downloaded))
}

// ---------------------------------------------------------------------------
// Transcription worker (slice 4; architecture sections 13-15, decision D5)
// ---------------------------------------------------------------------------

/// How long `main` waits for the worker to stop during shutdown before
/// proceeding to exit; on timeout the process exits and teardown terminates
/// the worker thread (decision D5).
pub const WORKER_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// One transcription operation (architecture section 13, decision D2). The
/// worker owns exactly one implementation for the process lifetime; a fake
/// implementation is injected in hardware-free tests.
pub trait Transcriber: Send {
    /// Runs one transcription of `audio` and returns plain text on success or
    /// an opaque message on failure. `cancel` is the flag shared with the
    /// controller (architecture section 16): the implementation polls it
    /// (whisper abort callback) so cancellation is cooperative. The caller
    /// (the worker) decides the terminal outcome — completed, cancelled, or
    /// failed — from the result together with `cancel`.
    fn transcribe(
        &mut self,
        audio: &RecordedAudio,
        cancel: &Arc<AtomicBool>,
    ) -> Result<String, String>;
}

/// Whisper-backed transcriber owning the loaded model and one reusable
/// decoding state (architecture section 14, ADR-09/10). The state holds the
/// model alive internally (`WhisperState` keeps an `Arc` clone of the
/// context), so the worker owns the native resources without sharing them.
pub struct WhisperTranscriber {
    state: WhisperState,
}

impl WhisperTranscriber {
    /// Takes ownership of the already-loaded model and creates the reusable
    /// decoding state. Failure here is a worker startup failure reported by
    /// the startup handshake before Ready (decision D5).
    pub fn new(context: WhisperContext) -> Result<Self, String> {
        // The app owns the terminal, so whisper.cpp and GGML logs (model
        // init INFO lines, and — in debug builds — per-token WHISPER_DEBUG
        // dumps) must not reach stdout. With no `log_backend`/`tracing_backend`
        // feature enabled, `install_logging_hooks` routes them into a no-op
        // trampoline (whisper-rs 0.16 `install_logging_hooks`). Safe to call
        // multiple times; only the first call has an effect.
        whisper_rs::install_logging_hooks();
        let state = context.create_state().map_err(|e| e.to_string())?;
        Ok(Self { state })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(
        &mut self,
        audio: &RecordedAudio,
        cancel: &Arc<AtomicBool>,
    ) -> Result<String, String> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        // whisper.cpp prints progress and timestamps to stdout by default
        // (verified in whisper_full_default_params, whisper.cpp 1.8.3); the
        // app owns the terminal, so everything whisper would print is turned
        // off. Final text is extracted below instead (functional spec 13).
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // Language: whisper.cpp's default language is "en"
        // (whisper_full_default_params, whisper.cpp 1.8.3 — decision D5).
        // `set_detect_language(true)` is deliberately NOT used: empirically it
        // returns zero segments on the pinned multilingual base model in this
        // build (verified 2026-08-10 against jfk.wav: greedy and beam both
        // yield segments=0 with detection on, while the default language
        // yields the expected transcript), so detection would make every real
        // transcription empty (functional spec 19.9).
        // Cooperative cancellation (decision D2): whisper.cpp polls this
        // callback between encode/decode passes; on true it aborts and
        // returns an error code (verified in the vendored whisper.cpp 1.8.3).
        let cancel = cancel.clone();
        let abort: Box<dyn FnMut() -> bool> = Box::new(move || cancel.load(Ordering::SeqCst));
        params
            .set_abort_callback_safe::<Option<Box<dyn FnMut() -> bool>>, Box<dyn FnMut() -> bool>>(
                Some(abort),
            );

        self.state
            .full(params, &audio.samples)
            .map_err(|e| e.to_string())?;

        // Final plain text only; partial output is never surfaced. Segments
        // are joined with newlines and trimmed so the transcript reads
        // naturally in the terminal.
        let mut text = String::new();
        for i in 0..self.state.full_n_segments() {
            if let Some(segment) = self.state.get_segment(i) {
                let segment = segment.to_str_lossy().map_err(|e| e.to_string())?;
                text.push_str(segment.trim());
                text.push('\n');
            }
        }
        Ok(text.trim_end().to_string())
    }
}

/// Handle to the long-lived transcription worker (decision D5).
#[derive(Debug)]
pub struct WorkerHandle {
    join: thread::JoinHandle<()>,
}

impl WorkerHandle {
    /// Waits up to `timeout` for the worker to stop. `true` when the worker
    /// exited within the timeout (its thread was joined); `false` when it is
    /// still running — the caller proceeds to exit and process teardown
    /// terminates it (decision D5).
    pub fn join_with_timeout(self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while !self.join.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.join.join();
        true
    }
}

/// Starts the long-lived transcription worker and waits for its startup
/// handshake, so the app only becomes Ready after the worker reports
/// success (decision D5; architecture section 15).
///
/// `factory` runs on the worker thread and produces the transcriber the
/// worker owns for the process lifetime; `Err` (or a factory panic, or an
/// early thread exit) is a startup failure that fails `main` before any
/// interactive state is entered.
pub fn spawn_worker(
    factory: impl FnOnce() -> Result<Box<dyn Transcriber>, String> + Send + 'static,
    jobs: mpsc::Receiver<TranscriptionJob>,
    events: mpsc::Sender<AppEvent>,
) -> Result<WorkerHandle, ModelError> {
    let (start_tx, start_rx) = mpsc::channel::<Result<(), String>>();
    let join = thread::Builder::new()
        .name("transcription-worker".to_string())
        .spawn(move || match factory() {
            Ok(transcriber) => {
                let _ = start_tx.send(Ok(()));
                worker_loop(transcriber, jobs, events);
            }
            Err(message) => {
                let _ = start_tx.send(Err(message));
            }
        })
        .map_err(|source| ModelError::WorkerSpawn { source })?;
    match start_rx.recv() {
        Ok(Ok(())) => Ok(WorkerHandle { join }),
        Ok(Err(message)) => Err(ModelError::WorkerStartup { message }),
        // Factory panicked or the thread died before reporting: never hang.
        Err(_) => Err(ModelError::WorkerStartup {
            message: "worker exited before reporting startup".to_string(),
        }),
    }
}

/// The worker's job loop (architecture section 15, decision D5): accepts at
/// most one job at a time and reports exactly one terminal outcome per
/// accepted job. Exits when the job channel closes (all senders dropped) or
/// when the controller's event receiver is gone (shutdown).
fn worker_loop(
    mut transcriber: Box<dyn Transcriber>,
    jobs: mpsc::Receiver<TranscriptionJob>,
    events: mpsc::Sender<AppEvent>,
) {
    while let Ok(job) = jobs.recv() {
        let outcome = process_job(&mut *transcriber, &job);
        if events.send(outcome).is_err() {
            break; // controller is gone: shutdown
        }
    }
}

/// Runs one job to exactly one terminal outcome (plan step 3, decision D5):
/// cancelled when the shared flag is set (before or during inference),
/// completed with plain text otherwise, failed on error or panic. A panicking
/// job becomes a failure and never strands the controller in `Transcribing`.
fn process_job(transcriber: &mut dyn Transcriber, job: &TranscriptionJob) -> AppEvent {
    let id = job.id;
    if job.cancel.load(Ordering::SeqCst) {
        return AppEvent::TranscriptionCancelled { id };
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        transcriber.transcribe(&job.audio, &job.cancel)
    }));
    match result {
        Ok(Ok(text)) if !job.cancel.load(Ordering::SeqCst) => {
            AppEvent::TranscriptionCompleted { id, text }
        }
        Ok(Err(message)) if !job.cancel.load(Ordering::SeqCst) => {
            AppEvent::TranscriptionFailed { id, message }
        }
        // Cancellation has precedence even when inference happened to finish
        // or fail at the same moment (decision R3): report the cancellation.
        Ok(Ok(_)) | Ok(Err(_)) => AppEvent::TranscriptionCancelled { id },
        Err(panic) => AppEvent::TranscriptionFailed {
            id,
            message: format!("worker panicked: {}", panic_message(&panic)),
        },
    }
}

/// Human-readable payload of a caught panic.
fn panic_message(panic: &Box<dyn Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Loads the verified model artifact once. `Err` on any load failure; the
/// caller must not enter the Ready state in that case (architecture section 14).
pub fn load_model(path: &Path) -> Result<WhisperContext, ModelError> {
    WhisperContext::new_with_params(path, WhisperContextParameters::default()).map_err(|source| {
        ModelError::Load {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// SHA-256 of a file as lowercase hex, streamed (no full-file buffer).
fn sha256_of(path: &Path) -> Result<String, ModelError> {
    let mut file = fs::File::open(path).map_err(|source| ModelError::ReadForVerify {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|source| ModelError::ReadForVerify {
                path: path.to_path_buf(),
                source,
            })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::Write as _;
    use std::sync::atomic::AtomicUsize;

    /// Fake downloader that writes `payload` and counts invocations.
    struct FakeDownloader {
        calls: Cell<u32>,
        payload: Vec<u8>,
    }

    impl FakeDownloader {
        fn new(payload: Vec<u8>) -> Self {
            Self {
                calls: Cell::new(0),
                payload,
            }
        }
    }

    impl Downloader for FakeDownloader {
        fn download_to(
            &self,
            _url: &str,
            dest: &Path,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.calls.set(self.calls.get() + 1);
            let mut file = fs::File::create(dest)?;
            file.write_all(&self.payload)?;
            Ok(())
        }
    }

    /// Failing downloader that never writes anything.
    struct FailingDownloader;

    impl Downloader for FailingDownloader {
        fn download_to(
            &self,
            _url: &str,
            _dest: &Path,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            Err(Box::new(io::Error::other("simulated network failure")))
        }
    }

    fn fixture_release(payload: &[u8]) -> ModelRelease {
        ModelRelease {
            filename: "fixture.bin",
            url: "https://example.invalid/fixture.bin",
            sha256: Box::leak(hex::encode(Sha256::digest(payload)).into_boxed_str()),
        }
    }

    const PAYLOAD: &[u8] = b"transclip fixture model payload";

    #[test]
    fn cache_path_resolution_is_deterministic() {
        let base = Path::new("/injected/data/dir");
        assert_eq!(
            model_cache_dir(base),
            PathBuf::from("/injected/data/dir/transclip/models")
        );
    }

    #[test]
    fn missing_model_downloads_verifies_and_installs() {
        let temp = tempfile::tempdir().unwrap();
        let downloader = FakeDownloader::new(PAYLOAD.to_vec());
        let (path, outcome) =
            ensure_model(temp.path(), &fixture_release(PAYLOAD), &downloader).unwrap();

        assert_eq!(outcome, ProvisionOutcome::Downloaded);
        assert_eq!(path, temp.path().join("fixture.bin"));
        assert_eq!(downloader.calls.get(), 1);
        assert_eq!(fs::read(&path).unwrap(), PAYLOAD);
        // No leftover temp file after a successful install.
        assert!(!temp.path().join("fixture.bin.part").exists());
    }

    #[test]
    fn valid_cached_model_skips_download() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = temp.path().join("fixture.bin");
        fs::write(&model_path, PAYLOAD).unwrap();

        let downloader = FakeDownloader::new(vec![]);
        let (path, outcome) =
            ensure_model(temp.path(), &fixture_release(PAYLOAD), &downloader).unwrap();

        assert_eq!(outcome, ProvisionOutcome::Reused);
        assert_eq!(path, model_path);
        assert_eq!(downloader.calls.get(), 0);
    }

    #[test]
    fn corrupt_cached_model_is_deleted_and_redownloaded() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("fixture.bin"), b"corrupt bytes").unwrap();

        let downloader = FakeDownloader::new(PAYLOAD.to_vec());
        let (_, outcome) =
            ensure_model(temp.path(), &fixture_release(PAYLOAD), &downloader).unwrap();

        assert_eq!(outcome, ProvisionOutcome::Downloaded);
        assert_eq!(downloader.calls.get(), 1);
        assert_eq!(fs::read(temp.path().join("fixture.bin")).unwrap(), PAYLOAD);
    }

    #[test]
    fn stale_partial_download_is_replaced() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("fixture.bin.part"), b"partial").unwrap();

        let downloader = FakeDownloader::new(PAYLOAD.to_vec());
        ensure_model(temp.path(), &fixture_release(PAYLOAD), &downloader).unwrap();

        assert_eq!(downloader.calls.get(), 1);
        assert_eq!(fs::read(temp.path().join("fixture.bin")).unwrap(), PAYLOAD);
        assert!(!temp.path().join("fixture.bin.part").exists());
    }

    #[test]
    fn checksum_mismatch_fails_without_installing() {
        let temp = tempfile::tempdir().unwrap();
        let downloader = FakeDownloader::new(b"tampered payload".to_vec());

        let err = ensure_model(temp.path(), &fixture_release(PAYLOAD), &downloader)
            .expect_err("checksum mismatch must fail");

        assert!(matches!(err, ModelError::ChecksumMismatch { .. }));
        assert!(!temp.path().join("fixture.bin").exists());
        // The failed temp file must not survive.
        assert!(!temp.path().join("fixture.bin.part").exists());
    }

    #[test]
    fn failed_download_never_installs_an_artifact() {
        let temp = tempfile::tempdir().unwrap();

        let err = ensure_model(temp.path(), &fixture_release(PAYLOAD), &FailingDownloader)
            .expect_err("download failure must fail");

        assert!(matches!(err, ModelError::Download { .. }));
        assert!(!temp.path().join("fixture.bin").exists());
        assert!(!temp.path().join("fixture.bin.part").exists());
    }

    #[test]
    fn unreadable_cached_artifact_fails_before_loading() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let model_path = temp.path().join("fixture.bin");
        fs::write(&model_path, PAYLOAD).unwrap();
        fs::set_permissions(&model_path, fs::Permissions::from_mode(0o000)).unwrap();

        let downloader = FakeDownloader::new(vec![]);
        let err = ensure_model(temp.path(), &fixture_release(PAYLOAD), &downloader)
            .expect_err("unreadable cache must fail");

        assert!(matches!(err, ModelError::ReadForVerify { .. }));
        assert_eq!(downloader.calls.get(), 0);
    }

    #[test]
    fn load_failure_on_garbage_model() {
        let temp = tempfile::tempdir().unwrap();
        let garbage = temp.path().join("garbage.bin");
        fs::write(&garbage, b"this is not a ggml model").unwrap();

        assert!(matches!(load_model(&garbage), Err(ModelError::Load { .. })));
    }

    #[test]
    fn load_failure_on_missing_file() {
        let temp = tempfile::tempdir().unwrap();
        assert!(matches!(
            load_model(&temp.path().join("does-not-exist.bin")),
            Err(ModelError::Load { .. })
        ));
    }

    // ---- Worker protocol (slice 4, decision D5) ----

    /// Shared counters a test uses to observe a fake transcriber living on
    /// the worker thread.
    #[derive(Clone, Default)]
    struct FakeProbe {
        calls: Arc<AtomicUsize>,
        in_flight: Arc<AtomicUsize>,
        max_concurrent: Arc<AtomicUsize>,
    }

    /// Per-call scripted behavior of the fake transcriber.
    #[derive(Clone)]
    enum FakeBehavior {
        /// Sleep `delay`, then succeed with `result <call#>`.
        Success { delay: Duration },
        /// Sleep `delay`, then fail with `message`.
        Fail { delay: Duration, message: String },
        /// Set the shared cancel flag, then succeed: a completion racing a
        /// cancellation (decision R3).
        SetCancelThenOk,
        /// Set the shared cancel flag, then fail: a failure racing a
        /// cancellation.
        SetCancelThenFail,
        /// Panic mid-transcription.
        Panic,
    }

    /// Fake transcriber for hardware-free worker tests; `transcribe` is
    /// scripted per call (`behaviors` consumed in order, last one repeats).
    struct FakeTranscriber {
        probe: FakeProbe,
        behaviors: Vec<FakeBehavior>,
        next: usize,
    }

    impl FakeTranscriber {
        fn new(probe: FakeProbe, behaviors: Vec<FakeBehavior>) -> Self {
            Self {
                probe,
                behaviors,
                next: 0,
            }
        }
    }

    impl Transcriber for FakeTranscriber {
        fn transcribe(
            &mut self,
            _audio: &RecordedAudio,
            cancel: &Arc<AtomicBool>,
        ) -> Result<String, String> {
            self.probe.calls.fetch_add(1, Ordering::SeqCst);
            let now = self.probe.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.probe.max_concurrent.fetch_max(now, Ordering::SeqCst);

            let behavior =
                self.behaviors
                    .get(self.next)
                    .cloned()
                    .unwrap_or(FakeBehavior::Success {
                        delay: Duration::ZERO,
                    });
            self.next += 1;
            let result = match behavior {
                FakeBehavior::Success { delay } => {
                    thread::sleep(delay);
                    Ok(format!(
                        "result {}",
                        self.probe.calls.load(Ordering::SeqCst)
                    ))
                }
                FakeBehavior::Fail { delay, message } => {
                    thread::sleep(delay);
                    Err(message)
                }
                FakeBehavior::SetCancelThenOk => {
                    cancel.store(true, Ordering::SeqCst);
                    Ok("late text".to_string())
                }
                FakeBehavior::SetCancelThenFail => {
                    cancel.store(true, Ordering::SeqCst);
                    Err("late failure".to_string())
                }
                FakeBehavior::Panic => panic!("simulated worker panic"),
            };
            self.probe.in_flight.fetch_sub(1, Ordering::SeqCst);
            result
        }
    }

    /// A job whose cancellation flag the caller keeps (to set it externally).
    fn make_job(id: u64, cancel: Arc<AtomicBool>) -> TranscriptionJob {
        TranscriptionJob {
            id: crate::app::TranscriptionId::new(id),
            audio: RecordedAudio {
                samples: vec![0.0; 8],
                sample_rate: 16_000,
            },
            cancel,
        }
    }

    /// Spawns a worker owning a scripted fake transcriber.
    fn spawn_fake_worker(
        behaviors: Vec<FakeBehavior>,
    ) -> (
        WorkerHandle,
        FakeProbe,
        mpsc::SyncSender<TranscriptionJob>,
        mpsc::Receiver<AppEvent>,
    ) {
        let probe = FakeProbe::default();
        let fake = FakeTranscriber::new(probe.clone(), behaviors);
        let (job_tx, job_rx) = mpsc::sync_channel::<TranscriptionJob>(1);
        let (event_tx, event_rx) = mpsc::channel::<AppEvent>();
        let handle = spawn_worker(
            move || Ok::<Box<dyn Transcriber>, String>(Box::new(fake)),
            job_rx,
            event_tx,
        )
        .expect("fake worker must start");
        (handle, probe, job_tx, event_rx)
    }

    fn completed(id: u64, text: &str) -> AppEvent {
        AppEvent::TranscriptionCompleted {
            id: crate::app::TranscriptionId::new(id),
            text: text.to_string(),
        }
    }

    fn cancelled(id: u64) -> AppEvent {
        AppEvent::TranscriptionCancelled {
            id: crate::app::TranscriptionId::new(id),
        }
    }

    fn failed(id: u64, message: &str) -> AppEvent {
        AppEvent::TranscriptionFailed {
            id: crate::app::TranscriptionId::new(id),
            message: message.to_string(),
        }
    }

    #[test]
    fn worker_processes_one_job_at_a_time_and_reuses_its_transcriber() {
        let (handle, probe, job_tx, event_rx) = spawn_fake_worker(vec![FakeBehavior::Success {
            delay: Duration::from_millis(20),
        }]);

        // Two jobs submitted back to back; the bounded channel holds one and
        // the worker must serialize them (max_concurrent stays 1).
        job_tx
            .send(make_job(1, Arc::new(AtomicBool::new(false))))
            .unwrap();
        job_tx
            .send(make_job(2, Arc::new(AtomicBool::new(false))))
            .unwrap();

        assert_eq!(
            event_rx.recv().unwrap(),
            completed(1, "result 1"),
            "one outcome per accepted job, in order"
        );
        assert_eq!(event_rx.recv().unwrap(), completed(2, "result 2"));
        assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            probe.max_concurrent.load(Ordering::SeqCst),
            1,
            "the worker never runs two transcriptions at once"
        );

        drop(job_tx);
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn worker_acknowledges_only_after_the_job_is_released() {
        // Plan step 6 / ADR-11: the worker must emit its terminal outcome
        // only after the active job has stopped and released the single
        // model owner. While `transcribe` is still running, no event may
        // reach the controller — otherwise the controller could start a new
        // cycle while the model is still busy.
        let (handle, probe, job_tx, event_rx) = spawn_fake_worker(vec![FakeBehavior::Success {
            delay: Duration::from_millis(150),
        }]);

        job_tx
            .send(make_job(1, Arc::new(AtomicBool::new(false))))
            .unwrap();

        // Mid-inference: the job is still in flight and no acknowledgement
        // has been emitted.
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            probe.in_flight.load(Ordering::SeqCst),
            1,
            "transcribe must still be running"
        );
        assert!(
            event_rx.try_recv().is_err(),
            "no acknowledgement while the job is in flight (plan step 6)"
        );

        // Only once transcribe has returned (the job released the model) is
        // the single terminal outcome sent.
        assert_eq!(event_rx.recv().unwrap(), completed(1, "result 1"));
        assert_eq!(
            probe.in_flight.load(Ordering::SeqCst),
            0,
            "the job must be released before the acknowledgement"
        );

        drop(job_tx);
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn pre_set_cancel_flag_reports_cancelled_without_inference() {
        let (handle, probe, job_tx, event_rx) = spawn_fake_worker(vec![]);

        job_tx
            .send(make_job(1, Arc::new(AtomicBool::new(true))))
            .unwrap();
        assert_eq!(event_rx.recv().unwrap(), cancelled(1));
        assert_eq!(
            probe.calls.load(Ordering::SeqCst),
            0,
            "no inference runs for a job cancelled before processing"
        );

        drop(job_tx);
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn completion_racing_cancellation_reports_cancelled() {
        let (handle, _probe, job_tx, event_rx) =
            spawn_fake_worker(vec![FakeBehavior::SetCancelThenOk]);

        job_tx
            .send(make_job(1, Arc::new(AtomicBool::new(false))))
            .unwrap();
        assert_eq!(
            event_rx.recv().unwrap(),
            cancelled(1),
            "a completion racing a cancellation must not be reported (R3)"
        );

        drop(job_tx);
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn failure_racing_cancellation_reports_cancelled() {
        let (handle, _probe, job_tx, event_rx) =
            spawn_fake_worker(vec![FakeBehavior::SetCancelThenFail]);

        job_tx
            .send(make_job(1, Arc::new(AtomicBool::new(false))))
            .unwrap();
        assert_eq!(
            event_rx.recv().unwrap(),
            cancelled(1),
            "a failure racing a cancellation must not surface an error (R3)"
        );

        drop(job_tx);
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn inference_failure_reports_failed() {
        let (handle, _probe, job_tx, event_rx) = spawn_fake_worker(vec![FakeBehavior::Fail {
            delay: Duration::ZERO,
            message: "no speech detected".to_string(),
        }]);

        job_tx
            .send(make_job(1, Arc::new(AtomicBool::new(false))))
            .unwrap();
        assert_eq!(event_rx.recv().unwrap(), failed(1, "no speech detected"));

        drop(job_tx);
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn panicking_job_fails_and_the_worker_survives() {
        let (handle, probe, job_tx, event_rx) = spawn_fake_worker(vec![
            FakeBehavior::Panic,
            FakeBehavior::Success {
                delay: Duration::ZERO,
            },
        ]);

        job_tx
            .send(make_job(1, Arc::new(AtomicBool::new(false))))
            .unwrap();
        match event_rx.recv().unwrap() {
            AppEvent::TranscriptionFailed { id, message } => {
                assert_eq!(id, crate::app::TranscriptionId::new(1));
                assert!(
                    message.contains("worker panicked")
                        && message.contains("simulated worker panic"),
                    "panic payload must be surfaced: {message}"
                );
            }
            other => panic!("expected a failed outcome, got {other:?}"),
        }

        // The worker is still alive and processes the next job normally.
        job_tx
            .send(make_job(2, Arc::new(AtomicBool::new(false))))
            .unwrap();
        assert_eq!(event_rx.recv().unwrap(), completed(2, "result 2"));
        assert_eq!(probe.calls.load(Ordering::SeqCst), 2);

        drop(job_tx);
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn worker_exits_when_the_job_channel_closes() {
        let (handle, _probe, job_tx, _event_rx) = spawn_fake_worker(vec![]);

        drop(job_tx); // all senders dropped: the worker's recv errors -> exit
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn worker_exits_when_the_controller_event_receiver_is_gone() {
        let (handle, _probe, job_tx, event_rx) = spawn_fake_worker(vec![]);

        drop(event_rx); // controller is gone: outcome sends fail -> exit
        job_tx
            .send(make_job(1, Arc::new(AtomicBool::new(false))))
            .unwrap();
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn join_with_timeout_reports_busy_worker_false_and_idle_true() {
        // A worker busy for 200 ms is not joined within 50 ms.
        let (tx, rx) = mpsc::channel::<()>();
        let busy = WorkerHandle {
            join: thread::spawn(move || {
                let _ = rx.recv();
            }),
        };
        assert!(!busy.join_with_timeout(Duration::from_millis(50)));
        tx.send(()).unwrap(); // let the thread finish on its own

        // An already-finished worker joins immediately.
        let done = WorkerHandle {
            join: thread::spawn(|| {}),
        };
        assert!(done.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn spawn_worker_reports_startup_failure_before_ready() {
        let (job_tx, job_rx) = mpsc::sync_channel::<TranscriptionJob>(1);
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>();
        let err = spawn_worker(
            || Err::<Box<dyn Transcriber>, String>("state creation failed".to_string()),
            job_rx,
            event_tx,
        )
        .expect_err("a failing factory must fail startup");
        assert!(matches!(
            err,
            ModelError::WorkerStartup { ref message } if message == "state creation failed"
        ));
        drop(job_tx);
    }

    #[test]
    fn spawn_worker_reports_success_and_the_worker_stops_cleanly() {
        let (handle, _probe, job_tx, _event_rx) = spawn_fake_worker(vec![]);
        drop(job_tx);
        assert!(handle.join_with_timeout(Duration::from_secs(1)));
    }

    #[test]
    fn spawn_worker_survives_a_panicking_factory_as_startup_failure() {
        let (job_tx, job_rx) = mpsc::sync_channel::<TranscriptionJob>(1);
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>();
        let err = spawn_worker(
            || -> Result<Box<dyn Transcriber>, String> { panic!("factory boom") },
            job_rx,
            event_tx,
        )
        .expect_err("a panicking factory must fail startup, not hang");
        assert!(matches!(
            err,
            ModelError::WorkerStartup { ref message }
                if message == "worker exited before reporting startup"
        ));
        drop(job_tx);
    }
}
