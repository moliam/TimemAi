#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:---current}"

cd "$ROOT_DIR"

case "$MODE" in
  --current|--history) ;;
  -h|--help)
    echo "Usage: scripts/sensitive_scan.sh [--current|--history]"
    echo
    echo "  --current  scan tracked files at HEAD"
    echo "  --history  scan all reachable git history"
    exit 0
    ;;
  *)
    echo "error: unknown mode: $MODE" >&2
    exit 2
    ;;
esac

private_markers=(
  "c""hj"
  "che""hejia"
  "inner.""c""hj"
  "c""hj"".cloud"
  "llm-""gateway""-proxy"
  "api-""hub.inner"
  "X-""C""HJ"
  "BCS-""APIHub"
)

secret_regexes=(
  'sk-[A-Za-z0-9_-]{20,}'
  'Bearer[[:space:]]+[A-Za-z0-9._-]{20,}'
  'eyJ[A-Za-z0-9_-]{40,}\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}'
)

scan_current() {
  local failed=0
  local files
  files="$(git ls-files | grep -Ev '(^target/|^data/|^env$|^Cargo.lock$)' || true)"
  if [ -z "$files" ]; then
    echo "No tracked files to scan."
    return 0
  fi

  for marker in "${private_markers[@]}"; do
    if printf '%s\n' "$files" | xargs rg -n -i -F -- "$marker" >/tmp/timem_sensitive_hits.$$ 2>/dev/null; then
      echo "private marker found in current tree: $marker" >&2
      cat /tmp/timem_sensitive_hits.$$ >&2
      failed=1
    fi
  done

  for regex in "${secret_regexes[@]}"; do
    if printf '%s\n' "$files" | xargs rg -n --pcre2 -- "$regex" >/tmp/timem_sensitive_hits.$$ 2>/dev/null; then
      echo "secret-like token found in current tree: $regex" >&2
      cat /tmp/timem_sensitive_hits.$$ >&2
      failed=1
    fi
  done

  rm -f /tmp/timem_sensitive_hits.$$
  if [ "$failed" -ne 0 ]; then
    return 1
  fi
  echo "sensitive_scan current: ok"
}

scan_history() {
  local failed=0
  for marker in "${private_markers[@]}"; do
    if git grep -n -i -F -- "$marker" "$(git rev-list --all)" >/tmp/timem_sensitive_history_hits.$$ 2>/dev/null; then
      echo "private marker found in git history: $marker" >&2
      cat /tmp/timem_sensitive_history_hits.$$ >&2
      failed=1
    fi
  done
  rm -f /tmp/timem_sensitive_history_hits.$$
  if [ "$failed" -ne 0 ]; then
    return 1
  fi
  echo "sensitive_scan history: ok"
}

if [ "$MODE" = "--history" ]; then
  scan_history
else
  scan_current
fi
