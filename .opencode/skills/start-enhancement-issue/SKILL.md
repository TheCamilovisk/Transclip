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
3. Read the issue requirements through the script output and then update the applicable functional and architecture specifications. Add a new implementation-plan slice only when the enhancement requires one; append it in index order and resolve any applicable decision gates before coding.
4. Use `publish-enhancement-issue` only after the intended work is committed.

The script must remain the exclusive path for GitHub access in this workflow. Do not call `gh` directly from the skill procedure.
