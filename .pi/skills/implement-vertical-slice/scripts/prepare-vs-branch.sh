#!/usr/bin/env bash
#
# prepare-vs-branch.sh — Prepare the dev branch and create the vertical-slice branch.
#
# Ensures a local `dev` branch exists and is in sync with the latest changes,
# then creates (and checks out) a vertical-slice branch named
# `vs-XXX_kebab-case-title`, derived from the slice issue title `[VS-NNN] - Title`
# (e.g. `[VS-001] - Startup and Model Readiness` -> `vs-001_startup-and-model-readiness`).
#
# Dev-branch policy:
#   - If `dev` does not exist, it is created from `origin/dev` when present,
#     otherwise from `main`.
#   - If `dev` is behind `origin/dev`, it is fast-forwarded.
#   - If `dev` has unpushed commits, they are pushed.
#   - If `dev` and `origin/dev` have diverged, the script stops for manual resolution.
#
# Usage:
#   prepare-vs-branch.sh [--dry-run] [--repo OWNER/NAME] [--title 'TITLE'] ISSUE_NUMBER
#
#   --dry-run   Print the derived branch name and planned git commands without
#               changing anything.
#   --repo      Repository to query (default: the one of the current directory).
#   --title     Use this issue title instead of fetching it (mainly for testing).
#
# Exit status:
#   0  branch prepared and checked out
#   1  GitHub query, repository, or git failure
#   2  usage error
set -euo pipefail

MODE="run"
REPO=""
ISSUE=""
TITLE_OVERRIDE=""

usage() {
  sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) MODE="dry-run" ;;
    --repo) REPO="${2:-}"; shift ;;
    --title) TITLE_OVERRIDE="${2:-}"; shift ;;
    -h|--help) usage 0 ;;
    -*) echo "error: unknown option: $1" >&2; usage 2 ;;
    *) ISSUE="$1" ;;
  esac
  shift
done

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

# Fetch the issue title (or use the override).
if [[ -n "$TITLE_OVERRIDE" ]]; then
  title="$TITLE_OVERRIDE"
elif [[ -n "$ISSUE" ]]; then
  title="$(gh issue view "$ISSUE" --repo "$REPO" --json title -q .title)" || {
    echo "error: failed to fetch issue $ISSUE in $REPO (is gh authenticated?)" >&2
    exit 1
  }
else
  echo "error: provide ISSUE_NUMBER or --title 'TITLE'" >&2
  usage 2
fi

# Derive `XXX` (zero-padded) and the kebab-case title from `[VS-NNN] - Title`.
vs="$(printf '%s' "$title" | sed -nE 's/.*\[VS-([0-9]+)\].*/\1/p')"
if [[ -z "$vs" ]]; then
  echo "error: title has no [VS-NNN] identifier: $title" >&2
  exit 1
fi
kebab="$(printf '%s' "$title" \
  | sed -E 's/\[VS-[0-9]+\][[:space:]-]*//' \
  | tr '[:upper:]' '[:lower:]' \
  | sed -E 's/[^a-z0-9]+/-/g; s/^-+|-+$//g')"
if [[ -z "$kebab" ]]; then
  echo "error: could not derive a kebab-case title from: $title" >&2
  exit 1
fi
branch="vs-$(printf '%03d' "$vs")_$kebab"
echo "issue title: $title"
echo "branch name: $branch"

# Ensure the local `dev` branch exists and is in sync with the latest changes.
run git fetch origin --prune
if git rev-parse --verify -q refs/heads/dev >/dev/null; then
  run git checkout -q dev
  if git rev-parse --verify -q refs/remotes/origin/dev >/dev/null; then
    if [[ "$(git rev-parse dev)" != "$(git rev-parse origin/dev)" ]]; then
      if git merge-base --is-ancestor dev origin/dev; then
        warn "local dev is behind origin/dev; fast-forwarding"
        run git merge --ff-only origin/dev
      elif git merge-base --is-ancestor origin/dev dev; then
        warn "local dev is ahead of origin/dev by $(git rev-list --count origin/dev..dev) commit(s)"
      else
        echo "error: dev and origin/dev have diverged; resolve manually before creating the slice branch" >&2
        exit 1
      fi
    fi
  fi
else
  if git rev-parse --verify -q refs/remotes/origin/dev >/dev/null; then
    run git checkout -q -b dev origin/dev
  else
    warn "no origin/dev exists; creating dev from local main"
    run git checkout -q -b dev main
  fi
fi

# Publish dev when it is new or strictly ahead of origin/dev.
if git rev-parse --verify -q refs/remotes/origin/dev >/dev/null; then
  if [[ "$(git rev-parse dev)" != "$(git rev-parse origin/dev)" ]] \
     && git merge-base --is-ancestor origin/dev dev; then
    run git push -u origin dev
  fi
else
  run git push -u origin dev
fi

# Create the vertical-slice branch from dev.
if git rev-parse --verify -q "refs/heads/$branch" >/dev/null; then
  echo "error: branch '$branch' already exists; refusing to clobber it" >&2
  exit 1
fi
run git checkout -q -b "$branch" dev

echo "ok: on branch '$branch' at $(git rev-parse --short HEAD), derived from dev"
