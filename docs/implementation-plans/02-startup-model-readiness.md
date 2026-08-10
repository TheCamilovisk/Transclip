# Slice 1: Startup And Model Readiness

## Outcome

A Linux Rust executable reaches an interactive `Ready to record` terminal state only after a pinned, integrity-verified local Whisper model is available and loaded. Any provisioning, verification, model-load, required-terminal, or unsupported-runtime failure produces a clear startup error and exits without enabling recording.

## Prerequisites

- Resolve D1-D3 in `01-decision-register.md` before selecting model constants, cache behavior, dependency versions, or packaging instructions.
- Initialize a standard Cargo binary project if it does not exist. Add only dependencies needed for this slice: Crossterm, Whisper binding, cache-directory support, HTTPS download, SHA-256, and error handling justified by the selected approach.

## Implementation Steps

1. Create the prescribed flat `src/` layout, keeping `main.rs` restricted to startup, channel/resource construction, app-loop invocation, and process-level reporting.
2. Add model-release metadata as compile-time release constants, not CLI arguments or user configuration. Include source provenance in code or project documentation.
3. Resolve the Linux user data directory using the selected platform-directory crate or an explicitly documented XDG-compatible implementation.
4. Implement `ensure_model()`:
   - create the cache directory with owner-appropriate permissions;
   - reuse an existing artifact only according to the D1 integrity policy;
   - download to a temporary file when absent or invalid;
   - SHA-256 verify before it becomes the cached artifact;
   - atomically install it only after verification;
   - remove or safely retain failed temporary downloads according to D1.
5. Load the verified model once. Do not expose Ready if initialization fails. The later long-lived worker will own the loaded context; choose an initialization handoff compatible with D2/D5.
6. Implement a terminal RAII guard that enables required per-key input mode and restores raw mode, cursor visibility, and changed modes on drop.
7. Render an initial Ready status with `Ctrl+R  Start recording`. Keep rendering simple and compatible with persistent terminal output required by later slices.
8. Handle terminal initialization errors before interactive mode is partially entered. Ensure all startup failures return a non-zero process status after cleanup.

## Automated Tests

- Cache path resolution is deterministic under injected/test paths.
- Missing model requests download; valid cached model skips it.
- Incorrect checksum and unreadable/corrupt artifact fail before model loading.
- Failed download never installs an accepted artifact.
- Model-load failure prevents creation of a usable application loop.
- Terminal guard restoration logic is covered as far as test seams permit; do not require a real TTY in unit tests.

## Manual Checks

- First run displays provisioning feedback appropriate to the chosen terminal policy and reaches Ready only after completion.
- Offline, invalid checksum, and model-load failures show actionable errors and restore the terminal.
- Deleting the cached model causes a verified re-download.

## Acceptance Criteria

- Functional specification 2.1, 15.5, and 19.1 are satisfied.
- Architecture sections 3.4, 14, 24, 28, 31, 41, and 42 are satisfied.
- No microphone controls exist yet; the slice only establishes a safe Ready shell.
