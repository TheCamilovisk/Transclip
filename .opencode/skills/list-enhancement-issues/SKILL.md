---
name: list-enhancement-issues
description: List open GitHub issues labeled enhancement as Markdown title links. Use when asked to retrieve, show, or list open enhancement issues from the remote repository.
---

# List Enhancement Issues

List the open issues labeled `enhancement` in the current repository.

Run the supporting script from the repository root:

```bash
.opencode/skills/list-enhancement-issues/scripts/list-enhancement-issues.sh
```

To query a different repository, pass its GitHub owner/name:

```bash
.opencode/skills/list-enhancement-issues/scripts/list-enhancement-issues.sh --repo OWNER/NAME
```

Return the script output unchanged. The script performs every GitHub interaction and formats each issue as its title linked to its GitHub URL.
