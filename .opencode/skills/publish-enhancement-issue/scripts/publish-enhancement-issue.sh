#!/usr/bin/env bash
# Push an enhancement branch and create or return its PR to dev.
set -euo pipefail

MODE="run"
JSON_OUT="no"
REPO=""
BASE="dev"
BODY_FILE=""
ISSUE=""

usage() {
  cat <<'EOF'
Usage: publish-enhancement-issue.sh [--dry-run] [--json] [--repo OWNER/NAME]
                                    [--base BRANCH] [--body-file FILE] [ISSUE_NUMBER]

Validates the current enhan_kebab-case-title branch and its linked open
enhancement issue, then pushes the branch and creates or returns a PR to dev.
When ISSUE_NUMBER is omitted, resolves the issue from the current branch's
title slug among open enhancement issues.
The script never approves, merges, requests review, or closes an issue.
EOF
  exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) MODE="dry-run" ;;
    --json) JSON_OUT="yes" ;;
    --repo) REPO="${2:-}"; shift ;;
    --base) BASE="${2:-}"; shift ;;
    --body-file) BODY_FILE="${2:-}"; shift ;;
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

if [[ -n "$ISSUE" && ! "$ISSUE" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: ISSUE_NUMBER must be a positive integer" >&2
  usage 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE="$SCRIPT_DIR/../templates/pr-body.md"
branch="$(git rev-parse --abbrev-ref HEAD)"
if [[ ! "$branch" =~ ^enhan_[a-z0-9]+(-[a-z0-9]+)*$ ]]; then
  echo "error: current branch is not an enhancement branch (expected enhan_kebab-case-title): $branch" >&2
  exit 1
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: worktree must be clean before publishing" >&2
  exit 1
fi
if [[ -z "$REPO" ]]; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
  if [[ -z "$REPO" ]]; then
    echo "error: could not determine the repository; run inside it or pass --repo OWNER/NAME" >&2
    exit 1
  fi
fi

if [[ -z "$ISSUE" ]]; then
  branch_slug="${branch#enhan_}"
  matches="$(gh issue list --repo "$REPO" --state open --label enhancement --limit 100 \
    --json number,title \
    | jq --arg branch_slug "$branch_slug" \
      '[.[] | select((.title | ascii_downcase | gsub("[^a-z0-9]+"; "-") | gsub("^-|-$"; "")) == $branch_slug)]')"
  match_count="$(printf '%s' "$matches" | jq 'length')"
  if [[ "$match_count" != "1" ]]; then
    echo "error: could not uniquely resolve an open enhancement issue from branch $branch" >&2
    echo "pass ISSUE_NUMBER explicitly or use an enhan_ branch whose slug exactly matches one open enhancement issue title" >&2
    exit 1
  fi
  ISSUE="$(printf '%s' "$matches" | jq -r '.[0].number')"
fi

issue="$(gh issue view "$ISSUE" --repo "$REPO" --json number,title,state,labels,url 2>/dev/null || true)"
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

if ! git rev-parse --verify -q "refs/remotes/origin/$BASE" >/dev/null; then
  echo "error: base branch origin/$BASE does not exist" >&2
  exit 1
fi
if ! git merge-base --is-ancestor "origin/$BASE" "$branch"; then
  echo "error: $branch is not based on current origin/$BASE; rebase or merge it first" >&2
  exit 1
fi
diff_stat="$(git diff --stat "origin/$BASE...$branch")"
if [[ -z "$diff_stat" ]]; then
  echo "error: no committed changes compared with origin/$BASE" >&2
  exit 1
fi

issue_title="$(printf '%s' "$issue" | jq -r '.title')"
issue_url="$(printf '%s' "$issue" | jq -r '.url')"
if [[ -n "$BODY_FILE" ]]; then
  if [[ ! -f "$BODY_FILE" ]]; then
    echo "error: body file not found: $BODY_FILE" >&2
    exit 1
  fi
  summary="$(<"$BODY_FILE")"
else
  summary="See the linked issue and committed changes."
fi
verification="- Not run (documentation/workflow change)."
body="$(jq -Rsr \
  --arg issue_number "$ISSUE" \
  --arg issue_title "$issue_title" \
  --arg issue_url "$issue_url" \
  --arg summary "$summary" \
  --arg diff_stat "$diff_stat" \
  --arg verification "$verification" \
  'gsub("\\{\\{ISSUE_NUMBER\\}\\}"; $issue_number)
   | gsub("\\{\\{ISSUE_TITLE\\}\\}"; $issue_title)
   | gsub("\\{\\{ISSUE_URL\\}\\}"; $issue_url)
   | gsub("\\{\\{SUMMARY\\}\\}"; $summary)
   | gsub("\\{\\{DIFF_STAT\\}\\}"; $diff_stat)
   | gsub("\\{\\{VERIFICATION\\}\\}"; $verification)' \
  "$TEMPLATE")"
title="enhancement: $issue_title"

echo "branch: $branch"
echo "issue:  #$ISSUE - $issue_title"
echo "title:  $title"
if [[ "$MODE" == "dry-run" ]]; then
  echo "--- planned commands ---"
  echo "  -> git push -u origin $branch"
  echo "  -> gh pr create --repo $REPO --base $BASE --head $branch --title '$title' --body '<rendered body>'"
  echo "--- PR body ---"
  printf '%s\n' "$body"
  exit 0
fi

git push -u origin "$branch"

existing="$(gh pr list --repo "$REPO" --head "$branch" --state all --json url -q '.[0].url // empty')"
if [[ -n "$existing" ]]; then
  status="exists"
  url="$existing"
else
  status="created"
  url="$(gh pr create --repo "$REPO" --base "$BASE" --head "$branch" --title "$title" --body "$body")"
fi

if [[ "$JSON_OUT" == "yes" ]]; then
  jq -n --arg status "$status" --arg url "$url" --arg title "$title" \
    '{status: $status, url: $url, title: $title}'
else
  echo "status: $status"
  echo "pr: $url"
fi
