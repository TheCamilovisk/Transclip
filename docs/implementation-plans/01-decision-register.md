# Decision Register And Implementation Gates

These decisions are absent or contradictory in the specifications. Resolve and record each decision before implementing the affected slice. Do not silently select placeholder values in production code.

## Resolved Interpretations

| ID | Decision | Rationale |
| --- | --- | --- |
| R1 | `Esc` during transcription enters `Transcribing(Cancelling)` until the matching worker outcome arrives. | This is required by functional spec sections 5 and 16 and architecture sections 6, 16, 17, and ADR-11. It supersedes the contradictory immediate-Ready diagram in architecture section 39. |
| R2 | Every transcription gets a monotonically increasing `TranscriptionId`; every terminal worker event includes it. | Architecture section 7 says IDs are mandatory, and they prevent stale events and cancellation races. |
| R3 | A completion received after cancellation was requested is discarded, even if inference finished first. | Cancellation has precedence once the controller processes `Esc`; no text is printed or copied during the cancelling phase. |
| R4 | Clipboard outcome is separate from transcription outcome. | A successful transcription remains printed and successful if copying fails. |

## Blocking Decisions

| ID | Required decision | Affected slices | Completion evidence |
| --- | --- | --- | --- |
| D1 | Pinned model source: compatible multilingual Whisper `base` artifact URL, version/revision, filename, SHA-256, cache directory, cache validation frequency, partial-download cleanup, and atomic replacement strategy. | 1+ | Constants and provenance documented; checksum test uses known fixture. |
| D2 | Exact `whisper-rs` version and verified cooperative-cancellation API. Define its callback/abort wiring, expected cancellation latency, and fallback if native interruption is unavailable. | 1, 4, 6, 7 | Minimal spike or vendor documentation proves the selected API; cancellation integration test is possible. |
| D3 | Linux support baseline and native dependency/packaging policy for CPAL, arboard, and whisper.cpp. | 1, 7 | Supported distributions/session prerequisites and failure diagnostics documented. |
| D4 | Audio contract: target Whisper sample rate, supported CPAL sample formats/channel layouts, resampler, silence/empty-recording behavior, and stop/flush failure behavior. | 3+ | Unit fixtures establish conversion/downmix/resampling behavior. |
| D5 | Worker lifecycle: bounded job protocol, startup handshake, submission failure, panic policy, shutdown signal/acknowledgement, join timeout, and repeated Ctrl+C behavior. | 4, 6, 7 | Worker protocol tests cover each terminal outcome. |
| D6 | Terminal operational policy: append/redraw approach, resize handling, runtime I/O failure, raw-mode Ctrl+C behavior, and shutdown wait policy. | 1, 2, 7 | Manual test checklist covers normal and failure restoration. |

## Non-Goals

Do not add settings, global shortcuts, GUI, cloud services, stored audio/history, partial results, concurrent recordings, configurable keys, VAD, or a general job framework. These are outside the initial release.
