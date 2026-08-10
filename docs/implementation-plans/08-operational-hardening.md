# Slice 7: Operational Hardening And Acceptance

## Outcome

The complete application recovers from specified runtime failures, performs orderly Ctrl+C shutdown, restores the terminal, and demonstrates the end-to-end acceptance criteria over repeated cycles on supported Linux environments.

## Prerequisites

- Slices 1-6 are implemented and their automated tests pass.
- D3, D5, and D6 are resolved and documented.

## Implementation Steps

1. Audit every external boundary for the required recovery behavior:
   - microphone cannot start: display error, remain/return Ready;
   - stream/capture failure: stop/release and discard buffer, display error, Ready;
   - transcription failure or failed submission: discard data, display error, Ready;
   - clipboard failure: print result and warning, Ready;
   - startup provisioning/load/terminal failure: clear error and exit.
2. Ensure all active data and resources are released after success, cancellation, failure, and process exit: recorder stream, audio buffer, pending job references, worker channel/model resources, and terminal modes.
3. Implement Ctrl+C as normal shutdown, using the key/signal strategy established by D6. It must stop recording or request worker cancellation, wait/join only according to the bounded D5/D6 policy, then restore terminal state even when work cannot finish gracefully.
4. Handle worker-channel closure, worker panic/error policy, and terminal render/input failure as D5/D6 require. Do not claim recoverability where process integrity cannot be guaranteed; restore terminal state before reporting fatal errors.
5. Add test coverage for repeated mixed cycles: success -> cancel recording -> success -> cancel transcription -> recoverable failure -> success. Verify no output or clipboard leakage across operations.
6. Create or update a manual acceptance checklist in project documentation if the implementation does not already have one. It must identify hardware/display prerequisites and expected terminal output.

## Automated Verification

Run after implementation:

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Required test cases include all state transitions, recoverable errors, output/clipboard ordering, cancellation races, stale events, terminal key mapping, audio normalization, and fake-boundary isolation. Tests must not require real microphone hardware, a model download, a graphical clipboard, or an interactive terminal.

## Manual Acceptance Matrix

| Scenario | Expected result |
| --- | --- |
| First run | Downloads/verifies/loads pinned model, then shows Ready. |
| Model provisioning failure | Clear error, non-zero exit, no recording. |
| Record and finish | Recording feedback, final local text, matching clipboard content, Ready. |
| Cancel recording | Audio discarded, no worker job/text/clipboard, Ready. |
| Cancel transcription | Cancelling feedback, no text/clipboard, Ready only after worker acknowledgement. |
| Microphone failure | Error and usable Ready state. |
| Transcription failure | Error, no clipboard, usable Ready state. |
| Clipboard failure | Printed text plus warning, usable Ready state. |
| Multiple cycles | No restart/model reload required; no concurrent model jobs. |
| Focus behavior | Commands work only when delivered to the terminal; no global interception. |
| X11 and Wayland | Clipboard behavior validated on both supported session types. |
| Ctrl+C/error exit | Recorder/worker cleanup follows policy and terminal is restored. |

## Acceptance Criteria

- Every item in functional specification section 19 passes.
- Architecture sections 24, 27-29, 40, 43-50, and the dependency/security boundaries are validated.
- No out-of-scope capability is added as part of hardening.
