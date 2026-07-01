#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OUT="docs/static-prompt-expanded.md"
TMP="$(mktemp)"
cleanup() {
  rm -f "$TMP"
}
trap cleanup EXIT

cargo run -q -p agent_core --example expand_static_prompt > "$TMP"

if [ "${1:-}" = "--check" ]; then
  if ! cmp -s "$TMP" "$OUT"; then
    echo "expanded static prompt snapshot is stale: $OUT" >&2
    echo "run: scripts/update_static_prompt_snapshot.sh" >&2
    diff -u "$OUT" "$TMP" >&2 || true
    exit 1
  fi
  echo "static_prompt_snapshot: ok"
else
  mv "$TMP" "$OUT"
  trap - EXIT
  echo "updated $OUT"
fi
