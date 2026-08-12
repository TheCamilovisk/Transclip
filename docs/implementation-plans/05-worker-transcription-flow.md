# Slice 4: Worker Transcription Flow

## Outcome

A completed recording is transcribed locally by one long-lived background worker that owns the loaded Whisper model. The main terminal loop stays responsive, displays only final successful text, and supports repeated successful cycles without reloading the model. Slice 10 defines the fixed-interface presentation that retains only the latest successful text.

## Prerequisites

- Slices 1-3 are complete.
- D2 confirms compatible whisper-rs APIs; D5 defines the worker protocol.
- Audio produced by Slice 3 conforms to D4.

## Implementation Steps

1. In `transcriber.rs`, define a long-lived worker startup API that receives or creates the already-verified loaded model, then reports startup success/failure before the app becomes Ready.
2. Define a bounded/no-backlog job channel. A job contains `TranscriptionId`, normalized `RecordedAudio`, and its cancellation flag. The worker must accept only one active job.
3. Define an app-event channel carrying exactly one terminal outcome for each accepted job: completed text, cancelled, or failed. Include the job ID in every outcome.
4. Start the worker once during startup and leave it alive across transcription cycles. The worker exclusively owns Whisper native resources; do not share the context with the main thread or spawn a worker per job.
5. Connect Recording + ToggleRecording behavior: stop recorder, obtain audio, allocate next ID and cancellation flag, set `Transcribing(Running)`, render it, then submit the job. Define submission failure as a recoverable transcription failure that returns Ready after cleanup.
6. Run inference only in the worker. Convert a successful final result to plain text according to the chosen whisper-rs API; do not stream partial output.
7. In the controller, accept `TranscriptionCompleted` only if the ID is active and phase is Running. Print a clearly labeled final transcription through the terminal output boundary, retain it in scrollback, then transition to Ready. Clipboard integration is deferred to Slice 5.
8. On worker failure, discard audio/results, display an error, and return Ready without success output.

## Automated Tests

- Controller submits exactly one job after a successful recording stop and enters Transcribing before waiting for a result.
- Main-loop event polling remains testably independent of job completion; no synchronous inference call exists in controller paths.
- Completion for active Running ID prints final text and returns Ready.
- Failure for active ID shows an error and returns Ready without print/copy action.
- Events for inactive or prior IDs are ignored.
- Worker protocol tests prove one job at a time and preserve model-worker ownership using a fake transcriber where native inference is unsuitable.

## Manual Checks

- A spoken recording produces local final text without freezing Esc handling.
- Two or more successful cycles reuse the process and model worker.
- No network request occurs after model provisioning during transcription.

## Acceptance Criteria

- Functional specification sections 5, 6, 7.1, 10.2, 13, 15.3, 18, and 19.4, 19.6-19.9, 19.11-19.12, 19.16 are met except clipboard/cancellation-specific requirements assigned to later slices.
- Architecture sections 3.4-3.6, 8-9, 13-15, 26-27, 38, 40, 47, and 49 are met.
