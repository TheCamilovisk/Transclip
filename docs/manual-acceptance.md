# Transclip Manual Acceptance Checklist

Manual acceptance for the initial release (functional spec section 19,
architecture sections 24, 27-29, 40, 43-50, and the dependency/security
boundaries). This checklist is maintained by slice 7
(`docs/implementation-plans/08-operational-hardening.md`, plan step 6); each
row records the expected terminal output and the hardware/display
prerequisites it needs.

## Prerequisites

Hardware and environment:

- Linux x86_64 desktop with a working **microphone** (e.g. a webcam/USB mic or
  an analog input). Every audio-input path used here is real; there is no
  simulated input.
- A **display session**: X11 (`DISPLAY` set) or Wayland (`WAYLAND_DISPLAY`
  set) with a clipboard service running (e.g. a clipboard manager or
  `wl-clipboard` for Wayland). Without either, the clipboard scenario below
  becomes the headless-warning scenario.
- `script` (util-linux) for pseudo-TTY runs, `curl` for integrity spot-checks,
  and a terminal emulator capable of raw-mode key events.
- Network access for the first-run model download (only the pinned
  `ggml-base.bin` artifact is downloaded; see `src/transcriber.rs`).

Every manual run must isolate the app's data directory so the real cache is
never touched or altered:

```bash
export XDG_DATA_HOME="$(mktemp -d /tmp/transclip-xdg.XXXXXX)"
```

Clean up the temp directory after the session.

## How to run a scenario

Interactive behavior must be exercised through a pseudo-TTY, never a bare
pipe:

```bash
(sleep 3; printf '\003') | TERM=xterm script -qec "./target/debug/transclip" /dev/null
```

- The `sleep` gives provisioning/startup time to finish so the first key is
  delivered after raw mode is active (sending a key too early delivers SIGINT
  instead of a key event).
- `Ctrl+C` (`\003`) is the exit key; a normal exit returns code 0.
- Terminal restoration is visible in the capture: cursor hide `^[[?25l` on
  entry, cursor show `^[[?25h` on exit.

## Expected terminal output

Status blocks (append-only; previous statuses and results stay in the
terminal history):

```text
Ready to record

Ctrl+R  Start recording
```

```text
Recording...

Ctrl+R  Finish
Esc     Cancel
```

```text
Transcribing...

Esc     Cancel
```

```text
Cancelling transcription...

Esc     Cancel
```

Successful cycle (the printed text must equal the clipboard content):

```text
Transcription:

This is the text that was recorded.

Copied to clipboard.

Ready to record

Ctrl+R  Start recording
```

Recoverable errors keep the app usable:

```text
Error: unable to start recording: <reason>

Ready to record
```

```text
Error: recording failed: <reason>

Ready to record
```

```text
Error: transcription failed: <reason>

Ready to record
```

Clipboard failure keeps the successful transcription (functional spec 15.4):

```text
Transcription:

This is the text that was recorded.

Warning: unable to copy transcription to clipboard: <reason>

Ready to record
```

Startup failure (model provisioning/load or terminal init) prints a clear
`error:` line on stderr and exits non-zero before any interactive state.

## Acceptance matrix

| # | Scenario | Prerequisites | Result |
| --- | --- | --- | --- |
| 1 | First run | Network; isolated `XDG_DATA_HOME` (cold cache) | Downloads/verifies/loads pinned model, then shows `Ready to record`. No `.part` file remains. |
| 2 | Model provisioning failure | Isolated `XDG_DATA_HOME`; force failure (e.g. unwritable cache dir or offline) | Clear error on stderr, non-zero exit, no recording possible. |
| 3 | Record and finish | Working microphone; display session | `Recording...` feedback; final local text printed; clipboard content matches printed text byte-for-byte; `Ready to record`. |
| 4 | Cancel recording | Working microphone | `Esc` during `Recording...` returns to `Ready to record` silently; no worker job, no text, no clipboard write. |
| 5 | Cancel transcription | Working microphone; display session | `Esc` during `Transcribing...` renders `Cancelling transcription...` immediately; returns to `Ready to record` only after the worker acknowledgement; no text/clipboard write. |
| 6 | Microphone failure | Start with no default input device (or force failure) | Error line, app remains/returns usable `Ready to record`. |
| 7 | Transcription failure | Any capture (silent audio gives empty text, not failure) | Error line, no clipboard write, usable `Ready to record`. |
| 8 | Clipboard failure | No `DISPLAY`/`WAYLAND_DISPLAY` (headless) | Printed text plus `Warning: unable to copy...`, usable `Ready to record`. |
| 9 | Multiple cycles | Working microphone; display session | Several record/transcribe cycles without restart or model reload; one job at a time; no stale text between cycles. |
| 10 | Focus behavior | Terminal emulator | `Ctrl+R`/`Esc` work only while the terminal is focused; typing in another window never triggers Transclip (no global interception). |
| 11 | X11 and Wayland | One X11 session and one Wayland session | Clipboard behavior validated on both session types (X11 through arboard/x11rb; Wayland through `wl-clipboard-rs`). |
| 12 | Ctrl+C / error exit | Any state (Ready, Recording, Transcribing) | Recorder/worker cleanup follows the bounded shutdown policy; terminal restored (cursor shown, raw mode off); exit code 0. |

## Record-keeping

Run each scenario with the isolated `XDG_DATA_HOME`, capture the pty log, and
note the result (pass / fail + evidence) next to the scenario. Environment
limitations count as evidence when the failing path is covered by the
automated suite instead (e.g. this test machine's audio-input limitations are
documented in `docs/implementation-plans/01-decision-register.md` slice 4/6
notes; the audio→worker→text path is covered by worker-protocol tests with a
fake transcriber).

## Out of scope

No settings, global shortcuts, GUI, cloud services, stored audio/history,
partial results, concurrent recordings, configurable keys, VAD, or general
job framework are part of this acceptance.
