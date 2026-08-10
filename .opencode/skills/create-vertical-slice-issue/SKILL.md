---
name: create-vertical-slice-issue
description: Create one sequentially numbered GitHub issue from a vertical slice implementation plan. Use when asked to turn a vertical slice plan file into a trackable AI implementation issue.
---

# Create Vertical Slice Issue

Create exactly one GitHub issue from one vertical slice plan file in the current repository.

## Input

The user must identify one plan file. If no file is provided, ask for its path. Do not create issues from an index, decision register, specification, or arbitrary document unless it has a heading in this form:

```text
# Slice N: Title
```

Parse `N` as the vertical slice number and use the plan path relative to the repository root. Preserve leading zeroes in the issue identifier: slice `1` becomes `VS-001`.

## Required Procedure

1. Read the plan file and verify that it contains `## Outcome` and `## Acceptance Criteria` sections. Stop if either section is missing or ambiguous.
2. Resolve the repository root with `git rev-parse --show-toplevel`. Confirm the plan is inside that root, then compute its relative path.
3. Inspect the complete issue history before creating anything. Search for an existing issue whose title contains the same `[VS-NNN]` identifier or whose body references the same relative plan path. Stop instead of creating a duplicate.
4. Verify GitHub authentication and the repository selected by `gh`. Stop if the repository cannot be identified or authenticated.
5. Ensure the `ai:vertical-slice` label exists. Create it if necessary with a suitable color and the description `Implementation guideline for AI-agent vertical slices`.
6. Determine the next GitHub issue number from all existing issues, including closed issues. Account for GitHub's shared issue and pull-request number sequence. The next available number must equal the vertical slice number.
7. If the numbers do not match, stop and explain that GitHub issue numbers are immutable. Do not close, delete, repurpose, or create a replacement issue automatically.
8. Build the issue title exactly as:

   ```text
   [VS-NNN] - Title
   ```

   Use the plan heading title as the basis for `Title`, removing only the `Slice N:` prefix. Use normal title capitalization and no trailing bracket.
9. Build the issue body exactly with these sections:

   ```markdown
   ## Vertical Slice Plan
   `relative/path/to/plan.md`

   ## Scope
   [The complete contents of the plan's Outcome section]

   ## Acceptance Criteria
   [The complete contents of the plan's Acceptance Criteria section]
   ```

   Preserve the acceptance criteria verbatim. Do not invent criteria, add implementation tasks, or silently broaden the plan's scope.
10. Create the issue serially with `gh issue create`, applying `ai:vertical-slice`. Never create multiple slice issues concurrently because GitHub assigns numbers in completion order.
11. Read the created issue back and verify its number, title, label, relative plan path, scope, and acceptance criteria. If verification fails, report the issue URL and the exact mismatch; do not create another issue.

## GitHub Number Check

Use all issue and pull-request records when calculating the next number. A robust REST-based check is:

```bash
gh api --paginate --slurp \
  'repos/{owner}/{repo}/issues?state=all&per_page=100' \
  | jq '[.[][] | .number] | max + 1'
```

If the repository has pull requests whose numbers may exceed the issue numbers returned by that endpoint, inspect the repository's latest issue/PR numbering before proceeding. Never assume that the count of open issues equals the next number.

## Verification Checklist

- The created issue number equals the parsed slice number.
- The title is `[VS-NNN] - Title`.
- The `ai:vertical-slice` label is attached.
- The body contains the plan location relative to the project root.
- The body contains the plan's Outcome under `Scope`.
- The body contains the plan's Acceptance Criteria verbatim.
- No duplicate issue was created.

Return the issue URL and a concise summary of these checks. If the number preflight fails, return the conflict and wait for explicit user direction.
