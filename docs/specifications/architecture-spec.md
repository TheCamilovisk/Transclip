# Architecture Specification — Terminal Voice Transcriber

## 1. Purpose

This document defines the software architecture for the Terminal Voice Transcriber.

The application is a local terminal program that allows a user to:

- start and stop microphone recording with `Ctrl+R`;
- cancel recording with `Esc`;
- transcribe completed recordings;
- cancel an active transcription with `Esc`;
- print successful transcription text in the terminal;
- copy successful transcription text to the system clipboard;
- repeat this workflow without restarting the application.

The architecture is intentionally small and is designed around three principles:

1. explicit application state;
2. non-blocking user interaction;
3. clear separation between terminal interaction, audio capture, transcription, and clipboard access.

---

# 2. Architectural Goals

The architecture shall prioritize:

- simplicity;
- responsiveness;
- explicit state transitions;
- local execution;
- minimal dependencies;
- clean cancellation behavior;
- testability of application behavior;
- straightforward future maintenance.

The architecture should avoid introducing abstractions that are not required by the initial product scope.

---

# 3. Technology Stack

## 3.1 Programming Language

**Rust**

Rust is used for the complete application.

It provides:

- native executable generation;
- strong memory and concurrency guarantees;
- good support for terminal and audio applications;
- low runtime overhead;
- interoperability with native speech-to-text libraries.

The initial release targets Linux desktop environments only. Cross-platform support is deferred until Linux behavior is accepted end to end.

---

## 3.2 Terminal Interaction

**Crossterm**

Crossterm is responsible for:

- terminal raw mode;
- keyboard event capture;
- detection of `Ctrl+R`;
- detection of `Esc`;
- terminal rendering;
- cursor and screen manipulation when required.

No global keyboard-hook library shall be used.

Keyboard events are received only through the terminal running the application.

---

## 3.3 Audio Capture

**CPAL**

CPAL is responsible for:

- selecting the available input device;
- opening the microphone input stream;
- receiving audio samples;
- stopping audio capture when recording finishes or is cancelled.

Audio is stored temporarily in memory.

The initial implementation does not require persisted audio files.

---

## 3.4 Speech-to-Text

**whisper-rs backed by whisper.cpp**

The transcription component shall use a local Whisper model through `whisper-rs`.

Responsibilities include:

- accepting recorded audio samples;
- converting them into the format required by Whisper;
- automatically detecting the source language for each completed recording and transcribing in that language;
- executing transcription;
- returning final whitespace-normalized plain-text transcription without decoder-segment line breaks;
- supporting cancellation of active transcription.

Transcription shall execute outside the main terminal event loop.

The initial model is the pinned multilingual Whisper `base` artifact. It is downloaded on first run to the Linux user data directory, verified against a pinned SHA-256 checksum, and subsequently loaded from that local cache. The model artifact URL, version, and checksum are release metadata; failed download, verification, or loading is a startup failure.

Language detection is part of each inference request. It does not persist a selected language between jobs, expose a user setting, or request Whisper's English-only translation mode.

---

## 3.5 Clipboard

**arboard**

The clipboard component is responsible for copying successful transcription results into the system clipboard.

Clipboard failures shall be reported independently from transcription failures.

---

## 3.6 Concurrency

The initial implementation shall use Rust standard-library concurrency primitives where sufficient:

- `std::thread`;
- `std::sync::mpsc`;
- `Arc`;
- atomic cancellation flags where required.

An async runtime such as Tokio is not required for the initial architecture.

---

# 4. High-Level Architecture

The application consists of a central application controller surrounded by four infrastructure-facing components.

```text
┌──────────────────────────────────────────────┐
│                  Application                 │
│                                              │
│              ┌────────────────┐              │
│ Keyboard ───►│ Event Loop /   │              │
│              │ State Machine  │              │
│              └───────┬────────┘              │
│                      │                       │
│       ┌──────────────┼──────────────┐        │
│       │              │              │        │
│       ▼              ▼              ▼        │
│ ┌──────────┐   ┌────────────┐ ┌───────────┐ │
│ │ Recorder │   │Transcriber │ │ Clipboard │ │
│ │   CPAL   │   │ whisper-rs │ │  arboard  │ │
│ └──────────┘   └────────────┘ └───────────┘ │
│                      │                       │
│                      │ worker events         │
│                      ▼                       │
│              Application Loop               │
│                                              │
│              ┌───────────────┐               │
│              │   Terminal    │               │
│              │   Renderer    │               │
│              └───────────────┘               │
└──────────────────────────────────────────────┘
```

The central application state machine owns the current application mode and decides which actions are valid.

Infrastructure components shall not independently decide application state transitions.

---

# 5. Architectural Components

## 5.1 Application Controller

The application controller is the central coordination component.

Suggested representation:

```rust
struct App {
    mode: AppMode,
    recorder: Recorder,
    transcriber: Transcriber,
    clipboard: Clipboard,
}
```

The exact type ownership may differ during implementation, particularly for worker-managed resources.

Its responsibilities are to:

- maintain current application state;
- receive terminal commands;
- receive internal worker events;
- validate commands according to current state;
- trigger recording actions;
- trigger transcription;
- handle cancellation;
- request clipboard operations;
- request terminal rendering;
- recover from operational errors.

The application controller shall be the authoritative owner of state transitions.

---

# 6. Application State Model

The application shall use an explicit state model.

```rust
enum AppMode {
    Ready,
    Recording,
    Transcribing,
}
```

Additional internal data may be associated with individual states if needed, but the externally visible states remain these three. In particular, `Transcribing` shall retain an internal phase of `Running` or `Cancelling`, the active `TranscriptionId`, and its cancellation flag.

The valid transitions are:

```text
Ready
  │
  │ Ctrl+R
  ▼
Recording
   │
   ├── Esc ─────────────────────► Ready
   │
   │ Ctrl+R
   ▼
Transcribing
   │
   ├── Esc ─────────────────────► Transcribing (Cancelling)
   │                                │
   │                                └── worker stopped ──► Ready
  │
  ├── transcription failure ───► Ready
  │
  └── transcription success ───► Ready
```

Recording errors also return the application to `Ready`.

---

# 7. Event Model

The architecture shall distinguish between:

1. external user events;
2. internal application events.

## 7.1 User Events

Suggested representation:

```rust
enum UserCommand {
    ToggleRecording,
    Cancel,
}
```

Mapping:

```text
Ctrl+R → ToggleRecording
Esc    → Cancel
```

The meaning of `ToggleRecording` depends on the current state.

---

## 7.2 Internal Events

Suggested representation:

```rust
enum AppEvent {
    TranscriptionCompleted { id: TranscriptionId, text: String },
    TranscriptionCancelled { id: TranscriptionId },
    TranscriptionFailed { id: TranscriptionId, message: String },
    RecordingFailed(String),
}
```

Each transcription event must include its operation ID. This makes late worker events harmless after cancellation or recovery.

Internal events are sent back to the application loop through a channel.

Workers shall not directly modify application state.

---

# 8. Main Event Loop

The main thread owns the terminal event loop.

Conceptually:

```text
loop
 │
 ├── receive terminal input
 │
 ├── receive worker events
 │
 ├── apply state transition
 │
 ├── execute resulting actions
 │
 └── render current UI
```

The event loop must never synchronously wait for a long-running transcription operation.

A conceptual implementation is:

```rust
loop {
    process_terminal_events();
    process_worker_events();
    update_application_state();
    render_if_needed();
}
```

Terminal event polling should use a bounded polling interval rather than blocking indefinitely so that worker completion messages can also be processed promptly.

---

# 9. Threading Model

The initial threading model shall remain small.

```text
┌─────────────────────────┐
│       Main Thread       │
│                         │
│ Terminal input          │
│ Application state       │
│ Rendering               │
│ Clipboard operations    │
└────────────┬────────────┘
             │
              │ submit transcription job
             ▼
┌─────────────────────────┐
│ Transcription Worker    │
│                         │
│ owns loaded whisper-rs  │
│ Whisper inference       │
└────────────┬────────────┘
             │
             │ AppEvent
             ▼
        event channel
```

CPAL executes its audio callbacks according to its own audio-stream execution model.

The audio callback shall perform minimal work:

1. receive samples;
2. convert samples if necessary;
3. append them to the current recording buffer.

Heavy processing shall not occur inside the audio callback.

---

# 10. Recorder Component

## 10.1 Responsibilities

The recorder component is responsible for:

- discovering or opening the selected/default microphone;
- starting recording;
- accumulating audio samples;
- stopping recording;
- cancelling recording;
- releasing microphone resources;
- returning completed audio samples.

Conceptual API:

```rust
impl Recorder {
    fn start(&mut self) -> Result<(), RecorderError>;

    fn stop(&mut self) -> Result<RecordedAudio, RecorderError>;

    fn cancel(&mut self);
}
```

---

# 11. Recorded Audio Representation

Audio should remain in memory for the initial implementation.

A possible domain type is:

```rust
struct RecordedAudio {
    samples: Vec<f32>,
    sample_rate: u32,
}
```

Additional metadata may be stored if required.

The recorded audio lifecycle is:

```text
microphone
    ↓
CPAL input stream
    ↓
in-memory sample buffer
    ↓
recording finishes
    ↓
RecordedAudio
    ↓
transcriber
    ↓
discard
```

Cancelled recordings discard the sample buffer immediately.

Successful recordings may also be discarded as soon as transcription completes or is cancelled.

---

# 12. Audio Normalization

The recorder and transcriber may operate with different expected audio formats.

A small normalization step may therefore be required between them.

Responsibilities may include:

- converting integer microphone samples to floating point;
- converting stereo or multichannel input to mono;
- resampling audio to the sample rate expected by the transcription engine.

Conceptually:

```text
Raw microphone samples
        ↓
Audio normalization
        ↓
Mono f32 samples
        ↓
Transcriber
```

This processing should remain an implementation detail rather than introducing a standalone subsystem unless its complexity justifies doing so.

---

# 13. Transcriber Component

## 13.1 Responsibilities

The transcriber component is responsible for:

- owning or accessing the loaded Whisper model;
- accepting normalized recorded audio;
- configuring automatic source-language detection for that recording;
- running transcription;
- returning final text;
- responding to cancellation requests.

Conceptual API:

```rust
impl Transcriber {
    fn transcribe(
        &self,
        audio: RecordedAudio,
        cancel: CancellationToken,
    ) -> Result<String, TranscriptionError>;
}
```

The exact cancellation type may be implemented using an atomic boolean rather than a dedicated abstraction.

---

# 14. Whisper Model Lifecycle

The Whisper model should be loaded once during application startup.

It should not be reloaded for every recording.

Preferred lifecycle:

```text
Application startup
       ↓
Provision and verify model if missing
       ↓
Start model-owning worker
       ↓
Ready
       ↓
Transcription 1
       ↓
Ready
       ↓
Transcription 2
       ↓
...
       ↓
Application shutdown
```

This minimizes latency between recordings.

Failure to initialize the transcription model is considered a startup failure because transcription is a core application capability.

---

# 15. Transcription Worker Lifecycle

The application starts one long-lived transcription worker during startup. The worker owns the loaded Whisper model and accepts at most one job at a time. This avoids requiring the model or its native resources to be shared across arbitrary worker threads.

After recording finishes:

```text
Recording
   │
   │ Ctrl+R
   ▼
stop recorder
   │
   ▼
RecordedAudio
   │
   ▼
set mode = Transcribing (Running)
   │
   ▼
submit job to worker
   │
   ▼
Whisper inference
```

On success:

```text
worker
  │
  ▼
TranscriptionCompleted(text)
  │
  ▼
event channel
  │
  ▼
main event loop
```

The main event loop then:

1. prints the result;
2. copies it to the clipboard;
3. returns to `Ready`.

---

# 16. Transcription Cancellation

Transcription cancellation shall be cooperative.

A cancellation flag is shared between the application controller and transcription worker.

Conceptually:

```text
Main thread                       Worker
    │                               │
    │ create cancellation flag      │
    ├──────────────────────────────►│
    │                               │
    │             Whisper running   │
    │                               │
Esc │                               │
    │ set cancelled = true          │
    ├──────────────────────────────►│
    │                               │
    │                  observe flag │
    │                               │
    │                  abort work   │
    │                               │
    │◄── TranscriptionCancelled ────┤
```

A possible implementation is:

```rust
Arc<AtomicBool>
```

Cancellation should terminate computation when the underlying transcription library allows it.

The architecture shall not rely solely on ignoring a completed result after cancellation if actual cooperative cancellation is available.

On `Esc`, the controller sets the cancellation flag and changes the internal phase to `Cancelling`. It shall render a cancellation status and reject recording commands until it receives the matching worker event. Only then may it transition to `Ready` and accept another transcription job.

---

# 17. Race Conditions Around Cancellation

A transcription may finish at approximately the same time that the user presses `Esc`.

The application controller shall resolve this through application state.

For example:

1. user presses `Esc`;
2. application changes `Transcribing` from `Running` to `Cancelling`;
3. cancellation flag is set;
4. a completion event arrives before the worker observes cancellation.

The controller shall discard a completion event received while the matching operation is cancelling, then return to `Ready` only after the worker has stopped the job.

To make this reliable, transcription operations may be assigned an identifier.

Example:

```rust
struct TranscriptionId(u64);
```

Events can then contain:

```rust
AppEvent::TranscriptionCompleted {
    id: TranscriptionId,
    text: String,
}
```

The application accepts a result only when the identifier corresponds to the currently running transcription. Results for cancelled, completed, or otherwise inactive operations are discarded.

This mechanism is recommended if implementation testing reveals completion/cancellation races.

It does not need to become a more general job-management subsystem.

---

# 18. Clipboard Component

The clipboard component has one responsibility:

```rust
fn copy_text(text: &str) -> Result<(), ClipboardError>;
```

The normal successful flow is:

```text
TranscriptionCompleted(text)
          ↓
render transcription
          ↓
copy_text(text)
          ↓
Ready
```

If clipboard copying fails:

```text
TranscriptionCompleted(text)
          ↓
print transcription
          ↓
clipboard failure
          ↓
print warning
          ↓
Ready
```

A clipboard failure must not change a successful transcription into a transcription failure.

---

# 19. Terminal Component

The terminal component has two responsibilities:

1. terminal input;
2. terminal presentation.

These may remain in the same module because both are small and tightly related.

---

# 20. Terminal Input

Terminal initialization shall enable the mode necessary to receive individual key events without waiting for newline input.

The terminal input layer maps physical keyboard events into application commands.

Example:

```text
KeyEvent
    ↓
Terminal adapter
    ↓
UserCommand
```

Mappings:

```text
Ctrl+R → UserCommand::ToggleRecording
Esc    → UserCommand::Cancel
```

Unsupported keys are ignored.

The application shall not register global operating-system hotkeys.

---

# 21. Terminal Rendering

Rendering depends only on application state and current status information.

Conceptually:

```rust
fn render(view: &AppView) -> Result<(), TerminalError>;
```

A small display model may be used:

```rust
struct AppView {
    mode: AppMode,
    status_message: Option<String>,
}
```

The renderer shall not own business behavior.

The renderer receives logical text, not terminal-specific payload bytes. It shall serialize every logical newline written to the terminal as carriage return plus line feed (`\r\n`), including newlines embedded in status blocks or output text. Raw terminal mode can disable the terminal's normal line-feed-to-carriage-return-plus-line-feed processing, so the renderer shall not depend on that processing to return the cursor to column zero.

This serialization belongs exclusively to terminal presentation. The controller, transcriber, and clipboard boundary exchange canonical transcription text without terminal line-ending conversion.

---

# 22. Rendering by State

## Ready

```text
Ready to record

Ctrl+R  Start recording
```

## Recording

```text
Recording...

Ctrl+R  Finish
Esc     Cancel
```

## Transcribing

```text
Transcribing...

Esc     Cancel
```

Additional symbols, colors, elapsed-time counters, or animations may be implemented without changing the architecture.

---

# 23. Preserving Transcription Output

Successful transcription output should remain part of normal terminal history rather than being erased immediately during subsequent state rendering.

A practical terminal design is therefore:

```text
persistent output area
----------------------
previous transcription
warnings/errors

current status area
-------------------
Recording...
Ctrl+R Finish
Esc Cancel
```

The implementation may accomplish this through simple line rendering rather than a full terminal-widget framework.

Append-only rendering shall preserve left-aligned logical lines across status changes and transcript output. The renderer must apply the terminal newline contract to both status blocks and persistent output, including a transcript containing embedded logical newlines.

---

# 24. Terminal Cleanup

The terminal adapter is responsible for restoring terminal state before application termination.

Cleanup includes, where applicable:

- disabling raw mode;
- restoring cursor visibility;
- restoring terminal modes modified by the application.

Terminal cleanup should also occur after recoverable or unexpected application errors whenever possible.

An RAII-style terminal guard is recommended.

Conceptually:

```rust
struct TerminalGuard;
```

Its destructor restores terminal configuration.

---

# 25. Focus Behavior

No explicit focus-detection subsystem is required.

The application receives keyboard commands through the terminal input stream.

Therefore:

```text
terminal focused
    ↓
terminal receives Ctrl+R
    ↓
application receives Ctrl+R
```

When another application owns keyboard focus:

```text
Ctrl+R
    ↓
other focused application
```

The Terminal Voice Transcriber does not intercept the command globally.

---

# 26. Error Architecture

Errors should be divided by subsystem.

Suggested categories:

```rust
enum AppError {
    Terminal(TerminalError),
    Recorder(RecorderError),
    Transcriber(TranscriptionError),
    Clipboard(ClipboardError),
}
```

Subsystem errors may remain small enums or error structs.

A general-purpose complex error hierarchy is unnecessary.

---

# 27. Recoverable Errors

The following errors are recoverable during runtime:

- failure to start recording;
- recording stream failure;
- transcription failure;
- clipboard failure.

The application should report them and return to a usable state where possible.

Typical recovery:

```text
operation
   ↓
error
   ↓
display error
   ↓
cleanup active resources
   ↓
Ready
```

---

# 28. Startup Errors

Some failures prevent the application from functioning at all and should cause startup to fail with a clear message.

Examples include:

- Whisper model download or integrity verification fails;
- Whisper model cannot be loaded;
- required terminal initialization fails;
- unsupported runtime environment.

Microphone availability may either be validated at startup or when recording begins.

Validating it when recording begins keeps startup simpler and permits devices to change during the application's lifetime.

---

# 29. Shutdown Architecture

The application should define an explicit shutdown path.

Although the initial functional specification does not define a dedicated quit shortcut, application termination may occur through:

- terminal/process termination;
- future explicit quit command;
- unrecoverable application error.

Shutdown responsibilities include:

1. stop active recording;
2. request active transcription cancellation;
3. wait for the worker to release the active job when practical;
4. release audio resources;
5. restore terminal state;
6. release transcription model resources;
7. exit the process.

`Ctrl+C` shall initiate this normal shutdown path.

---

# 30. Suggested Source Structure

The initial implementation should use a flat source structure.

```text
src/
├── main.rs
├── app.rs
├── terminal.rs
├── recorder.rs
├── transcriber.rs
├── clipboard.rs
└── audio.rs
```

`audio.rs` is optional and should exist only if audio normalization warrants its own module.

---

# 31. Module Responsibilities

## `main.rs`

Responsible for:

- application startup;
- dependency initialization;
  - model provisioning and transcription-worker startup;
- terminal initialization;
- channel creation;
- starting the application loop;
- final cleanup;
- process-level error reporting.

It should contain minimal application logic.

---

## `app.rs`

Responsible for:

- `App`;
- `AppMode`;
- user commands;
- internal application events;
- state transitions;
- orchestration between components.

This is the behavioral core of the program.

---

## `terminal.rs`

Responsible for:

- terminal setup;
- terminal restoration;
- keyboard event polling;
- key-to-command mapping;
- status rendering.

---

## `recorder.rs`

Responsible for:

- microphone interaction;
- CPAL stream management;
- recording buffer;
- start;
- stop;
- cancel.

---

## `audio.rs`

Optional.

Responsible for:

- channel conversion;
- sample conversion;
- resampling;
- creation of transcription-ready audio.

---

## `transcriber.rs`

Responsible for:

- Whisper initialization support;
- transcription execution;
- transcription cancellation;
- extraction of final transcription text.

---

## `clipboard.rs`

Responsible for:

- copying plain text to the operating-system clipboard.

---

# 32. Dependency Direction

The application should maintain simple dependencies:

```text
main
 │
 ▼
app
 │
 ├────► recorder
 │
 ├────► transcriber
 │
 ├────► clipboard
 │
 └────► terminal
```

Infrastructure modules should not depend on the application controller.

For example:

```text
recorder ─X─► app
transcriber ─X─► app
clipboard ─X─► app
```

Instead, they return values or emit events that the application controller interprets.

---

# 33. Interfaces and Abstraction Level

The initial implementation does not require dependency-injection frameworks or a large interface hierarchy.

However, small Rust traits may be introduced where they provide concrete testing value.

For example:

```rust
trait Clipboard {
    fn copy(&mut self, text: &str) -> Result<(), ClipboardError>;
}
```

A transcription abstraction may similarly be useful:

```rust
trait SpeechTranscriber {
    fn transcribe(
        &mut self,
        audio: RecordedAudio,
        cancellation: CancellationToken,
    ) -> Result<String, TranscriptionError>;
}
```

Traits should be introduced primarily at external-system boundaries that need substitution in tests.

The architecture should avoid creating a trait for every struct.

---

# 34. State Transition Ownership

All state transitions shall occur through the application controller.

For example:

```text
Terminal
  │
  │ Ctrl+R
  ▼
App
  │
  ├── checks current state
  │
  ├── invokes Recorder
  │
  └── changes state to Recording
```

Not:

```text
Terminal
  │
  └── directly starts recorder

Recorder
  │
  └── independently changes UI
```

This centralization makes valid and invalid transitions explicit.

---

# 35. Recording Sequence

The complete recording-start sequence is:

```text
Ctrl+R
  ↓
terminal maps key
  ↓
ToggleRecording
  ↓
App sees Ready
  ↓
Recorder.start()
  ↓
success
  ↓
AppMode::Recording
  ↓
render Recording
```

If recorder startup fails:

```text
Recorder.start()
  ↓
error
  ↓
display error
  ↓
remain Ready
```

---

# 36. Finish Recording Sequence

```text
Ctrl+R
  ↓
ToggleRecording
  ↓
App sees Recording
  ↓
Recorder.stop()
  ↓
RecordedAudio
  ↓
AppMode::Transcribing
  ↓
render Transcribing
  ↓
submit transcription job to worker
```

---

# 37. Cancel Recording Sequence

```text
Esc
  ↓
Cancel
  ↓
App sees Recording
  ↓
Recorder.cancel()
  ↓
discard recording buffer
  ↓
AppMode::Ready
  ↓
render Ready
```

No transcription worker is created.

---

# 38. Successful Transcription Sequence

```text
Transcription Worker
        │
        ▼
TranscriptionCompleted(text)
        │
        ▼
event channel
        │
        ▼
App
        │
        ├── print transcription
        │
        ├── clipboard.copy(text)
        │
        └── mode = Ready
```

The application then waits for another recording.

---

# 39. Cancel Transcription Sequence

```text
Esc
  ↓
Cancel
  ↓
App sees Transcribing
  ↓
set cancellation flag
  ↓
mode = Ready
  ↓
render Ready
```

The transcription worker cancels the active job cooperatively and remains available for the next job.

Any result belonging to the cancelled operation shall be discarded.

---

# 40. Data Lifecycle

The primary transient data object is recorded audio.

```text
Recording buffer
      │
      │ finish
      ▼
RecordedAudio
      │
      │ transcription
      ▼
Transcription text
      │
      ├── terminal
      └── clipboard
```

No durable persistence is required.

Data disappears when:

- recording is cancelled;
- transcription completes;
- transcription is cancelled;
- the process exits.

The clipboard is the only external destination for successful output.

---

# 41. Persistence

The initial application shall not require:

- database;
- local data store;
- recording history;
- transcription history;
- cache;
- server-side persistence.

The verified Whisper model is a local cached dependency, not application-generated data. It is stored in the Linux user data directory and may be deleted to force a verified re-download.

---

# 42. Configuration

Configuration should remain minimal. The initial model location, artifact version, source URL, and checksum are application release metadata, rather than user configuration. On first run, the application downloads the pinned multilingual `base` model into the Linux user data directory and verifies it before use.

Other parameters use sensible defaults initially where practical:

- the default input microphone;
- automatic language detection;
- mono floating-point audio normalized to Whisper's required sample rate.

Potential future settings include:

- microphone device;
- model selection;
- keyboard shortcuts.

These should not drive premature configuration architecture.

No model-path command-line argument or environment variable is required for the first implementation.

---

# 43. Testing Architecture

Testing shall focus primarily on application-state behavior.

The state machine should be testable without:

- microphone hardware;
- Whisper inference;
- clipboard access;
- an interactive terminal.

---

# 44. State Machine Tests

Core test cases should include:

```text
Ready + Ctrl+R
→ Recording
```

```text
Recording + Ctrl+R
→ Transcribing
```

```text
Recording + Esc
→ Ready
```

```text
Transcribing + Esc
→ Ready
```

```text
Transcribing + success
→ print/copy
→ Ready
```

```text
Transcribing + failure
→ error
→ Ready
```

Invalid commands should preserve state.

---

# 45. Component Tests

## Recorder

Tests should cover logic that does not require physical hardware where practical, including:

- sample conversion;
- buffering;
- cancellation cleanup;
- audio normalization.

Actual microphone integration may require integration/manual testing.

## Transcriber

Tests may use:

- a small known audio fixture;
- mock implementations for application-level tests.

## Clipboard

Application-level tests should use a fake clipboard implementation rather than modifying the developer's real clipboard.

## Terminal

Keyboard mapping can be tested independently:

```text
Ctrl+R → ToggleRecording
Esc    → Cancel
```

---

# 46. Manual Acceptance Testing

Some requirements are inherently integration-oriented and should be validated manually or through platform-specific integration tests.

Examples:

- microphone capture works;
- terminal receives keys while focused;
- terminal does not receive commands while unfocused;
- actual Whisper transcription works;
- clipboard content is correct;
- cancelling an active transcription is responsive;
- terminal state is restored after application termination.

---

# 47. Performance Characteristics

The architecture does not prescribe exact latency targets for the first version.

However:

- keyboard handling must remain responsive during recording;
- keyboard handling must remain responsive during transcription;
- long-running transcription must not execute on the terminal event-loop thread;
- audio callbacks must not perform expensive processing;
- the Whisper model should remain loaded between transcription cycles.

---

# 48. Resource Usage

Audio recordings are held in memory.

Therefore, memory consumption grows with recording duration.

For the initial personal terminal-tool scope, this is acceptable.

If unlimited or very long recording becomes a requirement, the design may later evolve toward:

- bounded buffers;
- temporary audio files;
- chunked transcription;
- streaming processing.

These are outside the initial architecture.

---

# 49. Security and Privacy

The application operates locally.

The initial architecture does not require sending:

- microphone audio;
- transcription text;
- clipboard contents

to remote services.

First-run model provisioning downloads only the pinned model artifact and its required integrity metadata. It does not transmit user audio, transcription text, or clipboard contents.

Microphone capture occurs only while the application is in `Recording` mode.

Recorded audio should be discarded after:

- cancellation;
- successful transcription;
- transcription failure.

No transcription history is persisted by the application.

---

# 50. Cross-Platform Considerations

The initial release supports Linux desktop environments only. Linux behavior may still differ for:

- microphone device enumeration;
- audio formats;
- clipboard implementation;
- terminal capabilities;
- key event representation.

X11 and Wayland clipboard operation require manual acceptance testing. Platform-specific handling shall remain confined to infrastructure components whenever possible.

The state machine and core application behavior should remain platform-independent.

---

# 51. Architecture Decisions

## ADR-01 — Explicit State Machine

**Decision:** Use an explicit application state machine.

**Reason:** Application behavior is naturally state-dependent, and valid keyboard commands differ between `Ready`, `Recording`, and `Transcribing`.

---

## ADR-02 — One Main Application Loop

**Decision:** Keep keyboard processing and state ownership on one main application thread.

**Reason:** This minimizes shared mutable state and race conditions.

---

## ADR-03 — Background Transcription

**Decision:** Execute Whisper transcription on a worker thread.

**Reason:** Transcription can be computationally expensive and must not block `Esc` processing.

---

## ADR-04 — Cooperative Cancellation

**Decision:** Use an explicit cancellation flag for transcription.

**Reason:** The application must allow active transcription to be terminated through `Esc`.

---

## ADR-05 — Audio Kept in Memory

**Decision:** Keep recording samples in memory rather than writing temporary WAV files.

**Reason:** It minimizes filesystem interaction and simplifies the initial workflow.

---

## ADR-06 — No Async Runtime Initially

**Decision:** Use standard Rust threads and channels rather than Tokio.

**Reason:** The initial application has no network or high-volume asynchronous I/O requirements.

---

## ADR-07 — No Full TUI Framework Initially

**Decision:** Use Crossterm directly instead of introducing a full TUI framework.

**Reason:** The interface contains only a small status area, keyboard hints, results, and errors.

---

## ADR-08 — No Global Hotkeys

**Decision:** Keyboard shortcuts exist only inside the active terminal.

**Reason:** This directly satisfies the requirement that commands work only when the terminal has focus.

---

## ADR-09 — Reuse Whisper Model

**Decision:** Load the speech model once and reuse it for the application session.

**Reason:** Repeated model initialization would unnecessarily increase transcription-cycle latency.

---

## ADR-10 — Single Model-Owning Worker

**Decision:** Use one long-lived worker that owns the loaded Whisper model and processes one transcription at a time.

**Reason:** It avoids unsafe or unsupported sharing of native Whisper resources and prevents overlapping inference jobs.

---

## ADR-11 — Cancellation Waits for Worker Release

**Decision:** Keep the application in the `Transcribing` cancellation phase until the active worker reports completion, cancellation, or failure.

**Reason:** A new job must not start while cancellation may still be using the single model owner.

---

## ADR-12 — Verified First-Run Model Download

**Decision:** Download a pinned multilingual Whisper `base` model on first run, cache it in the Linux user data directory, and verify it with a pinned SHA-256 checksum.

**Reason:** This provides a usable default without bundling a large asset or requiring users to locate a model manually.

---

## ADR-13 — Automatic Per-Recording Language Detection

**Decision:** Configure each Whisper transcription request with a null language (`FullParams::set_language(None)`).

**Reason:** In `whisper-rs` 0.16 / whisper.cpp 1.8.3, this detects the source language and continues decoding. `set_detect_language(true)` is detection-only and returns before producing transcript segments. This supports Portuguese and other languages without introducing a language setting or English translation.

---

# 52. Dependencies

The core dependency categories are:

```text
Rust application
│
├── Crossterm
│     terminal input/output
│
├── CPAL
│     microphone capture
│
├── whisper-rs
│     local transcription
│
└── arboard
      clipboard
```

Additional small dependencies may be introduced for audio resampling or error handling if implementation needs justify them.

Dependency additions should remain conservative.

---

# 53. Initial Architecture Boundary

The complete first-version system can be summarized as:

```text
             Terminal keyboard
                    │
                    ▼
             ┌──────────────┐
             │ Application  │
             │ State Machine│
             └──────┬───────┘
                    │
        ┌───────────┼──────────────┐
        │           │              │
        ▼           ▼              ▼
   Microphone    Whisper       Clipboard
      CPAL      whisper-rs      arboard
        │           │
        └──── audio ┘
                    │
                    ▼
              transcription
                    │
                    ▼
                 terminal
```

The fundamental execution loop is:

```text
Ready
  ↓
Record
  ↓
Transcribe
  ↓
Print
  ↓
Copy
  ↓
Ready
```

with cancellation paths:

```text
Recording ── Esc ──► Ready
```

and:

```text
Transcribing ── Esc ──► Transcribing (Cancelling) ── worker stopped ──► Ready
```

This boundary should remain stable unless new product requirements create a concrete need for additional architectural components.
