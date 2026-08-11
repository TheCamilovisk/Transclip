# Slice 9: Automatic Language Detection

## Outcome

Each completed recording is transcribed in its automatically detected spoken language. Portuguese audio produces Portuguese text rather than English output, and a later recording can be detected independently without restarting the application or reloading the model.

## Prerequisites

- Slices 1-8 are implemented and their automated tests pass.
- The pinned `ggml-base.bin` artifact remains the multilingual Whisper `base` model resolved by decision D1.
- Functional specification sections 2, 6, and 19.19 and architecture specification sections 3.4, 13, 14, and ADR-13 define the required contract.

## Implementation Steps

1. In `src/transcriber.rs`, configure every `FullParams` instance created for an inference request with `params.set_language(None)` before calling `WhisperState::full`.
2. Do not call `params.set_detect_language(true)`. In whisper-rs 0.16 / vendored whisper.cpp 1.8.3, that flag performs language detection and returns success before decoding, which leaves the segment list empty. A null language performs detection and continues transcription.
3. Keep Whisper translation disabled. Do not set a fixed language, add language selection, persist detected languages, change the pinned model, or alter the model-owning worker lifecycle.
4. Preserve the existing cancellation callback, final-text normalization, terminal rendering, and clipboard flow. Detection remains wholly inside the transcriber boundary and returns the same canonical `String` contract to the worker.
5. Record the API behavior and version evidence in `01-decision-register.md` after the compatibility check: `FullParams::set_language(None)` and `set_language(Some("auto"))` trigger detection and continue decoding; `set_detect_language(true)` returns after detection in whisper.cpp.

## Automated Tests

- Existing transcriber worker-protocol, cancellation, normalization, controller, and clipboard tests continue to pass with fakes; no test requires a microphone, Whisper model, clipboard service, or interactive terminal.
- Keep inference parameter setup local to `WhisperTranscriber`; do not add a test-only abstraction around Whisper parameters solely to observe this one library call.

## Manual Checks

- Run through a pseudo-TTY with an isolated `XDG_DATA_HOME` and the verified pinned model.
- Record a Portuguese utterance. Confirm its printed and copied final text is Portuguese, not an English translation and not empty.
- In the same process, record an utterance in another supported language. Confirm it is detected and transcribed in that language without a model reload.
- Confirm cancellation during either transcription still displays cancellation progress, produces no final text or clipboard write, and returns to Ready only after the worker outcome.

## Acceptance Criteria

- Functional specification sections 2, 6, and 19.19 are satisfied.
- Architecture specification sections 3.4, 13, 14, 42, and ADR-13 are satisfied.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass.
- No language configuration, global shortcut, cloud service, model replacement, translation mode, persistence, or worker architecture change is added.
