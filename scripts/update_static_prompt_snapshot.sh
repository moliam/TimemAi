#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OUT_MARKDOWN="resources/protocol/markdown/expanded.md"
OUT_JSON="resources/protocol/json/expanded.md"
TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

render_one() {
  local protocol="$1"
  local out="$2"
  TIMEM_RESPONSE_PROTOCOL="$protocol" cargo run -q -p agent_core --example expand_static_prompt > "$out"
}

TMP_MARKDOWN="$TMP_DIR/markdown.md"
TMP_JSON="$TMP_DIR/json.md"
render_one markdown "$TMP_MARKDOWN"
render_one json "$TMP_JSON"

if [ "${1:-}" = "--check" ]; then
  failed=0
  for pair in "$TMP_MARKDOWN:$OUT_MARKDOWN" "$TMP_JSON:$OUT_JSON"; do
    tmp="${pair%%:*}"
    out="${pair#*:}"
    if ! cmp -s "$tmp" "$out"; then
      echo "expanded static prompt snapshot is stale: $out" >&2
      diff -u "$out" "$tmp" >&2 || true
      failed=1
    fi
  done
  if [ "$failed" -ne 0 ]; then
    echo "run: scripts/update_static_prompt_snapshot.sh" >&2
    exit 1
  fi
  echo "static_prompt_snapshot: ok"
else
  cp "$TMP_MARKDOWN" "$OUT_MARKDOWN"
  cp "$TMP_JSON" "$OUT_JSON"
  trap - EXIT
  cleanup
  echo "updated $OUT_MARKDOWN"
  echo "updated $OUT_JSON"
fi
