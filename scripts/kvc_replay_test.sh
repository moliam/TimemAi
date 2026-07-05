#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/timem-kvc-replay.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

EMPTY_OUTPUT="$(python3 scripts/kvc_replay.py \
  --data-dir "$TMP_DIR/missing" \
  --max-tail-blocks 1 \
  --max-threshold 1 \
  --max-checkpoints 1)"

case "$EMPTY_OUTPUT" in
  *"audit_files: 0"*$'\n'*"llm_requests: 0"*) ;;
  *)
    echo "kvc_replay_test: explicit --data-dir unexpectedly scanned other data" >&2
    echo "$EMPTY_OUTPUT" >&2
    exit 1
    ;;
esac

mkdir -p "$TMP_DIR/data/.space/audit"
cat >"$TMP_DIR/data/.space/audit/api_audit.jsonl" <<'JSONL'
{"type":"llm_request","provider":"fixture-json-protocol","model":"m","body":{"system":[{"type":"text","text":"STATIC JSON RESPONSE PROTOCOL\n{\"fields\":{\"status?\":\"...\",\"next_actions?\":\"...\"}}"}],"messages":[{"role":"user","content":[{"type":"text","text":"[BEGIN DELTA]\ndelta_id: pd_json_1\n\n## USER\njson hello\n[END DELTA]"}]}]}}
{"type":"llm_request","provider":"fixture-json-protocol","model":"m","body":{"system":[{"type":"text","text":"STATIC JSON RESPONSE PROTOCOL\n{\"fields\":{\"status?\":\"...\",\"next_actions?\":\"...\"}}"}],"messages":[{"role":"user","content":[{"type":"text","text":"[BEGIN DELTA]\ndelta_id: pd_json_1\n\n## USER\njson hello\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_json_2\n\n## TIMEM_ASSISTANT\nfinal_answer:\njson hi\n[END DELTA]"}]}]}}
{"type":"llm_request","provider":"fixture-json-protocol","model":"m","body":{"system":[{"type":"text","text":"STATIC JSON RESPONSE PROTOCOL\n{\"fields\":{\"status?\":\"...\",\"next_actions?\":\"...\"}}"}],"messages":[{"role":"user","content":[{"type":"text","text":"[BEGIN DELTA]\ndelta_id: pd_json_1\n\n## USER\njson hello\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_json_2\n\n## TIMEM_ASSISTANT\nfinal_answer:\njson hi\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_json_3\n\n## USER\njson follow up\n[END DELTA]"}]}]}}
{"type":"llm_request","provider":"fixture-markdown-protocol","model":"m","body":{"instructions":"STATIC MARKDOWN RESPONSE PROTOCOL\n## Response Protocol\nThe top-level response is Markdown, not JSON.","input":"[BEGIN DELTA]\ndelta_id: pd_md_1\n\n## USER\nmarkdown hello\n[END DELTA]"}}
{"type":"llm_request","provider":"fixture-markdown-protocol","model":"m","body":{"instructions":"STATIC MARKDOWN RESPONSE PROTOCOL\n## Response Protocol\nThe top-level response is Markdown, not JSON.","input":"[BEGIN DELTA]\ndelta_id: pd_md_1\n\n## USER\nmarkdown hello\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_md_2\n\n## TIMEM_ASSISTANT\nfinal_answer:\nmarkdown hi\n[END DELTA]"}}
{"type":"llm_request","provider":"fixture-markdown-protocol","model":"m","body":{"instructions":"STATIC MARKDOWN RESPONSE PROTOCOL\n## Response Protocol\nThe top-level response is Markdown, not JSON.","input":"[BEGIN DELTA]\ndelta_id: pd_md_1\n\n## USER\nmarkdown hello\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_md_2\n\n## TIMEM_ASSISTANT\nfinal_answer:\nmarkdown hi\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_md_3\n\n## USER\nmarkdown follow up\n[END DELTA]"}}
JSONL

OUTPUT="$(python3 scripts/kvc_replay.py \
  --data-dir "$TMP_DIR/data" \
  --max-tail-blocks 1 \
  --max-threshold 1 \
  --max-checkpoints 1)"

case "$OUTPUT" in
  *"audit_files: 1"*$'\n'*"llm_requests: 6"* ) ;;
  *)
    echo "kvc_replay_test: fixture replay did not count expected requests" >&2
    echo "$OUTPUT" >&2
    exit 1
    ;;
esac

TAIL_HIT_RATE="$(printf '%s\n' "$OUTPUT" | awk '$1 == "tail" && $2 == "tail=1" {gsub("%", "", $5); print $5; exit}')"
if [[ -z "$TAIL_HIT_RATE" ]]; then
  echo "kvc_replay_test: missing tail=1 replay row" >&2
  echo "$OUTPUT" >&2
  exit 1
fi

python3 - "$TAIL_HIT_RATE" <<'PY'
import sys
hit = float(sys.argv[1])
if hit <= 0.0:
    raise SystemExit(f"kvc_replay_test: expected cache read hits for replay fixture, got {hit}%")
PY

echo "kvc_replay_test: ok"
