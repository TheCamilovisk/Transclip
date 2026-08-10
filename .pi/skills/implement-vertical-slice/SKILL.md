---
name: implement-vertical-slice
description: Prepares and implements a Transclip vertical slice end to end — resolves the decision gates, prepares the dev and vs-XXX_* branches, implements the plan with its automated tests, passes quality gates and manual checks, and commits. Use when asked to implement a vertical slice or to proceed with a slice implementation.
---

# Implement Vertical Slice

Prepare and implement a vertical slice for the Transclip project. Slices are tracked as GitHub issues titled `[VS-NNN] - Title` and labeled `ai:vertical-slice`; each issue's body points to its plan in `docs/implementation-plans/`. Slices must be implemented in order (see `docs/implementation-plans/00-index.md`).

This skill is the execution counterpart of `next-vertical-slice` (which only reports the next plan and issue). It is self-contained: the helper script lives in this skill directory.

## Input

- By default, implement the **next** slice: run the `next-vertical-slice` skill (or its script) to get the plan path and issue URL.
- If the user names a specific slice number, verify it is the lowest-numbered open `[VS-NNN]` issue before starting. If it is not the next slice, surface the discrepancy and ask — do not silently skip ahead.
- If all slices are done (`status: done`), stop and report that; do not invent work.

## Phase 1 — Preparation (research and decision gates)

1. Read the plan file completely (`.pi/skills/next-vertical-slice/scripts/next-vertical-slice.sh --json` gives its path).
2. Read `docs/implementation-plans/00-index.md` for plan rules, shared contracts (e.g. `AppMode`, `UserCommand`, `AppEvent`), and the global Definition of Done.
3. Read `docs/implementation-plans/01-decision-register.md`. Identify every **blocking decision** whose "Affected slices" column includes the current slice number.
4. Read the specification sections the plan references (functional spec `## N.M` and architecture `# N` sections) plus enough surrounding context to implement correctly. Do not rely on the plan text alone.
5. **Resolve each blocking decision before writing code.** This is a gate, not a suggestion. For each decision, verify empirically rather than from memory:
   - dependency versions and their APIs (crates.io API for versions; download the crate source from `static.crates.io` and read the actual signatures);
   - artifact URLs, sizes, and SHA-256 checksums — compute them from a real download and cross-check against the publisher's metadata (e.g. HF `x-linked-etag`);
   - vendor provenance (e.g. which whisper.cpp revision a binding vendors).
6. Record the resolutions in the decision register as a "Slice N Gate Resolutions" section with evidence (constants/provenance location, tests that prove it, verification date). Never leave a gate resolved only in your head.

## Phase 2 — Branch setup

Run the helper script, or do the equivalent steps manually:

```bash
{baseDir}/scripts/prepare-vs-branch.sh --dry-run ISSUE_NUMBER   # preview first
{baseDir}/scripts/prepare-vs-branch.sh ISSUE_NUMBER            # then run
```

The script guarantees:
- a local `dev` branch exists (created from `origin/dev`, else from `main`) and is in sync with the latest changes (fast-forwarded if behind; unpushed commits pushed; diverged → stop);
- a branch named `vs-XXX_kebab-case-title` is created from `dev` and checked out, where `XXX` is the zero-padded slice number from the issue title and the title is in kebab case (e.g. `[VS-001] - Startup and Model Readiness` → `vs-001_startup-and-model-readiness`);
- an already-existing slice branch is never clobbered.

Manual fallback: `git fetch origin --prune`; create/update `dev`; `git checkout -b vs-XXX_kebab-case-title dev`.

## Phase 3 — Implementation

Follow the plan's **Implementation Steps** in order, then its **Automated Tests**:

1. Respect the plan rules from `00-index.md`: flat `src/` layout (`main.rs`, `app.rs`, `terminal.rs`, `recorder.rs`, `transcriber.rs`, `clipboard.rs`, optional `audio.rs`); std threads, `mpsc`, `Arc`, atomics — no async runtime or TUI framework; the application controller exclusively owns state transitions; only small boundary traits that enable hardware-free tests.
2. Keep `main.rs` restricted to startup, resource/channel construction, app-loop invocation, and process-level reporting.
3. Add only dependencies the slice actually needs, pinned to the versions verified in Phase 1. No speculative features, abstractions, or error handling for impossible scenarios.
4. Implement the plan's "Automated Tests" as unit tests that run with no microphone, clipboard, model, or interactive terminal. Use injected paths and fake collaborators at external boundaries.
5. A slice must not replace a prior acceptance behavior — it may only extend it.
6. If implementation conflicts with a decision gate or a spec section, stop and surface it rather than picking silently.

## Phase 4 — Automated quality gates

All three must pass before manual verification:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Fix every warning — `-D warnings` is part of the global Definition of Done.

## Phase 5 — Manual verification

Run the plan's **Manual Checks**. For a terminal application specifically:

- Use a pseudo-TTY, never a bare pipe, to exercise interactive behavior:
  `(sleep 3; printf '\003') | TERM=xterm script -qec "./target/debug/transclip" /dev/null`
- Verify: the expected status block is rendered; a key exits cleanly (exit code 0); the terminal is restored (cursor hide `^[[?25l` on entry, show `^[[?25h` on exit).
- **Isolate the app's data directory**: set `XDG_DATA_HOME` (or the equivalent env var) to a temp dir for every manual run so tests never touch the real home directory. Clean up artifacts afterward.
- Verify error paths explicitly: cold-cache first run (real download), corrupt cache (verified re-download), download failure (clear error, exit non-zero, nothing installed), and any plan-specific failure (e.g. no-TTY terminal init). Confirm exit codes and that no partial state is left behind.
- Beware timing races: sending a key before raw mode is active delivers SIGINT (exit 130) instead of a clean key exit; allow enough time for provisioning before sending input.

## Phase 6 — Commit and report

1. Commit on the `vs-XXX_...` branch with a clear conventional message referencing the slice (e.g. `feat: implement VS-001 startup and model readiness`) and a body summarizing what was delivered and how it was verified.
2. Report: the plan path, issue URL, branch name, commit, gate resolutions recorded, and verification evidence (tests, quality gates, manual checks).
3. Do **not** push, open a PR, merge, or close the issue unless the user explicitly asks. Offer these as next steps.

## Definition of Done

- [ ] Blocking decisions for the slice resolved and recorded in the decision register with evidence.
- [ ] `dev` in sync; `vs-XXX_kebab-case-title` branch created from `dev`.
- [ ] Plan implementation steps completed; acceptance criteria from the plan are satisfied.
- [ ] Plan's automated tests implemented and passing; `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all clean.
- [ ] Manual checks from the plan executed (with isolated data dirs; real downloads verified).
- [ ] Committed on the slice branch; push/PR/issue-close offered, not performed.

## Gotchas (learned on VS-001)

- `anyhow`: `err.to_string()` shows only the outermost context — assert on `format!("{err:#}")` for the full chain.
- `sha2` 0.11 (digest 0.11) removed the `io::Write` impl for hashers — stream manually with `Read` + `Update`.
- Test-only construction of release metadata with `&'static str` fields needs `Box::leak` for computed checksums.
- Rust trait imports that are easy to miss in tests: `std::io::IsTerminal`, `std::os::unix::fs::PermissionsExt`.
- Common clippy nits: `&temp.path()` → `temp.path()` (needless_borrow), `io::Error::new(ErrorKind::Other, ..)` → `io::Error::other(..)`.
- Truncated downloads produce wrong hashes — always verify a full download and cross-check the publisher's metadata; this is exactly why the SHA-256 gate exists.
- A slice branch must be created from `dev`, and a missing `dev` is created from the latest `main` (including any unpushed local commits — verify with `git rev-list --left-right --count main...origin/main`).

## Dependencies

- `gh` (authenticated against the repository), `jq`, `git` for the helper script.
- Rust toolchain (cargo, rustc), plus native build tools the selected crates require (e.g. cmake, a C/C++ compiler, libclang for bindgen-based bindings).
- Network access to crates.io and the artifact host (e.g. huggingface.co) for version/artifact verification and first-run provisioning.
- `script` (util-linux) for pseudo-TTY manual checks.
