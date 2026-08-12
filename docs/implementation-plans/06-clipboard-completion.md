# Slice 5: Clipboard Completion

## Outcome

Every accepted final transcription is printed and then copied to the Linux system clipboard. Clipboard failure produces a visible warning but never discards or reclassifies the successful transcription. Cancelled and failed operations never write clipboard content.

## Prerequisites

- Slice 4 successfully accepts final completion events and produces terminal output.
- D3 records supported Linux display-session prerequisites and expected arboard behavior.

## Implementation Steps

1. Add the smallest practical clipboard boundary in `clipboard.rs`, such as `copy_text(&str) -> Result<(), ClipboardError>`. Use a fake implementation in controller tests.
2. Construct/use the real arboard implementation on the main thread, consistent with the architecture. Do not move application-state logic into the clipboard module.
3. On active Running completion: print the exact final text first, attempt `copy_text` with the same text, emit `Copied to clipboard.` on success, or an explicit warning on failure, then return Ready.
4. Preserve the successful canonical text in application-owned display state after a later recording starts. Slice 10 supersedes this slice's original terminal-history behavior: a later successful completion replaces the displayed text in the fixed interface.
5. Keep clipboard invocation exclusively on this accepted completion path. Failed, stale, cancelling, and cancelled outcomes must have no copy action.
6. Surface backend errors without exposing sensitive transcription content unnecessarily in diagnostics.

## Automated Tests

- Successful completion invokes fake clipboard once with byte-for-byte printed text and returns Ready.
- Clipboard failure still emits the successful transcription, then a warning, and returns Ready.
- Transcription failure, recording cancellation, transcription cancellation, stale events, and ignored commands never invoke fake clipboard.
- Test the controller without changing the developer clipboard.

## Manual Checks

- Verify copied text equals terminal result on X11.
- Verify copied text equals terminal result on Wayland.
- Verify a missing/headless clipboard service displays a warning while preserving terminal output and future recording usability.

## Acceptance Criteria

- Functional specification sections 6, 14, 15.4, and 19.9-19.11, 19.17 are satisfied.
- Architecture sections 3.5, 18, 38, 45, and 50 are satisfied.
