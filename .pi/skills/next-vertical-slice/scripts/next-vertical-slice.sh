#!/usr/bin/env bash
#
# next-vertical-slice.sh — Report the next vertical slice plan to implement.
#
# The project tracks vertical slice implementation plans as GitHub issues
# titled `[VS-NNN] - Title` and labeled `ai:vertical-slice` (see
# .opencode/skills/create-vertical-slice-issue/SKILL.md). Each issue body
# carries the plan document location (`## Plan`) plus the slice's scope and
# acceptance criteria. Slices must be implemented in order
# (docs/implementation-plans/00-index.md), so the "next" slice is the
# lowest-numbered [VS-NNN] issue that is still open; a closed issue counts
# as implemented.
#
# Designed for agent skills: the output is minimal and structured.
#
# Human output (default):
#   status: next
#   plan: docs/implementation-plans/02-startup-model-readiness.md
#   issue: https://github.com/TheCamilovisk/Transclip/issues/1
#
# When every slice is already implemented, the script reports
# `status: done` with a message and exits 0 — that is not an error.
#
# Requirements: gh (authenticated against the repository) and jq.
#
# Usage:
#   scripts/next-vertical-slice.sh [--json] [--repo OWNER/NAME]
#
#   --json   Emit a single JSON object:
#            {"status":"next","plan":"...","issue":"..."}
#            or {"status":"done","message":"..."}.
#   --repo   Repository to query (default: the one of the current directory).
#
# Exit status:
#   0  a next slice was reported, or all slices are already implemented
#   1  GitHub query, repository resolution, or dependency failure
#   2  usage error
set -euo pipefail

MODE="human"
REPO=""

usage() {
  sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --json) MODE="json" ;;
    --repo) REPO="${2:-}"; shift ;;
    -h|--help) usage 0 ;;
    *) echo "error: unknown option: $1" >&2; usage 2 ;;
  esac
  shift
done

for cmd in gh jq; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: '$cmd' is required but not installed" >&2
    exit 1
  fi
done

# Resolve the repository to query.
if [[ -z "$REPO" ]]; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
  if [[ -z "$REPO" ]]; then
    echo "error: could not determine the repository; run from inside the repo or pass --repo OWNER/NAME" >&2
    exit 1
  fi
fi

# Fetch every issue carrying the vertical-slice label, open and closed.
if ! issues_json="$(gh issue list --repo "$REPO" --label ai:vertical-slice \
  --state all --json number,title,state,url,body --limit 1000)"; then
  echo "error: failed to list issues for $REPO (is gh authenticated and does the repository exist?)" >&2
  exit 1
fi

# Normalize the issues: parse the [VS-NNN] identifier, extract the plan path
# from the body, mark closed issues as done, and sort by slice order.
parsed="$(printf '%s' "$issues_json" | jq -c '
  def vsnum:
    try (.title | capture("\\[VS-(?<vs>[0-9]+)\\]") | .vs | tonumber) catch null;
  def planPath:
    .body as $b
    | if $b == null then null
      else ($b | split("\n")) as $lines
        | (($lines | map(select(test("^## (Vertical Slice )?Plan$"))) | first) as $hdr
          | if $hdr == null then null
            else ($lines | index($hdr) + 1) as $i
              | (($lines[$i] // "") | gsub("^[` ]+|[` ]+$"; "") | if . == "" then null else . end)
            end)
      end;
  [ .[]
    | select(vsnum != null)
    | { vs: vsnum,
        number, title, state, url, body,
        done: (.state == "CLOSED"),
        planPath: planPath } ]
  | sort_by(.vs)
')"

skipped="$(printf '%s' "$issues_json" | jq -r '[.[] | select((.title | test("\\[VS-[0-9]+\\]")) | not)] | length')"
if [[ "$skipped" -gt 0 ]]; then
  echo "note: $skipped issue(s) labeled 'ai:vertical-slice' have no [VS-NNN] title and were ignored." >&2
fi

# The next slice is the first not-done issue in ascending slice order.
next="$(printf '%s' "$parsed" | jq -c '[.[] | select(.done == false)] | .[0] // null')"

# No remaining slice: not an error — everything is already implemented.
if [[ "$next" == "null" ]]; then
  message="All vertical slice plans are already implemented. There is no next slice."
  if [[ "$MODE" == "json" ]]; then
    jq -n --arg m "$message" '{status: "done", message: $m}'
  else
    echo "status: done"
    echo "message: $message"
  fi
  exit 0
fi

plan="$(printf '%s' "$next" | jq -r '.planPath // ""')"
issue="$(printf '%s' "$next" | jq -r '.url')"

if [[ "$MODE" == "json" ]]; then
  jq -n --arg p "$plan" --arg i "$issue" '{status: "next", plan: $p, issue: $i}'
else
  echo "status: next"
  echo "plan: $plan"
  echo "issue: $issue"
fi
