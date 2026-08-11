# Transclip Implementation Plan Index

## Goal

Build the Linux-only Rust terminal voice transcriber described by:

- `docs/specifications/functional-spec.md`
- `docs/specifications/architecture-spec.md`

The release workflow is `Ctrl+R` to record, `Ctrl+R` to submit the completed recording for local Whisper transcription, print and copy the final text, then become ready for another cycle. `Esc` cancels recording or requests transcription cancellation. Input is terminal-local, never global.

## Plan Rules

- Implement slices in order. A later slice may depend on and extend earlier code, but must not replace a prior acceptance behavior.
- Keep the source structure flat: `main.rs`, `app.rs`, `terminal.rs`, `recorder.rs`, `transcriber.rs`, `clipboard.rs`, and optional `audio.rs`.
- The main application controller exclusively owns state transitions. Infrastructure returns values or emits events; it must not mutate app state or render directly.
- Use standard threads, `mpsc` channels, `Arc`, and atomics. Do not introduce an async runtime or a full TUI framework.
- Add only small boundary traits that enable hardware-free app tests. Avoid generic abstraction layers.
- Treat all items in `01-decision-register.md` as gates. Do not invent release metadata, native cancellation behavior, or platform guarantees.

## Slice Order

| Order | Document | Delivers | Depends on |
| --- | --- | --- | --- |
| 1 | `02-startup-model-readiness.md` | Runnable startup, verified model readiness, terminal lifecycle, Ready UI | Decision gates D1-D3 |
| 2 | `03-terminal-state-machine.md` | Testable behavior core and focused-terminal command loop | Slice 1 |
| 3 | `04-recording-and-audio.md` | Responsive microphone recording and Whisper-ready in-memory audio | Slice 2, D4 |
| 4 | `05-worker-transcription-flow.md` | Background local transcription, results, and repeated successful cycles | Slices 1-4, D2 |
| 5 | `06-clipboard-completion.md` | Clipboard copy and non-fatal clipboard failures | Slice 4 |
| 6 | `07-transcription-cancellation.md` | Correct worker cancellation and cancellation-race handling | Slices 4-5, D2 |
| 7 | `08-operational-hardening.md` | Runtime recovery, shutdown, and end-to-end acceptance validation | Slices 1-6, D5-D6 |
| 8 | `09-transcript-normalization-and-terminal-line-endings.md` | Canonical transcript text and raw-mode-safe terminal line rendering | Slice 7, D6 |

## Shared Contracts

The implementing AI should establish these contracts during Slice 2 and preserve them:

```rust
enum AppMode { Ready, Recording, Transcribing(/* Running | Cancelling */) }
enum UserCommand { ToggleRecording, Cancel }
struct TranscriptionId(u64);
enum AppEvent {
    TranscriptionCompleted { id: TranscriptionId, text: String },
    TranscriptionCancelled { id: TranscriptionId },
    TranscriptionFailed { id: TranscriptionId, message: String },
    RecordingFailed(String),
}
struct RecordedAudio { samples: Vec<f32>, sample_rate: u32 }
```

Exact ownership and error types can vary, but every terminal worker outcome must carry an ID. The controller accepts an event only when its ID is active and its current phase permits that outcome.

## Global Definition Of Done

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass.
- State-machine tests run with no microphone, clipboard, Whisper model, or interactive terminal.
- No user audio, transcription, or clipboard content is sent over the network. Network access is confined to first-run model provisioning.
- Manual Linux acceptance confirms microphone capture, focused terminal input, local Whisper inference, cancellation responsiveness, X11 and Wayland clipboard behavior, and terminal restoration.
- The full functional acceptance list in functional specification section 19 is traceably covered by slices 1-8.
