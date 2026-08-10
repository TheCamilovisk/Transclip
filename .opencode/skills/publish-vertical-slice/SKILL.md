---
name: publish-vertical-slice
description: Push the current vertical-slice branch and create or return its pull request to dev. Use when asked to publish, push, or open a PR for a vertical slice.
---

# Publish Vertical Slice

Publish the current `vs-NNN_kebab-case-title` branch and return its pull-request URL. This is the OpenCode counterpart to `.pi/skills/publish-vertical-slice/`; keep both workflows aligned.

## Procedure

1. Confirm the current branch contains the intended committed slice work. Do not create commits for the user.
2. Preview the repository operations and generated body:

   ```bash
   .opencode/skills/publish-vertical-slice/scripts/publish-vs-branch.sh --dry-run
   ```

3. Review the preview: the branch must match `vs-NNN_*`, the matching `[VS-NNN]` issue must exist, the target must be `dev`, and the diff must contain only the slice work.
4. Publish and create or reuse the pull request:

   ```bash
   .opencode/skills/publish-vertical-slice/scripts/publish-vs-branch.sh
   ```

5. Return the `pr:` URL. Do not approve, merge, request review, or close the issue unless explicitly asked.

## Options

```text
--dry-run              Preview without pushing or creating a PR.
--json                 Emit {status, url, title} JSON.
--repo OWNER/NAME      Override the GitHub repository.
--base BRANCH          Override the target branch (defaults to dev).
--body-file FILE       Use a custom PR body instead of the bundled template.
```

The script requires authenticated `gh`, `git`, and `jq`. It derives the PR title and body from the matching issue and plan, sets the branch upstream, and returns an existing PR instead of creating a duplicate.
