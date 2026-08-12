---
name: publish-enhancement-issue
description: Push an enhan_ branch and create or return its pull request to dev for an enhancement issue. Use when asked to publish, push, or open a PR for an enhancement issue.
---

# Publish Enhancement Issue

Publish committed work from the current `enhan_kebab-case-title` branch and create or return its pull request to `dev`.

## Input

The user must provide the linked enhancement issue number. If it is missing, ask for it. An optional Markdown body file may provide a concise summary and verification details.

## Procedure

1. Confirm the current branch contains the intended committed enhancement work. Do not create commits for the user.
2. Preview all repository operations and the generated PR body through the supporting script:

   ```bash
   .opencode/skills/publish-enhancement-issue/scripts/publish-enhancement-issue.sh --dry-run ISSUE_NUMBER
   ```

3. Review the preview: the branch must match `enhan_*`; the enhancement issue must be open and labeled `enhancement`; the target must be `dev`; and the diff must be non-empty and limited to the intended work.
4. Publish and create or reuse the PR through the script:

   ```bash
   .opencode/skills/publish-enhancement-issue/scripts/publish-enhancement-issue.sh ISSUE_NUMBER
   ```

5. Return the `pr:` URL. Do not approve, merge, request review, or close the issue directly unless explicitly asked. The generated `Closes #ISSUE_NUMBER` link is closed after merge into `dev` by the repository workflow.

The script must remain the exclusive path for GitHub access in this workflow. Do not call `gh` directly from the skill procedure.
