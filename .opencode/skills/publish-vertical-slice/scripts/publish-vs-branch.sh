#!/usr/bin/env bash
# Push the current vertical-slice branch and create or return its PR to dev.
set -euo pipefail

MODE="run"
JSON_OUT="no"
REPO=""
BASE="dev"
BODY_FILE=""

usage() {
  cat <<'EOF'
Usage: publish-vs-branch.sh [--dry-run] [--json] [--repo OWNER/NAME]
                            [--base BRANCH] [--body-file FILE]

Pushes the current vs-NNN_kebab-case-title branch and creates or returns its
pull request. The default target branch is dev. The issue number is resolved
by title via resolve-slice-issue.sh (never assumed to equal the slice
number); the PR body ends with `Closes #<issue>`. GitHub only honors closing
keywords on the default branch, so the repository workflow
.github/workflows/close-linked-issues.yml closes the issue when the PR is
merged into dev. The script itself never approves or merges a PR and never
closes an issue directly.
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
    *) echo "error: unexpected argument: $1" >&2; usage 2 ;;
  esac
  shift
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE="$SCRIPT_DIR/../templates/pr-body.md"

for command in gh jq git awk; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "error: '$command' is required but not installed" >&2
    exit 1
  fi
done

run() {
  echo "  -> $*"
  [[ "$MODE" == "dry-run" ]] || "$@"
}

warn() {
  echo "note: $*" >&2
}

if [[ -z "$REPO" ]]; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
  if [[ -z "$REPO" ]]; then
    echo "error: could not determine the repository; run from inside the repo or pass --repo OWNER/NAME" >&2
    exit 1
  fi
fi

branch="$(git rev-parse --abbrev-ref HEAD)"
slice_id="$(printf '%s' "$branch" | sed -nE 's/^vs-([0-9]{3})_.*/\1/p')"
if [[ -z "$slice_id" ]]; then
  echo "error: current branch is not a vertical-slice branch (expected vs-NNN_kebab-case-title): $branch" >&2
  exit 1
fi
slice_number=$((10#$slice_id))

# Resolve the actual issue number by title — it is never assumed to equal the
# slice number (issues and PRs share GitHub's number space).
issue_number="$("$SCRIPT_DIR/resolve-slice-issue.sh" --repo "$REPO" "$slice_id")" || exit 1

issue="$(gh issue view "$issue_number" --repo "$REPO" --json title,body,url -q . 2>/dev/null || true)"
if [[ -z "$issue" ]]; then
  echo "error: issue #$issue_number not found in $REPO (is gh authenticated?)" >&2
  exit 1
fi
issue_title="$(printf '%s' "$issue" | jq -r '.title')"
issue_prefix="[VS-$slice_id]"
if [[ "$issue_title" != "$issue_prefix"\ -* ]]; then
  echo "error: issue #$issue_number title must begin '$issue_prefix -': $issue_title" >&2
  exit 1
fi
title="VS-$slice_id: ${issue_title#"$issue_prefix - "}"

plan_path="$(printf '%s' "$issue" | jq -r '
  .body as $body
  | if $body == null then "" else
      ($body | split("\n")) as $lines
      | ($lines | index("## Vertical Slice Plan")) as $index
      | if $index == null then "" else ($lines[$index + 1] // "") end
    end
  | gsub("^[` ]+|[` ]+$"; "")')"

section() {
  awk -v heading="$2" '
    $0 == heading { found = 1; next }
    found && /^## / { exit }
    found { lines[++count] = $0 }
    END {
      start = 1
      while (start <= count && lines[start] ~ /^[[:space:]]*$/) start++
      end = count
      while (end >= start && lines[end] ~ /^[[:space:]]*$/) end--
      for (index = start; index <= end; index++) print lines[index]
    }
  ' "$1"
}

outcome=""
manual_checks=""
acceptance=""
if [[ -n "$plan_path" && -f "$plan_path" ]]; then
  outcome="$(section "$plan_path" '## Outcome')"
  manual_checks="$(section "$plan_path" '## Manual Checks')"
  acceptance="$(section "$plan_path" '## Acceptance Criteria')"
  [[ -n "$outcome" ]] || warn "plan $plan_path has no ## Outcome section"
  [[ -n "$manual_checks" ]] || warn "plan $plan_path has no ## Manual Checks section"
  [[ -n "$acceptance" ]] || warn "plan $plan_path has no ## Acceptance Criteria section"
else
  warn "plan file not found from issue body: ${plan_path:-<missing>}"
fi

run git fetch origin --prune
if ! git rev-parse --verify -q "refs/remotes/origin/$BASE" >/dev/null; then
  echo "error: base branch origin/$BASE does not exist" >&2
  exit 1
fi
diff_stat="$(git diff --stat "origin/$BASE...$branch")"

if [[ -n "$BODY_FILE" ]]; then
  if [[ ! -f "$BODY_FILE" ]]; then
    echo "error: body file not found: $BODY_FILE" >&2
    exit 1
  fi
  body="$(<"$BODY_FILE")"
else
  body="$(jq -Rsr \
    --arg title "$title" \
    --arg issue_number "$issue_number" \
    --arg plan_path "$plan_path" \
    --arg outcome "$outcome" \
    --arg diff_stat "$diff_stat" \
    --arg manual_checks "$manual_checks" \
    --arg acceptance "$acceptance" \
    'gsub("\\{\\{PR_TITLE\\}\\}"; $title)
     | gsub("\\{\\{ISSUE_NUMBER\\}\\}"; $issue_number)
     | gsub("\\{\\{PLAN_PATH\\}\\}"; $plan_path)
     | gsub("\\{\\{OUTCOME\\}\\}"; $outcome)
     | gsub("\\{\\{DIFF_STAT\\}\\}"; $diff_stat)
     | gsub("\\{\\{MANUAL_CHECKS\\}\\}"; $manual_checks)
     | gsub("\\{\\{ACCEPTANCE_CRITERIA\\}\\}"; $acceptance)' \
    "$TEMPLATE")"
fi

echo "branch: $branch"
echo "issue:  $issue_title"
echo "title:  $title"

if [[ "$MODE" == "dry-run" ]]; then
  echo "--- planned commands ---"
  run git push -u origin "$branch"
  run gh pr create --repo "$REPO" --base "$BASE" --head "$branch" --title "$title" --body '<rendered body>'
  echo "--- PR body ---"
  printf '%s\n' "$body"
  exit 0
fi

run git push -u origin "$branch"
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
