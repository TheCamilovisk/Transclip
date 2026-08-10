# Slice 2: Terminal State Machine

## Outcome

The controller is a deterministic, hardware-free state machine driven by focused-terminal commands and worker events. It renders valid commands for Ready, Recording, Transcribing, and the Transcribing cancellation phase. Infrastructure cannot transition state directly.

## Prerequisites

- Slice 1 startup and terminal lifecycle shell exists.
- Apply resolved interpretations R1-R4.

## Implementation Steps

1. In `app.rs`, define the public modes `Ready`, `Recording`, and `Transcribing`; represent `Running` versus `Cancelling`, active ID, and cancellation flag as Transcribing-associated data.
2. Define `UserCommand`, `AppEvent`, `TranscriptionId`, display/status data, and small action outputs or boundary calls that permit unit testing with fakes. Keep state transition logic in one place.
3. Implement the complete command matrix:
   - Ready + ToggleRecording requests recorder start.
   - Recording + ToggleRecording requests stop and transcription submission.
   - Recording + Cancel requests recorder cancellation and discards audio.
   - Transcribing + Cancel changes only Running to Cancelling and signals the cancellation flag.
   - Ready + Cancel and Transcribing + ToggleRecording leave state unchanged.
4. Make terminal input map Crossterm Ctrl+R to `ToggleRecording`, Esc to `Cancel`, and ignore all other events. Do not install OS-level key hooks.
5. Poll terminal input with a bounded timeout, drain worker events promptly, apply transitions, then render only when view data changes. Never block on inference.
6. Design rendering around persistent terminal history plus current status. It must show distinct Ready, Recording, Transcribing, and `Cancelling transcription...` status while treating the latter as a Transcribing substate.
7. Define how state actions report errors and output lines without giving terminal rendering ownership of business behavior.

## Automated Tests

- Command matrix covers every valid and ignored state/command pair.
- Ready start failure remains Ready and surfaces an error action.
- Recording cancellation invokes discard/no-submit behavior.
- Ctrl+R during Transcribing has no effect.
- Key mapping covers Ctrl+R, Esc, modifiers/key kinds selected by D6, and unsupported keys.
- App tests use fake recorder, worker sender, clipboard, and output sink; no real terminal, microphone, or Whisper dependency.

## Acceptance Criteria

- Functional specification sections 3, 4, 8-11, 16-17, and 19.2-19.3, 19.6-19.8, 19.13 are covered at the controller level.
- Architecture sections 5-9, 19-23, 32-34, and 43-45 are respected.
- The following slices attach real components to the established controller boundaries; they must not duplicate transition logic.
