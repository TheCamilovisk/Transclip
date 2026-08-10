# Slice 6: Transcription Cancellation

## Outcome

Esc requests real cooperative cancellation of an active transcription. The application visibly remains in `Transcribing(Cancelling)` until the matching worker has stopped and released model resources. It never prints or copies a cancelled result and cannot start a new cycle during cancellation.

## Prerequisites

- Slice 4 worker protocol and Slice 5 guarded clipboard path exist.
- D2 proves the selected whisper-rs cancellation mechanism; D5 defines terminal worker outcomes and shutdown behavior.
- Apply R1-R3 from the decision register exactly.

## Implementation Steps

1. Verify the worker uses a per-job `Arc<AtomicBool>` cancellation flag and wires it into the actual whisper-rs/whisper.cpp cancellation callback or supported abort API. Polling the flag only outside a non-interruptible inference call is insufficient unless D2 explicitly establishes that limitation and its accepted fallback.
2. On `Esc` while phase is Running, atomically request cancellation, transition to Cancelling, and immediately render `Cancelling transcription...`. Further Esc presses are idempotent.
3. While Cancelling, ignore `ToggleRecording` and all completion text for the active ID. Do not transition to Ready merely because cancellation was requested.
4. When the matching worker event confirms cancellation, failure, or completion after the cancellation request, discard any text, clean job data, render Ready, and allow the next cycle. Completion after cancellation is not a success.
5. Keep stale ID filtering active for all terminal outcomes. A late event must never alter a current recording/transcription or invoke clipboard actions.
6. Ensure worker emits its acknowledgement only after it has stopped the active job and released it sufficiently for the single model owner to accept another job.
7. Document observed cancellation latency and fallback limits from D2 in developer-facing validation notes.

## Automated Tests

- Running + Cancel sets the flag and enters Cancelling, not Ready.
- Cancelling + ToggleRecording cannot start recording or submit work.
- Cancelling + matching cancelled event returns Ready without print/copy.
- Cancelling + matching completed event discards text and returns Ready without print/copy.
- Completion that arrives before Esc is processed remains success; completion received after controller entered Cancelling is discarded.
- Stale completed/cancelled/failed events are ignored.
- Repeated Cancel does not send duplicate cancel requests or change state incorrectly.

## Manual Checks

- Start a sufficiently long transcription, press Esc, and verify status changes promptly while inference cleanup is pending.
- Verify no cancelled text appears in terminal or clipboard.
- Verify Ctrl+R during cancellation does nothing; after acknowledgement, it starts the next recording normally.

## Acceptance Criteria

- Functional specification sections 5, 7.3, 8, 10.2, 15.3, 16-17, and 19.7-19.8, 19.15 are satisfied.
- Architecture sections 6-7, 15-17, 29, 39 as corrected by R1, 44-45, and ADR-04/ADR-10/ADR-11 are satisfied.
