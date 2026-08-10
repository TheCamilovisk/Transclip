#!/usr/bin/env bash
#
# resolve-slice-issue.sh — Resolve the GitHub issue for a vertical slice.
#
# Given the zero-padded slice number from a branch name (e.g. "002" for
# `vs-002_terminal-state-machine`), find the issue whose title is
# `[VS-002] - ...` and print its actual issue number.
#
# The issue number is looked up, never assumed: GitHub issues and pull
# requests share one number space, so a slice's issue number can differ from
# its `VS-NNN` number. The publish skill uses the resolved number for the PR
# body's `Closes #<issue>` line so the right issue is closed on merge.
#
# Usage:
#   resolve-slice-issue.sh [--json] [--repo OWNER/NAME] SLICE_NUMBER
#
#   --json       Emit {"number":N,"title":"..."} instead of just the number.
#   --repo       Repository to query (default: the current directory's).
#   SLICE_NUMBER Slice number, with or without zero padding (e.g. 2 or 002).
#
# Exit status:
#   0  exactly one matching issue found
#   1  no match, ambiguous match, or GitHub/CLI failure
#   2  usage error
set -euo pipefail

MODE="number"
REPO=""
SLICE=""

usage() {
  sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --json) MODE="json" ;;
    --repo) REPO="${2:-}"; shift ;;
    -h|--help) usage 0 ;;
    -*) echo "error: unknown option: $1" >&2; usage 2 ;;
    *) SLICE="$1" ;;
  esac
  shift
done

for cmd in gh jq; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: '$cmd' is required but not installed" >&2
    exit 1
  fi
done

if [[ -z "$SLICE" ]]; then
  echo "error: provide SLICE_NUMBER (e.g. 2 or 002)" >&2
  usage 2
fi
if [[ ! "$SLICE" =~ ^[0-9]{1,3}$ ]]; then
  echo "error: invalid slice number: $SLICE (expected 1-3 digits)" >&2
  exit 2
fi
pad="$(printf '%03d' "$((10#$SLICE))")"

if [[ -z "$REPO" ]]; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
  if [[ -z "$REPO" ]]; then
    echo "error: could not determine the repository; run from inside the repo or pass --repo OWNER/NAME" >&2
    exit 1
  fi
fi

prefix="[VS-$pad]"
matches="$(gh issue list --repo "$REPO" --label "ai:vertical-slice" --state all --limit 100 \
  --json number,title \
  | jq -c --arg prefix "$prefix" \
    '[.[] | select(.title | test("^\\[VS-[0-9]{3}\\]")) | select(.title | startswith($prefix))]')"
count="$(printf '%s' "$matches" | jq 'length')"

if [[ "$count" -eq 0 ]]; then
  echo "error: no issue titled '$prefix - ...' (label ai:vertical-slice) found in $REPO" >&2
  exit 1
fi
if [[ "$count" -gt 1 ]]; then
  numbers="$(printf '%s' "$matches" | jq -r '[.[].number] | join(", ")')"
  echo "error: multiple issues match '$prefix - ...': $numbers" >&2
  exit 1
fi

number="$(printf '%s' "$matches" | jq -r '.[0].number')"
title="$(printf '%s' "$matches" | jq -r '.[0].title')"

if [[ "$MODE" == "json" ]]; then
  jq -n --arg number "$number" --arg title "$title" '{number: $number, title: $title}'
else
  printf '%s\n' "$number"
fi
