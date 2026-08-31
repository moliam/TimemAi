#!/usr/bin/env bash
set -euo pipefail

# macOS-native Web host acceptance.
#
# Scope: prove the release binary's macOS browser-launch adapter, owner-only
# runtime artifacts, loopback health endpoint, and graceful SIGTERM cleanup.
# Non-scope: browser rendering belongs to test:browser; shared restart/public
# lifecycle behavior belongs to the portable Web runtime smoke scripts.
# Constraint: use Darwin-native observations (`stat -f` and the real process
# signal path) rather than inferring macOS behavior from Linux or unit tests.
if [ "$(uname -s)" != "Darwin" ]; then
  echo "macos_web_platform_smoke: skipped (non-macOS)"
  exit 0
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${1:-$ROOT_DIR/target/release/timem}"

if [ ! -x "$binary" ]; then
  echo "missing executable: $binary" >&2
  echo "run: cargo build --locked --release --bin timem" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required for the macOS Web platform smoke" >&2
  exit 1
fi

test_root="$(mktemp -d "${TMPDIR:-/tmp}/timem-web-macos.XXXXXX")"
mem_root="$test_root/mem"
log_path="$test_root/web.log"
open_args="$test_root/open-args"
host_pid=""
cleanup() {
  if [ -n "$host_pid" ] && kill -0 "$host_pid" >/dev/null 2>&1; then
    kill -TERM "$host_pid" >/dev/null 2>&1 || true
    wait "$host_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$test_root"
}
trap cleanup EXIT

mkdir -p "$test_root/bin"
cat >"$test_root/bin/open" <<EOF_OPEN
#!/usr/bin/env bash
printf '%s\n' "\$@" >'$open_args'
EOF_OPEN
chmod +x "$test_root/bin/open"

# Deliberately omit --no-open. Shadowing `open` proves the product uses the
# platform command with a direct argv URL and does not invoke a shell.
env -u SSH_CONNECTION -u SSH_CLIENT -u SSH_TTY \
  PATH="$test_root/bin:$PATH" \
  "$binary" --space "$mem_root" </dev/null >"$log_path" 2>&1 &
host_pid=$!

url=""
for _ in $(seq 1 300); do
  if ! kill -0 "$host_pid" >/dev/null 2>&1; then
    echo "macOS timem-web exited before becoming ready" >&2
    sed -n '1,180p' "$log_path" >&2
    exit 1
  fi
  url="$(sed -n '/^http:\/\/127\.0\.0\.1:[0-9][0-9]*\/$/p' "$log_path" | tail -n 1)"
  [ -n "$url" ] && [ -s "$open_args" ] && break
  sleep 0.05
done
if [ -z "$url" ]; then
  echo "macOS timem-web did not become ready" >&2
  sed -n '1,180p' "$log_path" >&2
  exit 1
fi
if [ ! -s "$open_args" ]; then
  echo "macOS timem-web did not invoke the native browser command" >&2
  sed -n '1,180p' "$log_path" >&2
  exit 1
fi
if [ "$(wc -l <"$open_args" | tr -d ' ')" != "1" ] || [ "$(cat "$open_args")" != "$url" ]; then
  echo "macOS browser command did not receive exactly the direct local URL argument" >&2
  cat "$open_args" >&2
  exit 1
fi

authority="${url#http://}"
authority="${authority%%/*}"
port="${authority##*:}"
curl --fail --silent --show-error --retry 10 --retry-connrefused \
  --retry-delay 0 --connect-timeout 1 --max-time 10 \
  "http://127.0.0.1:$port/api/health" | grep -q '"ok":true'

permission_bits() {
  stat -f '%Lp' "$1"
}
if [ "$(permission_bits "$mem_root")" != "700" ]; then
  echo "macOS Web MEM directory is not mode 700" >&2
  stat -f '%Lp %N' "$mem_root" >&2
  exit 1
fi

diagnostics="$mem_root/diagnostics/timem-web"
for _ in $(seq 1 100); do
  [ -d "$diagnostics/current-runs" ] && break
  sleep 0.02
done
for directory in "$diagnostics" "$diagnostics/current-runs"; do
  if [ "$(permission_bits "$directory")" != "700" ]; then
    echo "macOS Web diagnostics directory is not mode 700: $directory" >&2
    stat -f '%Lp %N' "$directory" >&2
    exit 1
  fi
done
marker="$(find "$diagnostics/current-runs" -maxdepth 1 -type f -name '*.json' -print -quit)"
if [ -z "$marker" ] || [ "$(permission_bits "$marker")" != "600" ]; then
  echo "macOS Web running marker is missing or not mode 600" >&2
  [ -n "$marker" ] && stat -f '%Lp %N' "$marker" >&2
  exit 1
fi

kill -TERM "$host_pid"
if ! wait "$host_pid"; then
  echo "macOS timem-web did not shut down cleanly after SIGTERM" >&2
  exit 1
fi
host_pid=""

python3 - "$diagnostics/last-exit.json" <<'PY_JSON'
import json
import pathlib
import sys
record = json.loads(pathlib.Path(sys.argv[1]).read_text())
assert record["exit_reason"] == "sigterm", record
assert record["graceful"] is True, record
assert any(
    event.get("name") == "graceful_shutdown_completed"
    for event in record.get("recent_lifecycle_events", [])
), record
PY_JSON
if find "$diagnostics/current-runs" -maxdepth 1 -type f -name '*.json' | grep -q .; then
  echo "macOS Web left a running marker after graceful shutdown" >&2
  exit 1
fi
if [ "$(permission_bits "$diagnostics/last-exit.json")" != "600" ]; then
  echo "macOS Web exit diagnostics are not mode 600" >&2
  stat -f '%Lp %N' "$diagnostics/last-exit.json" >&2
  exit 1
fi

echo "macos_web_platform_smoke: ok"
