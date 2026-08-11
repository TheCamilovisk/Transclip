# Transclip

Transclip is a Linux terminal application for recording speech, transcribing it
locally with Whisper, printing the result, and copying it to the system
clipboard.

Keyboard input is handled only by the focused terminal. Transclip does not
install global shortcuts, upload recordings, or send transcription text over
the network.

## Requirements

- Linux x86_64 desktop
- Rust 1.85 or newer
- CMake 3.5 or newer
- A C/C++ compiler and libclang (required to build `whisper-rs`)
- A working microphone
- An X11 or Wayland desktop session for clipboard support
- Network access on the first run to download the Whisper model

## Build and Run

```bash
cargo build --release
cargo run --release
```

On its first successful startup, Transclip downloads the pinned multilingual
Whisper `base` model, verifies its SHA-256 checksum, and stores it at:

```text
$XDG_DATA_HOME/transclip/models/ggml-base.bin
```

When `XDG_DATA_HOME` is unset, the default location is:

```text
~/.local/share/transclip/models/ggml-base.bin
```

For an isolated manual run that does not use your normal model cache:

```bash
export XDG_DATA_HOME="$(mktemp -d /tmp/transclip-xdg.XXXXXX)"
cargo run --release
```

## Controls

| Key | Ready | Recording | Transcribing |
| --- | --- | --- | --- |
| `Ctrl+R` | Start recording | Finish recording and transcribe | No action |
| `Esc` | No action | Cancel recording | Cancel transcription |
| `Ctrl+C` | Exit | Exit | Exit |

After a successful transcription, its whitespace-normalized plain text is
printed in the terminal and copied to the clipboard. The app then returns to
the ready state for another recording.

## Privacy and Storage

Audio is held in memory only for the active recording/transcription cycle. No
audio or transcription history is persisted. Network access is limited to
first-run model provisioning; transcription runs locally through `whisper-rs`
and `whisper.cpp`.

## Development

Run the project quality gates in order:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The automated tests use fakes at hardware and terminal boundaries, so they do
not require a microphone, clipboard, Whisper model, or interactive terminal.

## Scope

The initial release intentionally excludes a GUI, global hotkeys, cloud
services, stored audio or transcription history, streaming results,
configurable shortcuts, and voice activity detection.

See the [functional specification](docs/specifications/functional-spec.md),
[architecture specification](docs/specifications/architecture-spec.md), and
[manual acceptance checklist](docs/manual-acceptance.md) for full behavior and
operational details.
