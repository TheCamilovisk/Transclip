# Transclip Agent Guide

## Build and verify

- Rust 1.85+ on Linux x86_64 is required. `whisper-rs` builds whisper.cpp from source; install `cmake`, a C/C++ compiler, and `libclang`.
- Run quality gates in this order: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, then `cargo test`.
- The test suite must not require a microphone, clipboard, Whisper model, or interactive terminal. Use fakes and injected paths at external boundaries.

## Application constraints

- This is a single Cargo binary. `main.rs` is startup/resource wiring and process-level errors; `app.rs` owns state transitions; `terminal.rs` owns terminal I/O; `transcriber.rs` owns model provisioning/loading.
- The implemented baseline is slice 1 only: model readiness and a Ready shell. `Ctrl+C` exits; `Ctrl+R` and `Esc` behavior belongs to later slices.
- The first run downloads the pinned Whisper `base` artifact to `$XDG_DATA_HOME/transclip/models/ggml-base.bin` (or `~/.local/share/...`) and re-verifies its SHA-256 on every startup. Set `XDG_DATA_HOME` to a temporary directory for manual runs so they never use or alter the real cache.
- Keep terminal input terminal-local; do not add global hotkeys. Keep transcription off the terminal event-loop thread, and do not introduce an async runtime or full TUI framework.

## Vertical slices

- This project uses an experimental dual-agent workflow: OpenCode skills live in `.opencode/skills/`; P-coding agent skills live in `.pi/skills/`. Keep their vertical-slice behavior aligned.
- Implement `docs/implementation-plans/` slices strictly in index order. Read `00-index.md` and resolve applicable gates in `01-decision-register.md` before coding.
- Preserve the flat `src/` layout and have infrastructure return values/events rather than mutate application state or render directly.
- For slice work, use the appropriate agent-local skills. Publication skills push only the current `vs-NNN_*` branch and open or return its PR against `dev`; never approve, merge, or close the issue without an explicit request.
