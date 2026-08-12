---
name: publish-enhancement-issue
description: Push an enhan_ branch and create or return its pull request to dev for an enhancement issue. Use when asked to publish, push, or open a PR for an enhancement issue.
---

# Publish Enhancement Issue

Publish committed work from the current `enhan_kebab-case-title` branch and create or return its pull request to `dev`.

## Input

The current enhancement branch identifies the linked issue automatically. An optional issue number may disambiguate the branch-to-issue match, and an optional Markdown body file may provide a concise summary and verification details.

## Procedure

1. Confirm the current branch contains the intended enhancement work. Commit all work completed in the current enhancement branch before publishing; stage only the intended files and use a concise message matching repository style.
2. Preview all repository operations and the generated PR body through the supporting script:

   ```bash
   .opencode/skills/publish-enhancement-issue/scripts/publish-enhancement-issue.sh --dry-run
   ```

3. Review the preview: the branch must match `enhan_*`; the enhancement issue must be open and labeled `enhancement`; the target must be `dev`; and the diff must be non-empty and limited to the intended work.
4. Publish and create or reuse the PR through the script:

   ```bash
   .opencode/skills/publish-enhancement-issue/scripts/publish-enhancement-issue.sh
   ```

5. Return the `pr:` URL. Do not approve, merge, request review, or close the issue directly unless explicitly asked. The generated `Closes #ISSUE_NUMBER` link is closed after merge into `dev` by the repository workflow.

The script must remain the exclusive path for GitHub access in this workflow. Do not call `gh` directly from the skill procedure.
