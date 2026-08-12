#!/usr/bin/env bash
# Synchronize dev and create an enhancement branch for one open issue.
set -euo pipefail

JSON_OUT="no"
REPO=""
ISSUE=""

usage() {
  cat <<'EOF'
Usage: start-enhancement-issue.sh [--json] [--repo OWNER/NAME] ISSUE_NUMBER

Verifies that ISSUE_NUMBER is open and labeled enhancement, requires a clean
worktree and local dev equal to origin/dev, then creates
enhan_kebab-case-title from dev. All GitHub queries are performed here.
EOF
  exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --json) JSON_OUT="yes" ;;
    --repo) REPO="${2:-}"; shift ;;
    -h|--help) usage 0 ;;
    -*) echo "error: unknown option: $1" >&2; usage 2 ;;
    *)
      if [[ -n "$ISSUE" ]]; then
        echo "error: provide exactly one issue number" >&2
        usage 2
      fi
      ISSUE="$1"
      ;;
  esac
  shift
done

for command in gh git jq; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "error: '$command' is required but not installed" >&2
    exit 1
  fi
done

if [[ ! "$ISSUE" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: ISSUE_NUMBER must be a positive integer" >&2
  usage 2
fi

if [[ -z "$REPO" ]]; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
  if [[ -z "$REPO" ]]; then
    echo "error: could not determine the repository; run inside it or pass --repo OWNER/NAME" >&2
    exit 1
  fi
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: worktree must be clean before starting an enhancement" >&2
  exit 1
fi

issue="$(gh issue view "$ISSUE" --repo "$REPO" --json number,title,body,state,labels,url 2>/dev/null || true)"
if [[ -z "$issue" ]]; then
  echo "error: issue #$ISSUE not found in $REPO (is gh authenticated?)" >&2
  exit 1
fi
if [[ "$(printf '%s' "$issue" | jq -r '.state')" != "OPEN" ]]; then
  echo "error: issue #$ISSUE is not open" >&2
  exit 1
fi
if ! printf '%s' "$issue" | jq -e '[.labels[].name] | index("enhancement") != null' >/dev/null; then
  echo "error: issue #$ISSUE is not labeled enhancement" >&2
  exit 1
fi

slug="$(printf '%s' "$title" \
  | tr '[:upper:]' '[:lower:]' \
  | tr -cs '[:alnum:]' '-' \
  | sed 's/^-*//; s/-*$//')"
if [[ -z "$slug" ]]; then
  echo "error: issue #$ISSUE title cannot produce a branch name" >&2
  exit 1
fi
branch="enhan_$slug"

git fetch origin dev
if ! git show-ref --verify --quiet refs/heads/dev; then
  echo "error: local dev branch does not exist" >&2
  exit 1
fi
if [[ "$(git rev-parse dev)" != "$(git rev-parse origin/dev)" ]]; then
  echo "error: local dev is not synchronized with origin/dev; update dev first" >&2
  exit 1
fi
if git show-ref --verify --quiet "refs/heads/$branch" || git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1; then
  echo "error: enhancement branch already exists: $branch" >&2
  exit 1
fi

git switch dev
git switch -c "$branch"

if [[ "$JSON_OUT" == "yes" ]]; then
  jq -n \
    --arg repo "$REPO" \
    --arg branch "$branch" \
    --argjson issue "$issue" \
    '{repo: $repo, branch: $branch, issue: $issue}'
else
  echo "branch: $branch"
  echo "issue:  #$(printf '%s' "$issue" | jq -r '.number') - $title"
  echo "url:    $(printf '%s' "$issue" | jq -r '.url')"
fi
