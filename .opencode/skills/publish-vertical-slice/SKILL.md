---
name: publish-vertical-slice
description: Push the current vertical-slice branch and create or return its pull request to dev, whose body closes the slice issue on merge (enforced by the repository workflow). Use when asked to publish, push, or open a PR for a vertical slice.
---

# Publish Vertical Slice

Publish the current `vs-NNN_kebab-case-title` branch and return its pull-request URL. The PR body ends with `Closes #<resolved-issue-number>`; when the PR is merged, the slice issue closes via `.github/workflows/close-linked-issues.yml` (GitHub's own closing keywords only work on the default branch, and these PRs target `dev`). The issue number is resolved by title through `scripts/resolve-slice-issue.sh` — never assumed to equal the slice number. This is the OpenCode counterpart to `.pi/skills/publish-vertical-slice/`; keep both workflows aligned.

## Procedure

1. Confirm the current branch contains the intended committed slice work. Do not create commits for the user.
2. Preview the repository operations and generated body:

   ```bash
   .opencode/skills/publish-vertical-slice/scripts/publish-vs-branch.sh --dry-run
   ```

3. Review the preview: the branch must match `vs-NNN_*`, the matching `[VS-NNN]` issue must exist and be resolved by title via `scripts/resolve-slice-issue.sh` (exactly one issue with that prefix), the target must be `dev`, and the diff must contain only the slice work.
4. Publish and create or reuse the pull request:

   ```bash
   .opencode/skills/publish-vertical-slice/scripts/publish-vs-branch.sh
   ```

5. Return the `pr:` URL. Do not approve, merge, request review, or close the issue directly unless explicitly asked. The issue closes automatically when the PR is merged (via the `Closes #` keyword in the body, enforced by `.github/workflows/close-linked-issues.yml`).

## Options

```text
--dry-run              Preview without pushing or creating a PR.
--json                 Emit {status, url, title} JSON.
--repo OWNER/NAME      Override the GitHub repository.
--base BRANCH          Override the target branch (defaults to dev).
--body-file FILE       Use a custom PR body instead of the bundled template.
```

The script requires authenticated `gh`, `git`, and `jq`. It resolves the slice issue by title (`scripts/resolve-slice-issue.sh`), derives the PR title and body from the matching issue and plan, sets the branch upstream, and returns an existing PR instead of creating a duplicate. Closing on merge requires `.github/workflows/close-linked-issues.yml` to exist on `dev` — GitHub's native closing keywords only work on the default branch (`main`).
