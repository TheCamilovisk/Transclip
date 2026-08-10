## Summary

Implements **{{PR_TITLE}}** (issue #{{ISSUE_NUMBER}}, plan `{{PLAN_PATH}}`).

{{OUTCOME}}

## What's included

```text
{{DIFF_STAT}}
```

## Verification

- `cargo fmt --check` — clean
- `cargo clippy --all-targets -- -D warnings` — clean
- `cargo test` — all tests pass
- Manual checks per the plan:
{{MANUAL_CHECKS}}

## Acceptance Criteria

{{ACCEPTANCE_CRITERIA}}

---

Closes #{{ISSUE_NUMBER}}
