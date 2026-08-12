# Slice 12: State Visual Feedback

## Outcome

The fixed terminal interface shows the current application state with a scoped emoji and color: `🟢` light green when Ready, `🔴` light red when Recording, and `⚙️` neutral/default when Transcribing or Cancelling. Only the status line is styled.

## Prerequisites

- Slice 11 is implemented.
- The Slice 12 D8 resolution in `01-decision-register.md` is applied.
- Functional specification section 11 and architecture specification section 21 define the display and styling boundaries.

## Implementation Steps

1. Extend the terminal presentation model with enough state-derived metadata for the renderer to select the status emoji and color without putting terminal escapes in application data.
2. Render the status line with the required prefix and color, then emit a terminal color reset before every following logical line.
3. Keep the status wording and all state transitions unchanged. Render Transcribing and Cancelling with the neutral `⚙️` style.
4. Leave the latest transcription, notices, command hints, and Whisper/GGML logging path unstyled and in the terminal default color.
5. Preserve the existing in-place redraw, CRLF serialization, resize redraw, cursor cleanup, and clipboard byte-identity behavior.

## Automated Tests

- Ready status output contains `🟢`, uses the light-green escape sequence, and resets before the next line.
- Recording status output contains `🔴`, uses the light-red escape sequence, and resets before transcription, notice, and command content.
- Transcribing and Cancelling output contains `⚙️` with no non-default state color applied.
- A view containing a latest transcription and notice verifies that neither content nor command hints occur between a state color set and its reset.
- Existing controller tests still establish plain canonical transcription text and clipboard byte identity; existing renderer tests still establish one CRLF-safe in-place redraw.

## Manual Checks

- In a color-capable terminal, verify Ready, Recording, Transcribing, and Cancelling transitions have the required emoji and color.
- Complete a transcription and verify the displayed result remains neutral while the Ready status is light green.
- Trigger a recoverable error and resize redraw; verify notices and command hints remain neutral and no prior status color bleeds into them.
- Start with a cached model and complete a cycle; verify Whisper/GGML diagnostics remain absent and never use application-state colors.

## Acceptance Criteria

- Ready status is `🟢` and light green; Recording status is `🔴` and light red.
- Transcribing and Cancelling status is `⚙️` in the terminal default color.
- Only the status line changes emoji and color; transcription, notices, commands, and Whisper/GGML output remain neutral.
- Terminal styling never changes canonical transcription or clipboard text.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass.
