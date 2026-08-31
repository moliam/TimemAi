#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

binary="$ROOT_DIR/target/release/timem"
if [ ! -x "$binary" ]; then
  echo "missing executable: $binary" >&2
  echo "run: cargo build --locked --release --bin timem" >&2
  exit 1
fi

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/timem_cross_host_resume.XXXXXX")"
model_server_pid=""
cleanup() {
  if [ -n "$model_server_pid" ]; then
    kill "$model_server_pid" >/dev/null 2>&1 || true
    wait "$model_server_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

memory_dir="$tmp_dir/mem"
workspace="$tmp_dir/workspace"
session_id="web_session_handoff"
history_path="$memory_dir/sessions/$session_id/raw_chat_history.jsonl"
prompt_capture="$tmp_dir/model_prompt.txt"
model_server_log="$tmp_dir/fake_model_server.log"
shell_output="$tmp_dir/shell_once.json"

mkdir -p "$workspace"

MEMORY_DIR="$memory_dir" \
WORKSPACE="$workspace" \
SESSION_ID="$session_id" \
python3 - <<'PY'
import json
import os
from pathlib import Path

memory_dir = Path(os.environ["MEMORY_DIR"])
workspace = Path(os.environ["WORKSPACE"])
session_id = os.environ["SESSION_ID"]
sessions_dir = memory_dir / "sessions"
history_path = sessions_dir / session_id / "raw_chat_history.jsonl"
history_path.parent.mkdir(parents=True, exist_ok=True)
sessions_dir.mkdir(parents=True, exist_ok=True)

session = {
    "session_id": session_id,
    "display_name": "Session0",
    "created_at_ms": 1,
    "updated_at_ms": 4,
    "current_dir": str(workspace),
    "profile": {
        "model": "fake-cross-host-model",
        "api_protocol": "openai-compatible",
        "response_protocol": "xml",
    },
    "env": {
        "TIMEM_MODEL": "fake-cross-host-model",
        "TIMEM_API_PROTOCOL": "openai-compatible",
        "TIMEM_RESPONSE_PROTOCOL": "xml",
        "TIMEM_BASH_APPROVAL": "approve",
        "TIMEM_WORK_INSTRUCTIONS": "off",
    },
    "state": "ready",
    "last_turn_id": "turn_web_1",
    "raw_chat_history_path": str(history_path),
}
(sessions_dir / "index.jsonl").write_text(json.dumps(session, ensure_ascii=False) + "\n", encoding="utf-8")

records = [
    {
        "type": "message",
        "role": "user",
        "turn_id": "turn_web_1",
        "created_at_ms": 2,
        "content": "web user question",
    },
    {
        "type": "event",
        "role": "system",
        "turn_id": "turn_web_1",
        "created_at_ms": 3,
        "kind": "action_result",
        "content": "Action result: run_bash\nok",
        "payload": {"action": "run_bash", "status": "completed"},
    },
    {
        "type": "message",
        "role": "assistant",
        "turn_id": "turn_web_1",
        "created_at_ms": 4,
        "content": "web final answer",
    },
]
with history_path.open("w", encoding="utf-8") as history:
    for record in records:
        history.write(json.dumps(record, ensure_ascii=False) + "\n")
PY

python3 scripts/fake_openai_server.py \
  --port 0 \
  --delay 0 \
  --capture-prompt-file "$prompt_capture" \
  >"$model_server_log" 2>&1 &
model_server_pid="$!"

for _ in $(seq 1 200); do
  if grep -q 'fake_model_server_ready:' "$model_server_log"; then
    break
  fi
  if ! kill -0 "$model_server_pid" >/dev/null 2>&1; then
    echo "fake model server exited before ready" >&2
    cat "$model_server_log" >&2 || true
    exit 1
  fi
  sleep 0.1
done
if ! grep -q 'fake_model_server_ready:' "$model_server_log"; then
  echo "fake model server did not start within 20s" >&2
  cat "$model_server_log" >&2 || true
  exit 1
fi
port="$(sed -n 's/^fake_model_server_ready://p' "$model_server_log" | tail -n 1)"

TIMEM_API_KEY=dummy \
TIMEM_API_PROTOCOL=openai-compatible \
TIMEM_RESPONSE_PROTOCOL=xml \
TIMEM_BASE_URL="http://127.0.0.1:$port/v1" \
TIMEM_MODEL=fake-cross-host-model \
TIMEM_BASH_APPROVAL=approve \
TIMEM_WORK_INSTRUCTIONS=off \
"$binary" \
  --shell \
  --space "$memory_dir" \
  --once-json "CROSS_HOST_RESUME_SMOKE" \
  >"$shell_output"

python3 - "$shell_output" <<'PY'
import json
import sys
doc = json.loads(open(sys.argv[1], encoding="utf-8").read())
assert doc["session_id"] == "web_session_handoff", doc
assert doc["status"] == "done", doc
assert "CROSS_HOST_RESUME_OK" in doc["output"], doc
PY

grep -q 'Refer to chat history when necessary:' "$prompt_capture"
grep -q 'raw_chat_history.jsonl' "$prompt_capture"
grep -q 'format: JSONL, one record per line.' "$prompt_capture"
grep -q '"type":"message"' "$prompt_capture"
grep -q '"type":"event"' "$prompt_capture"
grep -q 'Current cwd:' "$prompt_capture"
test -s "$history_path"
grep -q 'web user question' "$history_path"
grep -q 'web final answer' "$history_path"

echo "cross_host_resume_smoke: ok"
