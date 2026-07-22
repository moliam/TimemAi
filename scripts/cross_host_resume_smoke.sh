#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

binary="$ROOT_DIR/target/release/timem-native-rs"
if [ ! -x "$binary" ]; then
  echo "missing executable: $binary" >&2
  echo "run: cargo build --locked -p timem_shell -p timem_web --release" >&2
  exit 1
fi

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/timem_cross_host_resume.XXXXXX")"
provider_pid=""
cleanup() {
  if [ -n "$provider_pid" ]; then
    kill "$provider_pid" >/dev/null 2>&1 || true
    wait "$provider_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

data_dir="$tmp_dir/data"
workspace="$tmp_dir/workspace"
space=".cross_host_resume"
session_id="web_session_handoff"
history_path="$data_dir/$space/memory/sessions/$session_id/raw_chat_history.jsonl"
prompt_capture="$tmp_dir/provider_prompt.txt"
provider_log="$tmp_dir/fake_provider.log"
shell_output="$tmp_dir/shell_once.json"

mkdir -p "$workspace"

DATA_DIR="$data_dir" \
WORKSPACE="$workspace" \
SPACE="$space" \
SESSION_ID="$session_id" \
python3 - <<'PY'
import json
import os
from pathlib import Path

data_dir = Path(os.environ["DATA_DIR"])
workspace = Path(os.environ["WORKSPACE"])
space = os.environ["SPACE"]
session_id = os.environ["SESSION_ID"]
memory_dir = data_dir / space / "memory"
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
        "provider": "local",
        "model": "fake-cross-host-model",
        "api_protocol": "openai-compatible",
        "response_protocol": "xml",
    },
    "env": {
        "TIMEM_GATEWAY_PROVIDER": "local",
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

python3 scripts/fake_openai_provider.py \
  --port 0 \
  --delay 0 \
  --capture-prompt-file "$prompt_capture" \
  >"$provider_log" 2>&1 &
provider_pid="$!"

for _ in $(seq 1 200); do
  if grep -q 'fake_provider_ready:' "$provider_log"; then
    break
  fi
  if ! kill -0 "$provider_pid" >/dev/null 2>&1; then
    echo "fake provider exited before ready" >&2
    cat "$provider_log" >&2 || true
    exit 1
  fi
  sleep 0.1
done
if ! grep -q 'fake_provider_ready:' "$provider_log"; then
  echo "fake provider did not start within 20s" >&2
  cat "$provider_log" >&2 || true
  exit 1
fi
port="$(sed -n 's/^fake_provider_ready://p' "$provider_log" | tail -n 1)"

TIMEM_DATA_DIR="$data_dir" \
TIMEM_API_KEY=dummy \
TIMEM_GATEWAY_PROVIDER=local \
TIMEM_API_PROTOCOL=openai-compatible \
TIMEM_RESPONSE_PROTOCOL=xml \
TIMEM_BASE_URL="http://127.0.0.1:$port/v1" \
TIMEM_MODEL=fake-cross-host-model \
TIMEM_BASH_APPROVAL=approve \
TIMEM_WORK_INSTRUCTIONS=off \
"$binary" \
  --space "$space" \
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
