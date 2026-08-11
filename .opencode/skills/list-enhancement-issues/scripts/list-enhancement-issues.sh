#!/usr/bin/env bash
# List open GitHub issues labeled enhancement as Markdown links.
set -euo pipefail

REPO=""

usage() {
  cat <<'EOF'
Usage: list-enhancement-issues.sh [--repo OWNER/NAME]

Lists open GitHub issues labeled enhancement, formatting each as a Markdown
link containing its title and URL. The repository defaults to the current one.
EOF
  exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) REPO="${2:-}"; shift ;;
    -h|--help) usage 0 ;;
    -*) echo "error: unknown option: $1" >&2; usage 2 ;;
    *) echo "error: unexpected argument: $1" >&2; usage 2 ;;
  esac
  shift
done

if ! command -v gh >/dev/null 2>&1; then
  echo "error: 'gh' is required but not installed" >&2
  exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
  echo "error: GitHub CLI is not authenticated" >&2
  exit 1
fi

if [[ -z "$REPO" ]]; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
  if [[ -z "$REPO" ]]; then
    echo "error: could not determine the repository; run from inside the repo or pass --repo OWNER/NAME" >&2
    exit 1
  fi
fi

gh issue list --repo "$REPO" --label enhancement --state open --limit 100 --json title,url \
  --jq '.[] | "- [\(.title)](\(.url))"'
