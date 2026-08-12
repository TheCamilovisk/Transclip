---
name: start-enhancement-issue
description: Start work on an open enhancement issue by synchronizing dev and creating its enhan_kebab-case branch. Use when asked to begin or create a branch for an enhancement issue.
---

# Start Enhancement Issue

Create the working branch for one open GitHub issue labeled `enhancement`.

## Input

The user must provide the enhancement issue number. If it is missing, ask for it.

## Procedure

1. Before changing specifications or implementation plans, run the supporting script from the repository root:

   ```bash
   .opencode/skills/start-enhancement-issue/scripts/start-enhancement-issue.sh ISSUE_NUMBER
   ```

2. Return the script output unchanged. The script performs every GitHub interaction and verifies that:
   - the issue is open and has the `enhancement` label;
   - the worktree is clean;
   - local `dev` exactly matches `origin/dev` after fetching;
   - the new `enhan_kebab-case-title` branch is created from `dev`.
3. Read the issue requirements through the script output. Update `docs/specifications/functional-spec.md` and `docs/specifications/architecture-spec.md` as required to fully capture them. Create an indexed vertical-slice implementation plan in `docs/implementation-plans/`, update `00-index.md`, and resolve applicable gates in `01-decision-register.md`.
4. Do not modify application code, tests, dependencies, or runtime documentation. This workflow prepares specifications and plans only; a later vertical-slice implementation workflow performs code changes in its own pull request.
5. Use `publish-enhancement-issue` only after the intended documentation work is committed.

The script must remain the exclusive path for GitHub access in this workflow. Do not call `gh` directly from the skill procedure.
