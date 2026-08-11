# Slice 8: Transcript Normalization And Terminal Line Endings

## Outcome

Successful transcription has one canonical plain-text form: decoder or timestamp segments are joined as normalized prose rather than displayed or copied as arbitrary lines. Terminal output remains left-aligned while raw mode is active, and terminal CRLF serialization never changes the text copied to the clipboard.

## Prerequisites

- Slices 1-7 are implemented and their automated tests pass.
- Terminal operational policy D6 remains in effect: rendering is append-only and terminal I/O failures propagate through `TerminalError`.
- Functional specification sections 6, 11, 13, 14, and 19.18 and architecture specification sections 3.4, 21, and 23 define the required contracts.

## Implementation Steps

1. In `src/transcriber.rs`, make final-text construction a small pure normalization step at the Whisper segment boundary:
   - obtain each decoded segment as text;
   - split its whitespace into nonempty text tokens;
   - join all tokens from all segments with one ASCII space;
   - return that result as the completed transcription.
   Do not expose Whisper decoder or timestamp segment boundaries as newlines, and do not add character-count wrapping.
2. Keep the resulting canonical `String` unchanged through `AppEvent::TranscriptionCompleted`, controller output construction, and `Clipboard::copy_text`. Do not normalize or convert clipboard text in `app.rs` or `clipboard.rs`.
3. In `src/terminal.rs`, make terminal serialization explicitly raw-mode-safe. Every logical line feed in a status block or output line, including an embedded line feed, must be written as `\r\n`. The renderer's appended line terminator must use the same convention.
4. Use the smallest testable writer seam necessary to verify terminal bytes without a TTY, such as a private helper accepting `impl Write`. Keep `TerminalRenderer` responsible only for terminal I/O; do not move rendering behavior into the controller or add a terminal framework.
5. Do not alter recording, worker lifecycle, cancellation, clipboard-service, or append-only history behavior delivered by prior slices.

## Automated Tests

- Normalization trims segment-boundary whitespace, discards empty segments, collapses whitespace within and across segments, and joins remaining tokens with one space.
- Normalization produces no newline merely because Whisper returned multiple segments and preserves ordinary punctuation as text.
- A multiline status block is serialized with `\r\n` at every logical line ending, with no bare line feed bytes.
- A persistent output item containing embedded logical newlines is serialized with `\r\n` at every logical line ending, including its renderer-appended terminator.
- A completed canonical transcript reaches the fake clipboard byte-for-byte; terminal CRLF serialization is never applied to the clipboard payload.
- Existing controller, worker, cancellation, and clipboard-failure tests continue to pass without a microphone, Whisper model, graphical clipboard, or interactive terminal.

## Manual Checks

- Run a recording whose Whisper result has multiple decoder segments. Confirm the result is one normalized text flow rather than regularly inserted segment lines.
- Read the system clipboard after the successful result and confirm it exactly matches the normalized transcription, with no decoder-segment newlines or terminal CR bytes.
- Confirm multiline status/result output and the following Ready block start at the left margin while raw mode is active. Natural visual wrapping at the terminal width is acceptable; application-inserted progressive indentation is not.

## Acceptance Criteria

- Functional specification sections 6, 11, 13, 14, and 19.18 are satisfied.
- Architecture specification sections 3.4, 19, 21, and 23 are satisfied.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass.
- No out-of-scope formatting controls, terminal framework, transcript persistence, or clipboard behavior changes are added.
