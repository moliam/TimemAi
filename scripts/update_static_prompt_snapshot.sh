#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OUT_DIR="${TIMEM_EXPANDED_PROMPT_DIR:-target/static-prompt-expanded}"
OUT_JSON="$OUT_DIR/json.md"
OUT_XML="$OUT_DIR/xml.md"
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

validate_one() {
  local protocol="$1"
  local out="$2"
  shift 2
  local required=(
    '#### `run_bash`'
    '#### `memmgr`'
    "**Usage**"
    "**Result**"
  )
  local forbidden=(
    "Runtime info:"
    "{{RT_ENV}}"
    "cwd:"
    "$HOME"
    "\"output\": {"
    "Background job id when background=true."
    "\"output_file\""
    "\"status_file\""
    "\"approval_status\""
    "\"static_prefix_policy\""
    "static prefix is immutable global guidance"
    "\"ui_status\""
    "ui_label"
    "ui_visible"
  )
  for pattern in "${required[@]}" "$@"; do
    if ! grep -Fq "$pattern" "$out"; then
      echo "missing required expanded static prompt item in $protocol: $pattern" >&2
      return 1
    fi
  done
  for pattern in "${forbidden[@]}"; do
    if grep -Fq "$pattern" "$out"; then
      echo "forbidden expanded static prompt item in $protocol: $pattern" >&2
      return 1
    fi
  done
}

TMP_JSON="$TMP_DIR/json.md"
TMP_XML="$TMP_DIR/xml.md"
render_one json "$TMP_JSON"
render_one xml "$TMP_XML"

validate_one json "$TMP_JSON" \
  "[BEGIN SYSTEM PROMPT]" \
  "[END SYSTEM PROMPT]" \
  '"run_bash":' \
  '"cmd":'
validate_one xml "$TMP_XML" \
  "<Timem System Prompt>" \
  "</Timem System Prompt>" \
  "# ASSISTANT Response Protocol" \
  'ONE VALID XML `<ASSISTANT>` ROOT' \
  '`<ASSISTANT>`' \
  '`<final_answer>`' \
  '<actions>' \
  '<parallel>' \
  '<run_bash name="check git status" timeout_ms="5000">' \
  '<cmd>git status</cmd>'

if [ "${1:-}" = "--check" ]; then
  echo "static_prompt_expansion: ok"
else
  mkdir -p "$OUT_DIR"
  cp "$TMP_JSON" "$OUT_JSON"
  cp "$TMP_XML" "$OUT_XML"
  trap - EXIT
  cleanup
  echo "generated $OUT_JSON"
  echo "generated $OUT_XML"
fi
