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
| D6 | Terminal operational policy: append/redraw approach, resize handling, runtime I/O failure, raw-mode Ctrl+C behavior, and shutdown wait policy. | 1, 2, 7 | ✅ Resolved during slice 2 — see Slice 2 Gate Resolutions below. |

## Slice 2 Gate Resolutions (D6)

Resolved 2026-08-10 while implementing slice 2. Blocking row D6 is satisfied; D4–D5 remain blocking for later slices.

| ID | Resolution | Evidence |
| --- | --- | --- |
| D6 | Terminal operational policy: **append-only line rendering** — every view change appends a status block below the persistent history (previous statuses, results, errors); no full-screen clear/redraw and no absolute cursor positioning (architecture section 23). **Resize handling**: resize events are non-key events, ignored by the input layer; append-only rendering is layout-independent. **Runtime I/O failure**: any read/write error propagates as `TerminalError` to `main`, which reports and exits non-zero; the RAII `TerminalGuard` restores raw mode and cursor visibility on the way out (architecture section 24). **Raw-mode Ctrl+C**: with raw mode enabled Ctrl+C arrives as a key event, not SIGINT; it remains the exit key (slice-1 behavior, architecture section 29) and initiates the normal shutdown path — slice 2 has no worker, so shutdown discards any active recording (recorder dropped) and restores the terminal; worker-stop waiting is added with the worker (slices 4/6/7, D5). **Bounded input polling**: terminal input is polled with a 100 ms bounded timeout so worker events are drained promptly and inference never blocks the loop (functional spec 10, architecture section 8). **Key mapping** (functional spec 8, architecture section 20): commands are produced only from `KeyEventKind::Press` (Repeat/Release ignored — prevents hold-to-toggle); `Ctrl+R` requires exactly the CONTROL modifier on `Char('r')`; `Esc` maps from `KeyCode::Esc` regardless of modifiers; all other keys ignored. No OS-level key hooks are installed. | Verified against the crossterm 0.29.0 source in the local registry: `KeyEvent { code, modifiers, kind, state }`, `KeyEvent::new` defaults `kind` to `Press`, `KeyEventKind::{Press,Repeat,Release}`, `event::poll(Duration)` / `event::read()`, `Event::{Key,Mouse,Resize,FocusGained,FocusLost,Paste}`. Poll timeout and key-kind/modifier selection are implemented in `src/terminal.rs` (`POLL_TIMEOUT`, `map_key`, `read_event`); append-only rendering in `terminal::TerminalRenderer`; the Ctrl+C shutdown path in `app::run` plus `main.rs`. Manual pty checks on 2026-08-10 cover status rendering, clean exit, and terminal restoration. |

### Slice 2 notes

- Interpretations R1–R4 are applied: `Esc` during transcription enters `Transcribing(Cancelling)` until a matching worker outcome arrives (R1); every transcription carries a monotonic `TranscriptionId` and every terminal worker event includes it (R2); a completion received while cancelling is discarded (R3); clipboard outcome is separate and out of slice-2 scope (R4).
- The plan's automated-test line mentions a fake clipboard; slice 2 introduces no clipboard boundary because clipboard behavior is slice 5 (index row 5, architecture section 18) and adding one would be speculative. App tests substitute the recorder with a fake, the worker sender with a real `mpsc` channel, and the output sink with assertions on `AppOutcome` (view change + output lines) — no real terminal, microphone, or Whisper dependency.
- `RecordedAudio` lives in `src/recorder.rs` (it is the recorder boundary's output, architecture section 10) so that `recorder` does not depend on the controller; the shared-contract shape is preserved. The `terminal` module imports `app::UserCommand` for key mapping because architecture section 20 assigns the key→command mapping to the terminal input layer.

## Non-Goals

Do not add settings, global shortcuts, GUI, cloud services, stored audio/history, partial results, concurrent recordings, configurable keys, VAD, or a general job framework. These are outside the initial release.
