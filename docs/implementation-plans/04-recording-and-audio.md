# Slice 3: Recording And Audio

## Outcome

`Ctrl+R` starts an in-memory microphone recording, and a second `Ctrl+R` yields normalized, Whisper-ready audio without blocking terminal input. `Esc` stops capture, discards all current samples, and returns to Ready without submitting transcription.

## Prerequisites

- Slice 2 controller and its test seam for recorder operations.
- Resolve D4 before hardcoding target sample rate, conversion behavior, or empty-recording handling.

## Implementation Steps

1. Define `RecordedAudio { samples: Vec<f32>, sample_rate: u32 }` and keep its lifecycle limited to the active recording/transcription cycle.
2. Implement `Recorder::start`, `stop`, and `cancel` using CPAL's default input device unless a later requirement adds selection. Open the stream and retain only resources needed for the active recording.
3. Implement a minimal callback: convert incoming samples as required, append them to synchronized active buffer, and surface stream errors through the controller event channel. Do not normalize/resample or perform inference in the callback.
4. Define synchronization that prevents callbacks from writing while `stop` extracts the buffer or `cancel` discards it. Make repeated stop/cancel/error cleanup safe.
5. On start failure, show an error and remain Ready. On capture error, stop/release the stream, discard audio, show an error, and return Ready.
6. Normalize completed audio outside the callback: supported numeric formats to `f32`, all channels to mono, and resample to the D4 target. Keep this in `recorder.rs` unless it makes `audio.rs` materially clearer.
7. Attach recorder outcomes to Slice 2 actions. Only a successful `stop` may create a transcription job; `cancel` must never do so.
8. Decide and implement the D4 behavior for empty, silent, or too-short recordings before worker submission.

## Automated Tests

- Buffer accumulation and transfer at stop.
- Cancel drops all buffered audio and no submission action occurs.
- Integer/float sample conversion, mono handling, multi-channel downmix, and resampling fixtures.
- Start and stream failure return the controller to Ready with no residual stream/buffer.
- Repeated cleanup calls do not panic or duplicate submission.

## Manual Checks

- Default microphone starts/stops correctly on supported Linux desktops.
- Ctrl+R and Esc remain responsive throughout recording.
- Device access failure and device removal produce a recoverable Ready state.

## Acceptance Criteria

- Functional specification sections 4, 7.2, 10, 12, 15.1-15.2, and 19.2-19.5, 19.14, 19.16 are met.
- Architecture sections 9-12, 27, 35-37, 40, and 45 are met.
