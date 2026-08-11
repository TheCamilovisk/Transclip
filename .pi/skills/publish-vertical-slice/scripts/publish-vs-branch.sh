#!/usr/bin/env bash
#
# publish-vs-branch.sh — Push the current vertical-slice branch and open a PR to dev.
#
# For the current `vs-XXX_kebab-case-title` branch:
#   1. validates the branch name and resolves the matching [VS-NNN] issue by
#      title via resolve-slice-issue.sh (the issue number is looked up, never
#      assumed to equal the slice number);
#   2. derives the PR title (`VS-001: Startup and Model Readiness`) and fills
#      the PR body template (templates/pr-body.md) with the issue number, plan
#      path, plan Outcome, diff stat vs the base branch, plan Manual Checks,
#      and plan Acceptance Criteria;
#   3. pushes the branch (setting upstream);
#   4. creates a pull request against `dev` (or `--base`), or reports the
#      existing PR if one already exists — never a duplicate;
#   5. prints the PR URL.
#
# The PR body ends with `Closes #<issue>` (filled from the resolved issue
# number). GitHub only honors closing keywords when a PR targets the default
# branch (`main`); these PRs target `dev`, so the repository workflow
# `.github/workflows/close-linked-issues.yml` performs the actual close when
# the PR is merged. The script itself still never approves or merges a PR and
# never closes an issue directly.
#
# Usage:
#   publish-vs-branch.sh [--dry-run] [--json] [--repo OWNER/NAME]
#                        [--base BRANCH] [--body-file FILE]
#
#   --dry-run     Print the PR title, the rendered body, and the planned
#                 commands without changing anything.
#   --json        Emit a single JSON object:
#                 {"status":"created|exists","url":"...","title":"..."}
#   --repo        Repository to query (default: the one of the current directory).
#   --base        Base branch for the pull request (default: dev).
#   --body-file   Use this file as the PR body instead of the template.
#
# Exit status:
#   0  PR created or already exists
#   1  GitHub query, repository, or git failure
#   2  usage error
set -euo pipefail

MODE="run"
JSON_OUT="no"
REPO=""
BASE="dev"
BODY_FILE=""

usage() {
  sed -n '2,55p' "$0" | sed 's/^# \{0,1\}//'
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

for cmd in gh jq git; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: '$cmd' is required but not installed" >&2
    exit 1
  fi
done

run() { echo "  -> $*"; [[ "$MODE" == "dry-run" ]] || "$@"; }
warn() { echo "note: $*" >&2; }

# Resolve the repository to query.
if [[ -z "$REPO" ]]; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
  if [[ -z "$REPO" ]]; then
    echo "error: could not determine the repository; run from inside the repo or pass --repo OWNER/NAME" >&2
    exit 1
  fi
fi

# The current branch must be a vertical-slice branch.
branch="$(git rev-parse --abbrev-ref HEAD)"
vs_pad="$(printf '%s' "$branch" | sed -nE 's/^vs-([0-9]{3})_.*/\1/p')"
if [[ -z "$vs_pad" ]]; then
  echo "error: current branch is not a vertical-slice branch (expected 'vs-NNN_kebab-case-title'): $branch" >&2
  exit 1
fi
vs_num=$((10#$vs_pad))

# Resolve the actual issue number by title — it is never assumed to equal the
# slice number (issues and PRs share GitHub's number space).
issue_number="$("$SCRIPT_DIR/resolve-slice-issue.sh" --repo "$REPO" "$vs_pad")" || exit 1

# Fetch the matching issue.
issue_json="$(gh issue view "$issue_number" --repo "$REPO" --json number,title,body,url -q . 2>/dev/null || true)"
if [[ -z "$issue_json" ]]; then
  echo "error: issue #$issue_number not found in $REPO (is gh authenticated?)" >&2
  exit 1
fi
issue_title="$(printf '%s' "$issue_json" | jq -r '.title')"

# PR title: "[VS-001] - Startup and Model Readiness" -> "VS-001: Startup and Model Readiness".
rest="$(printf '%s' "$issue_title" | sed -nE 's/^\[VS-[0-9]+\][[:space:]]*-[[:space:]]*//p')"
if [[ -z "$rest" ]]; then
  echo "error: issue title has no [VS-NNN] prefix: $issue_title" >&2
  exit 1
fi
pr_title="VS-$(printf '%03d' "$vs_num"): $rest"
echo "branch: $branch"
echo "issue:  $issue_title"
echo "title:  $pr_title"

# Plan path from the issue body ("## Vertical Slice Plan" / "## Plan" + path).
plan_path="$(printf '%s' "$issue_json" | jq -r '
  .body as $b
  | if $b == null then ""
    else ($b | split("\n")) as $lines
      | (($lines | map(select(test("^## (Vertical Slice )?Plan$"))) | first) as $hdr
        | if $hdr == null then ""
          else ($lines | index($hdr) + 1) as $i
            | (($lines[$i] // "") | gsub("^[` ]+|[` ]+$"; ""))
          end)
    end')"
if [[ -z "$plan_path" ]]; then
  warn "no plan path found in issue #$vs_num body"
fi

# Extract plan sections (Outcome, Manual Checks, Acceptance Criteria),
# trimmed of leading/trailing blank lines.
section() { # $1 = file, $2 = exact "## Heading"
  awk -v s="$2" '
    $0 == s { found = 1; next }
    found && /^## / { exit }
    found { lines[++n] = $0 }
    END {
      if (!found) exit
      start = 1
      while (start <= n && lines[start] ~ /^[[:space:]]*$/) start++
      end = n
      while (end >= start && lines[end] ~ /^[[:space:]]*$/) end--
      for (i = start; i <= end; i++) print lines[i]
    }
  ' "$1"
}
outcome=""
manual_checks=""
acceptance=""
if [[ -n "$plan_path" ]] && [[ -f "$plan_path" ]]; then
  outcome="$(section "$plan_path" '## Outcome')"
  manual_checks="$(section "$plan_path" '## Manual Checks')"
  acceptance="$(section "$plan_path" '## Acceptance Criteria')"
  [[ -n "$outcome" ]] || warn "plan $plan_path has no '## Outcome' section"
  [[ -n "$manual_checks" ]] || warn "plan $plan_path has no '## Manual Checks' section"
  [[ -n "$acceptance" ]] || warn "plan $plan_path has no '## Acceptance Criteria' section"
elif [[ -n "$plan_path" ]]; then
  warn "plan file not found at $plan_path"
fi

# Ensure the base branch is available, then compute the diff stat.
run git fetch origin --prune
if ! git rev-parse --verify -q "refs/remotes/origin/$BASE" >/dev/null; then
  echo "error: base branch 'origin/$BASE' does not exist" >&2
  exit 1
fi
diff_stat="$(git diff --stat "origin/$BASE...$branch" 2>/dev/null || true)"

# Render the PR body from the template (or a provided body file).
render_body() {
  if [[ -n "$BODY_FILE" ]]; then
    if [[ ! -f "$BODY_FILE" ]]; then
      echo "error: body file not found: $BODY_FILE" >&2
      exit 1
    fi
    cat "$BODY_FILE"
  else
    jq -Rsr \
      --arg pr_title "$pr_title" \
      --arg issue_number "$issue_number" \
      --arg plan_path "$plan_path" \
      --arg outcome "$outcome" \
      --arg diff_stat "$diff_stat" \
      --arg manual_checks "$manual_checks" \
      --arg acceptance_criteria "$acceptance" \
      'gsub("\\{\\{PR_TITLE\\}\\}"; $pr_title)
       | gsub("\\{\\{ISSUE_NUMBER\\}\\}"; $issue_number)
       | gsub("\\{\\{PLAN_PATH\\}\\}"; $plan_path)
       | gsub("\\{\\{OUTCOME\\}\\}"; $outcome)
       | gsub("\\{\\{DIFF_STAT\\}\\}"; $diff_stat)
       | gsub("\\{\\{MANUAL_CHECKS\\}\\}"; $manual_checks)
       | gsub("\\{\\{ACCEPTANCE_CRITERIA\\}\\}"; $acceptance_criteria)' \
      "$TEMPLATE"
  fi
}
body="$(render_body)"

if [[ "$MODE" == "dry-run" ]]; then
  echo "--- planned commands ---"
  run git push -u origin "$branch"
  run gh pr create --repo "$REPO" --base "$BASE" --head "$branch" --title "$pr_title" --body "<rendered from template>"
  echo "--- PR title ---"
  printf '%s\n' "$pr_title"
  echo "--- PR body ---"
  printf '%s\n' "$body"
  exit 0
fi

# Publish the branch.
run git push -u origin "$branch"

# Reuse an existing PR for this branch instead of creating a duplicate.
existing="$(gh pr list --repo "$REPO" --head "$branch" --state all --json url,number -q '.[0] // empty' 2>/dev/null || true)"
if [[ -n "$existing" ]]; then
  status="exists"
  url="$(printf '%s' "$existing" | jq -r '.url')"
  warn "a pull request already exists for $branch; returning it"
else
  status="created"
  url="$(gh pr create --repo "$REPO" --base "$BASE" --head "$branch" --title "$pr_title" --body "$body")"
fi

if [[ "$JSON_OUT" == "yes" ]]; then
  jq -n --arg status "$status" --arg url "$url" --arg title "$pr_title" \
    '{status: $status, url: $url, title: $title}'
else
  echo "status: $status"
  echo "pr: $url"
fi
