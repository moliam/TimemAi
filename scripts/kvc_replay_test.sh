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

mkdir -p "$TMP_DIR/data/.native/audit/api_audit.jsonl.segments"
cat >"$TMP_DIR/data/.native/audit/api_audit.jsonl.segments/manifest.json" <<'JSON'
{"version":1,"next_index":3,"segments":[{"file_name":"segment-0000000000000001.jsonl","bytes":1},{"file_name":"segment-0000000000000002.jsonl","bytes":1}]}
JSON
cat >"$TMP_DIR/data/.native/audit/api_audit.jsonl.segments/segment-0000000000000001.jsonl" <<'JSONL'
{"type":"llm_request","time_ms":100,"audit_request_id":"native_1","endpoint":"https://example.invalid/v1/responses","model":"native-model","body":{"instructions":"STATIC NATIVE PROMPT\nCurrent session_id: session_fixture\nCurrent context_id: context_fixture\nCurrent worker_id: worker_fixture","tools":[{"type":"function","name":"self_tool","parameters":{"type":"object"}}],"input":[{"role":"user","content":[{"type":"input_text","text":"[BEGIN DELTA delta_id: pd_native_1]\n\n## USER\nnative hello"}]},{"role":"assistant","content":[{"type":"output_text","text":"checking"}]},{"type":"function_call","call_id":"call_1","name":"self_tool","arguments":"{\"type\":\"cwd\"}"},{"type":"function_call_output","call_id":"call_1","output":"CWD: /tmp"}]}}
{"type":"llm_response","time_ms":110,"audit_request_id":"native_1","body":{"usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":0},"output_tokens":10,"total_tokens":1010}}}
JSONL
cat >"$TMP_DIR/data/.native/audit/api_audit.jsonl.segments/segment-0000000000000002.jsonl" <<'JSONL'
{"type":"llm_request","time_ms":200,"audit_request_id":"native_2","endpoint":"https://example.invalid/v1/responses","model":"native-model","body":{"instructions":"STATIC NATIVE PROMPT\nCurrent session_id: session_fixture\nCurrent context_id: context_fixture\nCurrent worker_id: worker_fixture","tools":[{"type":"function","name":"self_tool","parameters":{"type":"object"}}],"input":[{"role":"user","content":[{"type":"input_text","text":"[BEGIN DELTA delta_id: pd_native_1]\n\n## USER\nnative hello"}]},{"role":"assistant","content":[{"type":"output_text","text":"checking"}]},{"type":"function_call","call_id":"call_1","name":"self_tool","arguments":"{\"type\":\"cwd\"}"},{"type":"function_call_output","call_id":"call_1","output":"CWD: /tmp"},{"role":"user","content":[{"type":"input_text","text":"[BEGIN DELTA delta_id: pd_native_2]\n\n## USER\ncontinue"}]}]}}
{"type":"llm_response","time_ms":210,"audit_request_id":"native_2","body":{"usage":{"input_tokens":1200,"input_tokens_details":{"cached_tokens":1000},"output_tokens":20,"total_tokens":1220}}}
JSONL


mkdir -p "$TMP_DIR/data/.space/audit"
cat >"$TMP_DIR/data/.space/audit/api_audit.jsonl" <<'JSONL'
{"type":"llm_request","model":"m","body":{"system":[{"type":"text","text":"STATIC JSON RESPONSE PROTOCOL\n{\"fields\":{\"status?\":\"...\",\"next_actions?\":\"...\"}}"}],"messages":[{"role":"user","content":[{"type":"text","text":"[BEGIN DELTA]\ndelta_id: pd_json_1\n\n## USER\njson hello\n[END DELTA]"}]}]}}
{"type":"llm_request","model":"m","body":{"system":[{"type":"text","text":"STATIC JSON RESPONSE PROTOCOL\n{\"fields\":{\"status?\":\"...\",\"next_actions?\":\"...\"}}"}],"messages":[{"role":"user","content":[{"type":"text","text":"[BEGIN DELTA]\ndelta_id: pd_json_1\n\n## USER\njson hello\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_json_2\n\n## TIMEM_ASSISTANT\nfinal_answer:\njson hi\n[END DELTA]"}]}]}}
{"type":"llm_request","model":"m","body":{"system":[{"type":"text","text":"STATIC JSON RESPONSE PROTOCOL\n{\"fields\":{\"status?\":\"...\",\"next_actions?\":\"...\"}}"}],"messages":[{"role":"user","content":[{"type":"text","text":"[BEGIN DELTA]\ndelta_id: pd_json_1\n\n## USER\njson hello\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_json_2\n\n## TIMEM_ASSISTANT\nfinal_answer:\njson hi\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_json_3\n\n## USER\njson follow up\n[END DELTA]"}]}]}}
{"type":"llm_request","model":"m","body":{"instructions":"STATIC XML RESPONSE PROTOCOL\n## Response Protocol\nThe top-level response is XML, not JSON or Markdown.","input":"<prompt_delta id=\"pd_xml_1\" time_ms=\"1\">\n<USER>\nxml hello\n</USER>\n</prompt_delta>"}}
{"type":"llm_request","model":"m","body":{"instructions":"STATIC XML RESPONSE PROTOCOL\n## Response Protocol\nThe top-level response is XML, not JSON or Markdown.","input":"<prompt_delta id=\"pd_xml_1\" time_ms=\"1\">\n<USER>\nxml hello\n</USER>\n</prompt_delta>\n<prompt_delta id=\"pd_xml_2\" time_ms=\"2\">\n<ASSISTANT>\nfinal_answer:\nxml hi\n</ASSISTANT>\n</prompt_delta>"}}
{"type":"llm_request","model":"m","body":{"instructions":"STATIC XML RESPONSE PROTOCOL\n## Response Protocol\nThe top-level response is XML, not JSON or Markdown.","input":"<prompt_delta id=\"pd_xml_1\" time_ms=\"1\">\n<USER>\nxml hello\n</USER>\n</prompt_delta>\n<prompt_delta id=\"pd_xml_2\" time_ms=\"2\">\n<ASSISTANT>\nfinal_answer:\nxml hi\n</ASSISTANT>\n</prompt_delta>\n<prompt_delta id=\"pd_xml_3\" time_ms=\"3\">\n<USER>\nxml follow up\n</USER>\n</prompt_delta>"}}
JSONL

OUTPUT="$(python3 scripts/kvc_replay.py \
  --data-dir "$TMP_DIR/data" \
  --max-tail-blocks 1 \
  --max-threshold 1 \
  --max-checkpoints 1)"

case "$OUTPUT" in
  *"audit_files: 3"*$'\n'*"llm_requests: 8"*$'\n'*"observed_usage_pairs: 2"*$'\n'*"simulation_note: local character-prefix model; not provider-observed KV-cache usage"*$'\n'*"simulation_coverage: supported=8/8 prompt_extracted=8 skipped_no_prompt=0 skipped_no_delta_boundary=0"* ) ;;
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

LOGICAL_OUTPUT="$(python3 scripts/kvc_replay.py \
  --audit "$TMP_DIR/data/.native/audit/api_audit.jsonl" \
  --observed-only)"
case "$LOGICAL_OUTPUT" in
  *"audit_files: 2"*$'\n'*"llm_requests: 2"*$'\n'*"observed_usage_pairs: 2"*$'\n'*"below_90=2"*$'\n'*"transitions=1"*) ;;
  *)
    echo "kvc_replay_test: rolling Responses audit was not merged or paired correctly" >&2
    echo "$LOGICAL_OUTPUT" >&2
    exit 1
    ;;
esac

echo "kvc_replay_test: ok"
