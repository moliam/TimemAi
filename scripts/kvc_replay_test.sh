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
{"type":"llm_request","provider":"test","model":"m","body":{"system":[{"type":"text","text":"STATIC"}],"messages":[{"role":"user","content":[{"type":"text","text":"[BEGIN SEGMENT 1: prompt_delta]\ndelta_id: pd_1\nprompt_type: user_question\nhello\n[END SEGMENT 1: prompt_delta]"}]}]}}
{"type":"llm_request","provider":"test","model":"m","body":{"system":[{"type":"text","text":"STATIC"}],"messages":[{"role":"user","content":[{"type":"text","text":"[BEGIN SEGMENT 1: prompt_delta]\ndelta_id: pd_1\nprompt_type: user_question\nhello\n[END SEGMENT 1: prompt_delta]\n[BEGIN SEGMENT 2: prompt_delta]\ndelta_id: pd_2\nprompt_type: llm_response\nhi\n[END SEGMENT 2: prompt_delta]"}]}]}}
JSONL

OUTPUT="$(python3 scripts/kvc_replay.py \
  --data-dir "$TMP_DIR/data" \
  --max-tail-blocks 1 \
  --max-threshold 1 \
  --max-checkpoints 1)"

case "$OUTPUT" in
  *"audit_files: 1"*$'\n'*"llm_requests: 2"* ) ;;
  *)
    echo "kvc_replay_test: fixture replay did not count expected requests" >&2
    echo "$OUTPUT" >&2
    exit 1
    ;;
esac

echo "kvc_replay_test: ok"
