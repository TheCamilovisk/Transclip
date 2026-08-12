# Slice 10: Fixed Terminal UI And Latest Transcription

## Outcome

The application presents one bounded terminal interface for its entire session. Status and current keyboard commands update in place, and only the latest successful canonical transcription is retained and displayed. A later success replaces that text instead of appending output or duplicating interface blocks.

## Prerequisites

- Slices 1-9 are implemented and their automated tests pass.
- Apply the fixed-terminal-UI resolution D7 in `01-decision-register.md`; it supersedes only D6's append-only rendering policy.
- Functional specification sections 6, 11, 15, and 19.20 and architecture specification sections 19, 21, and 23 define the required contract.

## Implementation Steps

1. Replace the append-only renderer boundary with a complete display-view boundary. Keep `app.rs` responsible for display data and `terminal.rs` responsible for terminal-specific cursor, clear, and write operations.
2. Add application-owned display state for `latest_transcription: Option<String>` and `notice: Option<String>`. Keep this state separate from `AppMode` so a new mode does not erase the latest successful transcription.
3. On `TranscriptionCompleted` while Running, replace `latest_transcription` with the canonical completed text, copy that exact text to the clipboard, and replace `notice` with either the copy-success confirmation or clipboard warning. Do not let cancellation, transcription failure, recording failure, or recording cancellation replace or clear `latest_transcription`.
4. On recoverable errors, replace only `notice`. A subsequent state transition or successful completion may replace the notice, but no normal flow may append notices or prior results to terminal history.
5. In `terminal.rs`, render the full current view in place using the smallest direct Crossterm cursor/screen operations needed. Clear stale content from the prior view, write status, optional latest transcription, optional notice, and the current commands, then flush. Continue serializing all logical line endings as `\r\n` in raw mode.
6. Deliver terminal resize events to the render loop as redraw requests. Redraw the current view without changing application state, latest transcription, notice, recorder state, or worker state.
7. Preserve existing bounded input polling, terminal-local key mapping, cancellation behavior, worker protocol, clipboard behavior, and terminal error propagation. `TerminalGuard` must continue restoring raw mode and cursor visibility on every exit path.

## Automated Tests

- The initial view contains Ready status and its command hint, with no transcript or notice.
- Status transitions update the fixed view without clearing the prior successful transcription.
- Two successful completions retain only the second text in the display view while both successful texts are independently copied at completion time.
- Recording/transcription cancellation and recoverable errors retain the prior successful transcription and replace at most the transient notice.
- Renderer tests assert one in-place redraw operation clears stale view content, emits CRLF for every logical newline, and does not append a second interface block.
- A resize event requests a redraw using unchanged display data and no state transition.
- Existing state-machine, cancellation, worker, normalization, language-detection, and clipboard byte-identity tests continue to run with no microphone, Whisper model, graphical clipboard, or interactive terminal.

## Manual Checks

- Start the application and confirm exactly one Ready interface is visible, including the `Ctrl+R` command hint.
- Complete a first recording and confirm the fixed interface displays its transcription and copy result without an additional Ready/status block below it.
- Complete a second recording and confirm its transcription replaces the first in the same area; no previous transcription or duplicated command hints remain visible.
- While a prior transcription is visible, start/cancel a recording, cancel transcription, and trigger a recoverable error. Confirm the prior successful transcription remains visible and only the current status/notice changes.
- Resize the terminal during Ready, Recording, and after a successful transcription. Confirm the current complete interface redraws without changing application behavior or losing the latest transcript.
- Confirm a completed transcription copied from the clipboard exactly matches the displayed canonical text and has no terminal-introduced carriage returns.

## Acceptance Criteria

- Functional specification sections 6, 11, 15, and 19.20 are satisfied.
- Architecture specification sections 19, 21, and 23 are satisfied.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass.
- No full terminal-widget framework, transcript persistence, scrolling history, global shortcut, or change to audio, worker, cancellation, or clipboard boundaries is added.
