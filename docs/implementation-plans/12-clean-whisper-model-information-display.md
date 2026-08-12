# Slice 11: Clean Whisper Model Information Display

## Outcome

Whisper and GGML model-loading diagnostics never appear in the application's terminal output. After successful startup, the fixed Ready interface is the first normal display output.

## Prerequisites

- Slice 10 is implemented.
- The Slice 11 logging-hook resolution in `01-decision-register.md` is applied.
- Functional specification section 11 and architecture specification section 21 define the display boundary.

## Implementation Steps

1. Add a transcriber startup helper that installs the `whisper-rs` logging hook.
2. Invoke that helper before the first native Whisper operation, specifically before loading the verified model.
3. Keep logging configuration outside the terminal renderer and application controller. Do not change model provisioning, worker ownership, or fixed-view rendering.

## Automated Tests

- Existing startup failure tests continue to pass after logging is configured before model loading.
- Existing fixed-renderer tests continue to establish that the terminal UI is the only application presentation path.

## Manual Checks

- Start with a valid cached model in an isolated `XDG_DATA_HOME` and confirm no Whisper or GGML model information precedes the Ready interface.
- Complete a recording and confirm no native diagnostics appear before, during, or after the fixed interface redraws.

## Acceptance Criteria

- Native Whisper and GGML diagnostics do not appear in normal terminal output.
- The fixed Ready interface is the first normal display after successful startup.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass.
