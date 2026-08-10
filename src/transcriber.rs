//! Whisper model provisioning and loading.
//!
//! The pinned model artifact is release metadata, not user configuration
//! (architecture section 42). It is downloaded once, SHA-256 verified, and
//! atomically installed into the Linux user data directory before the first
//! usable session (functional spec 2.1, 15.5; architecture section 14).
//! A cached artifact is re-verified at every process start and re-downloaded
//! when invalid or corrupt.
//!
//! Provenance (decision D1):
//! - Artifact: `ggml-base.bin`, multilingual Whisper `base` (float32).
//! - Source: https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
//! - Size: 147,951,465 bytes.
//! - SHA-256 verified by full download on 2026-08-10; matches the HF
//!   `x-linked-etag` for that artifact.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;
use whisper_rs::{WhisperContext, WhisperContextParameters};

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
}
