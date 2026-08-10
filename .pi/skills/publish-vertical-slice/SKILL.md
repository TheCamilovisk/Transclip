---
name: publish-vertical-slice
description: Publishes the current vertical-slice branch and opens a pull request to the dev branch that auto-closes the slice issue on merge, returning the PR link without approving or merging. Use when asked to push or publish a vertical-slice branch and create its PR to dev.
---

# Publish Vertical Slice

Push the current vertical-slice branch and open a pull request to `dev`, then report the PR link. The PR body ends with `Closes #<issue-number>`, so merging the PR auto-closes the linked slice issue. The PR is created but never approved or merged — approval/merge is always left to the user.

This skill is the publishing counterpart of `implement-vertical-slice` (which prepares the branch and implements the slice) and `next-vertical-slice` (which selects the slice). It is self-contained: the script and PR template live in this skill directory.

## Input

Run it while on a vertical-slice branch named `vs-XXX_kebab-case-title` (e.g. `vs-001_startup-and-model-readiness`). The slice's GitHub issue must exist and be titled `[VS-NNN] - Title`; the issue number must equal the slice number (`VS-001` → issue #1), per the `create-vertical-slice-issue` convention.

## Procedure

Run the helper script:

```bash
{baseDir}/scripts/publish-vs-branch.sh --dry-run   # preview title + rendered PR body + commands
{baseDir}/scripts/publish-vs-branch.sh             # push and create the PR
```

### What the script does

1. Validates the current branch matches `vs-XXX_...` and resolves the matching `[VS-NNN]` issue.
2. Derives the PR title: `[VS-001] - Startup and Model Readiness` → `VS-001: Startup and Model Readiness`.
3. Renders the PR body from `templates/pr-body.md`, filling in:
   - issue number and plan path (parsed from the issue body's `## Vertical Slice Plan` section);
   - the plan's `## Outcome` (Summary), `## Manual Checks`, and `## Acceptance Criteria`;
   - `git diff --stat origin/<base>...HEAD` (What's included);
   - a trailing `Closes #<issue>` line, so merging the PR closes the slice issue.
   A missing plan file or section only produces a warning — it does not abort.
4. Fetches, pushes the branch (`git push -u origin <branch>`), and creates the pull request against `dev` (or `--base`).
5. Reuses an existing PR for the branch instead of creating a duplicate (`status: exists`).
6. Prints the PR URL (`pr: https://...`), or a JSON object with `--json`.

### Options

- `--dry-run` — print the title, rendered body, and planned commands without changing anything.
- `--json` — emit `{"status":"created|exists","url":"...","title":"..."}`.
- `--repo OWNER/NAME` — repository (default: the current directory's).
- `--base BRANCH` — base branch (default: `dev`).
- `--body-file FILE` — use a custom PR body file instead of the template.

Manual fallback if the script cannot run: push the branch with `git push -u origin <branch>`, then `gh pr create --base dev --head <branch> --title "VS-NNN: Title" --body <rendered>`.

## Output

Human mode:

```text
status: created
pr: https://github.com/TheCamilovisk/Transclip/pull/8
```

`status: exists` means a PR for the branch already existed and its link is returned — no duplicate is created.

## Agent Responsibilities

- Review the `--dry-run` output before creating the PR: confirm the title, the Summary, the trailing `Closes #<issue>` line, and that the diff stat reflects only the slice's intended changes. If the body needs richer content, pass `--body-file` with a customized body.
- Return the PR link to the user.
- Do **not** approve, merge, or request-review the PR, and do not close or modify the linked issue directly, unless the user explicitly asks. The issue closes automatically when the PR is merged (via the `Closes #` keyword in the body).

## Exit Status

- `0` — PR created or already exists.
- `1` — GitHub query, repository resolution, branch validation, or git failure (e.g. current branch is not a `vs-XXX_...` branch, issue not found, base branch missing, plan file missing).
- `2` — usage error.

## Gotchas

- The branch must already contain its commits; the script only pushes, it does not commit.
- `git push -u` on an up-to-date branch is a no-op (`Everything up-to-date`), which is fine.
- The issue number is derived from the branch name; if the issue title's `[VS-NNN]` differs from the branch number, the script uses the branch-derived number and reports the mismatch for inspection.
- Diff stat and body are computed against `origin/<base>` after a fetch, so create or update the PR after pushing all intended commits.

## Dependencies

- `gh` (authenticated against the repository), `jq`, `git`, `awk`.
- Network access to the repository remote.
