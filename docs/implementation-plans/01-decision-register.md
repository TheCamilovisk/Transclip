# Decision Register And Implementation Gates

These decisions are absent or contradictory in the specifications. Resolve and record each decision before implementing the affected slice. Do not silently select placeholder values in production code.

## Resolved Interpretations

| ID | Decision | Rationale |
| --- | --- | --- |
| R1 | `Esc` during transcription enters `Transcribing(Cancelling)` until the matching worker outcome arrives. | This is required by functional spec sections 5 and 16 and architecture sections 6, 16, 17, and ADR-11. It supersedes the contradictory immediate-Ready diagram in architecture section 39. |
| R2 | Every transcription gets a monotonically increasing `TranscriptionId`; every terminal worker event includes it. | Architecture section 7 says IDs are mandatory, and they prevent stale events and cancellation races. |
| R3 | A completion received after cancellation was requested is discarded, even if inference finished first. | Cancellation has precedence once the controller processes `Esc`; no text is printed or copied during the cancelling phase. |
| R4 | Clipboard outcome is separate from transcription outcome. | A successful transcription remains printed and successful if copying fails. |

## Blocking Decisions

| ID | Required decision | Affected slices | Completion evidence |
| --- | --- | --- | --- |
| D1 | Pinned model source: compatible multilingual Whisper `base` artifact URL, version/revision, filename, SHA-256, cache directory, cache validation frequency, partial-download cleanup, and atomic replacement strategy. | 1+ | ✅ Resolved during slice 1 — see Slice 1 Gate Resolutions below. |
| D2 | Exact `whisper-rs` version and verified cooperative-cancellation API. Define its callback/abort wiring, expected cancellation latency, and fallback if native interruption is unavailable. | 1, 4, 6, 7 | ✅ Resolved during slice 1 — see Slice 1 Gate Resolutions below. |
| D3 | Linux support baseline and native dependency/packaging policy for CPAL, arboard, and whisper.cpp. | 1, 7 | ✅ Resolved during slice 1 — see Slice 1 Gate Resolutions below. |

## Slice 1 Gate Resolutions (D1–D3)

Resolved 2026-08-10 while implementing slice 1. Blocking rows D1–D3 are satisfied; D4–D6 remain blocking for later slices.

| ID | Resolution | Evidence |
| --- | --- | --- |
| D1 | Pinned artifact: `ggml-base.bin` (multilingual Whisper `base`, float32) from `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin`. SHA-256 `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe`; size 147,951,465 bytes. Cache directory: `$XDG_DATA_HOME/transclip/models/` (default `~/.local/share/transclip/models/`), resolved via `dirs::data_dir()`. Validation frequency: SHA-256 re-verified at every process start; a corrupt or stale cached artifact is deleted and re-downloaded. Partial downloads stream into `<filename>.part` inside the cache directory; any failure (HTTP, write, checksum) removes the temp file; stale `.part` files are removed before a new download. Atomic replacement: verify the temp file, then `fs::rename` (atomic on the same filesystem). Cache dirs/files use standard XDG data-dir permissions (0755 dirs, 0644 files under the default umask); the cache is single-user. | Constants and provenance in `src/transcriber.rs`; checksum, download, and corruption fixture tests in `transcriber::tests`; full-download verification against the HF `x-linked-etag` on 2026-08-10. |
| D2 | `whisper-rs = 0.16.0` (whisper-rs-sys 0.15.0, vendored whisper.cpp 1.8.3; CPU-only default features; whisper.cpp compiled from source via cmake at build time). Load API: `WhisperContext::new_with_params(path, WhisperContextParameters::default())`; the context is `Send + Sync`, so the slice-5 long-lived worker can own it (handoff: `main` keeps the context alive for the session in slice 1). Cooperative cancellation: `FullParams::set_abort_callback_safe(FnMut() -> bool)` is polled by whisper.cpp during inference; exact wiring, cancellation latency, and fallback if native interruption is unavailable are validated in slices 4/6/7. | Vendor source inspected (`whisper-rs` 0.16.0, `whisper-rs-sys` 0.15.0); version pinned in `Cargo.toml`. |
| D3 | Baseline: Linux x86_64 desktop. Build prerequisites: cargo/rustc, cmake ≥ 3.5, a C/C++ compiler, and libclang (bindgen); runtime: glibc, libstdc++, libm, no OpenSSL (HTTPS via rustls). Validated on Ubuntu 24.04 (gcc 13.3, cmake 3.28, clang 18). Unsupported runtime: a non-Linux OS produces a clear startup error and exits non-zero before any interactive state is entered. CPAL/arboard native-dependency policies are deferred to slices 3/5; packaging is deferred to slice 8. | `ModelError::UnsupportedRuntime` path in `src/main.rs`; build prerequisites recorded here. |
| D4 | Audio contract: target Whisper sample rate, supported CPAL sample formats/channel layouts, resampler, silence/empty-recording behavior, and stop/flush failure behavior. | 3+ | Unit fixtures establish conversion/downmix/resampling behavior. |
| D5 | Worker lifecycle: bounded job protocol, startup handshake, submission failure, panic policy, shutdown signal/acknowledgement, join timeout, and repeated Ctrl+C behavior. | 4, 6, 7 | Worker protocol tests cover each terminal outcome. |
| D6 | Terminal operational policy: append/redraw approach, resize handling, runtime I/O failure, raw-mode Ctrl+C behavior, and shutdown wait policy. | 1, 2, 7 | Manual test checklist covers normal and failure restoration. |

## Non-Goals

Do not add settings, global shortcuts, GUI, cloud services, stored audio/history, partial results, concurrent recordings, configurable keys, VAD, or a general job framework. These are outside the initial release.
