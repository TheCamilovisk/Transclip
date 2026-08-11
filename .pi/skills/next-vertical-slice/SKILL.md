---
name: next-vertical-slice
description: Reports the next vertical slice implementation plan and its corresponding GitHub issue for the Transclip project. Use when asked which vertical slice to implement next, to fetch the next implementation plan, or to get the GitHub issue link for the next slice.
---

# Next Vertical Slice

Determine the next vertical slice to implement and report both the plan document and its GitHub issue. Slices are tracked as GitHub issues titled `[VS-NNN] - Title` and labeled `ai:vertical-slice` (see `.opencode/skills/create-vertical-slice-issue/SKILL.md`). Slices must be implemented in order, so the "next" slice is the lowest-numbered `[VS-NNN]` issue that is still open; closed issues count as implemented.

This skill is self-contained: the script lives in this skill directory and does not depend on the project's `scripts/` tree. It only needs `gh` (authenticated against the repository) and `jq`.

## Usage

Run the script from anywhere inside the repository, or from anywhere at all with `--repo TheCamilovisk/Transclip`:

```bash
{baseDir}/scripts/next-vertical-slice.sh
```

### Options

- `--json` — Emit a single JSON object instead of human output:
  - `{"status":"next","plan":"...","issue":"..."}`
  - `{"status":"done","message":"..."}` when everything is implemented
- `--repo OWNER/NAME` — Repository to query (default: resolved from the current directory via `gh`).

## Output

Human mode (default):

```text
status: next
plan: docs/implementation-plans/02-startup-model-readiness.md
issue: https://github.com/TheCamilovisk/Transclip/issues/1
```

## Exit Status

- `0` — A next slice was reported, or all slices are already implemented (`status: done` is not an error).
- `1` — GitHub query, repository resolution, or dependency (`gh`, `jq`) failure.
- `2` — Usage error.

## Procedure for the Agent

1. Run the script (with `--json` when the caller wants machine-readable output).
2. If `status: next`, report the plan path and the issue link. Read `docs/implementation-plans/<NN-name>.md` from the repository root when the caller asks for plan details.
3. If `status: done`, report the message and do not look for further slices.
4. If the script exits `1` (e.g., `gh` not authenticated), verify authentication with `gh auth status` and re-run.

## Dependencies

- `gh` — GitHub CLI, authenticated against `TheCamilovisk/Transclip`.
- `jq` — JSON processor.
