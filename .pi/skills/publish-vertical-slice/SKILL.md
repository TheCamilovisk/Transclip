---
name: publish-vertical-slice
description: Publishes the current vertical-slice branch and opens a pull request to the dev branch that auto-closes the slice issue on merge, returning the PR link without approving or merging. Use when asked to push or publish a vertical-slice branch and create its PR to dev.
---

# Publish Vertical Slice

Push the current vertical-slice branch and open a pull request to `dev`, then report the PR link. The PR body ends with `Closes #<resolved-issue-number>`; when the PR is merged, the slice issue closes (via the repository's `.github/workflows/close-linked-issues.yml`, because GitHub's own closing keywords only work on the default branch). The PR is created but never approved or merged — approval/merge is always left to the user.

This skill is the publishing counterpart of `implement-vertical-slice` (which prepares the branch and implements the slice) and `next-vertical-slice` (which selects the slice). It is self-contained: the script and PR template live in this skill directory.

## Input

Run it while on a vertical-slice branch named `vs-XXX_kebab-case-title` (e.g. `vs-001_startup-and-model-readiness`). The slice's GitHub issue must exist, be titled `[VS-NNN] - Title`, and carry the `ai:vertical-slice` label. Its actual issue number is resolved by title via `scripts/resolve-slice-issue.sh` — it is never assumed to equal the slice number (issues and PRs share GitHub's number space).

## Procedure

Run the helper script:

```bash
{baseDir}/scripts/publish-vs-branch.sh --dry-run   # preview title + rendered PR body + commands
{baseDir}/scripts/publish-vs-branch.sh             # push and create the PR
```

### What the script does

1. Validates the current branch matches `vs-XXX_...` and resolves the matching `[VS-NNN]` issue **by title** via `scripts/resolve-slice-issue.sh` (exactly one issue with that `[VS-NNN]` prefix must exist; the script fails on none or several).
2. Derives the PR title: `[VS-001] - Startup and Model Readiness` → `VS-001: Startup and Model Readiness`.
3. Renders the PR body from `templates/pr-body.md`, filling in:
   - the resolved issue number and plan path (parsed from the issue body's `## Vertical Slice Plan` section);
   - the plan's `## Outcome` (Summary), `## Manual Checks`, and `## Acceptance Criteria`;
   - `git diff --stat origin/<base>...HEAD` (What's included);
   - a trailing `Closes #<resolved-issue>` line.
   A missing plan file or section only produces a warning — it does not abort.
4. Fetches, pushes the branch (`git push -u origin <branch>`), and creates the pull request against `dev` (or `--base`).
5. Reuses an existing PR for the branch instead of creating a duplicate (`status: exists`).
6. Prints the PR URL (`pr: https://...`), or a JSON object with `--json`.

### Closing the slice issue on merge

GitHub only interprets closing keywords (`Closes #N` etc.) when the PR targets the repository's default branch (`main`). Vertical-slice PRs target `dev`, so the keyword is ignored there. Instead, `.github/workflows/close-linked-issues.yml` performs the close: when a PR to `dev` is merged, it closes every `ai:vertical-slice`-labeled issue referenced with a closing keyword in the PR body. Both files must stay in sync — if the body ever stops carrying the `Closes #<issue>` line, the workflow closes nothing.

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

- Review the `--dry-run` output before creating the PR: confirm the title, the Summary, the trailing `Closes #<resolved-issue>` line (the number comes from `resolve-slice-issue.sh`, never assumed), and that the diff stat reflects only the slice's intended changes. If the body needs richer content, pass `--body-file` with a customized body.
- Return the PR link to the user.
- Do **not** approve, merge, or request-review the PR, and do not close or modify the linked issue directly, unless the user explicitly asks. The issue closes automatically when the PR is merged (via the `Closes #` keyword in the body, enforced by `.github/workflows/close-linked-issues.yml`).

## Exit Status

- `0` — PR created or already exists.
- `1` — GitHub query, repository resolution, branch validation, or git failure (e.g. current branch is not a `vs-XXX_...` branch, issue not found, base branch missing, plan file missing).
- `2` — usage error.

## Gotchas

- The branch must already contain its commits; the script only pushes, it does not commit.
- `git push -u` on an up-to-date branch is a no-op (`Everything up-to-date`), which is fine.
- The issue number is resolved by title (`scripts/resolve-slice-issue.sh`), not derived from the branch number; if the branch's `VS-NNN` does not match any issue title, the script fails rather than closing the wrong issue.
- Diff stat and body are computed against `origin/<base>` after a fetch, so create or update the PR after pushing all intended commits.
- Closing on merge requires `.github/workflows/close-linked-issues.yml` to be present on `dev` (the base of every slice PR) — GitHub's native closing keywords only work on the default branch (`main`).

## Dependencies

- `gh` (authenticated against the repository), `jq`, `git`, `awk`.
- Network access to the repository remote.
