#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:---current}"

cd "$ROOT_DIR"

case "$MODE" in
  --current|--history|--self-test) ;;
  -h|--help)
    echo "Usage: scripts/sensitive_scan.sh [--current|--history|--self-test]"
    echo
    echo "  --current  scan tracked files at HEAD"
    echo "  --history  scan all reachable git history"
    echo "  --self-test verify text matches are detected and binary matches are ignored"
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
  files="$(git ls-files | grep -Ev '(^target/|^data/|^env$|^Cargo.lock$|(^|/)pnpm-lock\.yaml$)' || true)"
  if [ -z "$files" ]; then
    echo "No tracked files to scan."
    return 0
  fi

  for marker in "${private_markers[@]}"; do
    scan_cmd=(grep -n -i -I -F -- "$marker")
    if printf '%s\n' "$files" | xargs "${scan_cmd[@]}" >/tmp/timem_sensitive_hits.$$ 2>/dev/null; then
      echo "private marker found in current tree: $marker" >&2
      cat /tmp/timem_sensitive_hits.$$ >&2
      failed=1
    fi
  done

  for regex in "${secret_regexes[@]}"; do
    scan_cmd=(grep -n -I -E -- "$regex")
    if printf '%s\n' "$files" | xargs "${scan_cmd[@]}" >/tmp/timem_sensitive_hits.$$ 2>/dev/null; then
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
    if git grep -n -i -I -F -- "$marker" "$(git rev-list --all)" >/tmp/timem_sensitive_history_hits.$$ 2>/dev/null; then
      echo "private marker found in git history: $marker" >&2
      cat /tmp/timem_sensitive_history_hits.$$ >&2
      failed=1
    fi
  done

  for regex in "${secret_regexes[@]}"; do
    if git grep -n -I --perl-regexp -- "$regex" "$(git rev-list --all)" >/tmp/timem_sensitive_history_hits.$$ 2>/dev/null; then
      echo "secret-like token found in git history: $regex" >&2
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

self_test() (
  local test_root
  local text_file
  local binary_file
  local marker="${private_markers[0]}"
  local regex_sample="s""k-12345678901234567890"
  test_root="$(mktemp -d "${TMPDIR:-/tmp}/timem-sensitive-scan.XXXXXX")"
  trap 'rm -rf "$test_root"' EXIT
  text_file="$test_root/text.txt"
  binary_file="$test_root/font.woff"

  printf 'prefix %s suffix\n%s\n' "$marker" "$regex_sample" >"$text_file"
  printf '\0prefix %s suffix\n%s\n' "$marker" "$regex_sample" >"$binary_file"

  grep -n -i -I -F -- "$marker" "$text_file" >/dev/null
  ! grep -n -i -I -F -- "$marker" "$binary_file" >/dev/null
  grep -n -I -E -- "${secret_regexes[0]}" "$text_file" >/dev/null
  ! grep -n -I -E -- "${secret_regexes[0]}" "$binary_file" >/dev/null

  echo "sensitive_scan self-test: ok"
)

case "$MODE" in
  --history) scan_history ;;
  --self-test) self_test ;;
  *) scan_current ;;
esac
