#!/usr/bin/env bash
set -euo pipefail

if [ "$(uname -s)" != "Linux" ]; then
  echo "linux_web_platform_smoke: skipped (non-Linux)"
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
  echo "curl is required for the Linux Web platform smoke" >&2
  exit 1
fi

test_root="$(mktemp -d "${TMPDIR:-/tmp}/timem-web-linux.XXXXXX")"
mem_root="$test_root/mem"
log_path="$test_root/web.log"
xdg_marker="$test_root/xdg-open-called"
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
cat >"$test_root/bin/xdg-open" <<EOF_XDG
#!/usr/bin/env bash
printf 'unexpected xdg-open invocation\n' >'$xdg_marker'
exit 99
EOF_XDG
chmod +x "$test_root/bin/xdg-open"

# Model a systemd/SSH-style service environment: stdin is not a terminal and
# no local graphical display exists. Deliberately omit --no-open so this proves
# the Linux auto-open policy itself, not merely the launch option.
env -u DISPLAY -u WAYLAND_DISPLAY -u SSH_CONNECTION -u SSH_CLIENT -u SSH_TTY \
  PATH="$test_root/bin:$PATH" \
  "$binary" --space "$mem_root" </dev/null >"$log_path" 2>&1 &
host_pid=$!

url=""
for _ in $(seq 1 300); do
  if ! kill -0 "$host_pid" >/dev/null 2>&1; then
    echo "Linux timem-web exited before becoming ready" >&2
    sed -n '1,180p' "$log_path" >&2
    exit 1
  fi
  url="$(sed -n '/^http:\/\/127\.0\.0\.1:[0-9][0-9]*\/$/p' "$log_path" | tail -n 1)"
  [ -n "$url" ] && break
  sleep 0.05
done
if [ -z "$url" ]; then
  echo "Linux timem-web did not become ready" >&2
  sed -n '1,180p' "$log_path" >&2
  exit 1
fi

grep -q 'No local graphical session detected; browser auto-open skipped' "$log_path"
if [ -e "$xdg_marker" ]; then
  echo "headless Linux timem-web unexpectedly invoked xdg-open" >&2
  exit 1
fi

authority="${url#http://}"
authority="${authority%%/*}"
port="${authority##*:}"
curl --fail --silent --show-error --retry 10 --retry-connrefused \
  --retry-delay 0 --connect-timeout 1 --max-time 10 \
  "http://127.0.0.1:$port/api/health" | grep -q '"ok":true'

if [ "$(stat -c '%a' "$mem_root")" != "700" ]; then
  echo "Linux Web MEM directory is not mode 700" >&2
  stat -c '%a %n' "$mem_root" >&2
  exit 1
fi
diagnostics="$mem_root/diagnostics/timem-web"
for _ in $(seq 1 100); do
  [ -d "$diagnostics/current-runs" ] && break
  sleep 0.02
done
for directory in "$diagnostics" "$diagnostics/current-runs"; do
  if [ "$(stat -c '%a' "$directory")" != "700" ]; then
    echo "Linux Web diagnostics directory is not mode 700: $directory" >&2
    stat -c '%a %n' "$directory" >&2
    exit 1
  fi
done
marker="$(find "$diagnostics/current-runs" -maxdepth 1 -type f -name '*.json' -print -quit)"
if [ -z "$marker" ] || [ "$(stat -c '%a' "$marker")" != "600" ]; then
  echo "Linux Web running marker is missing or not mode 600" >&2
  [ -n "$marker" ] && stat -c '%a %n' "$marker" >&2
  exit 1
fi

kill -TERM "$host_pid"
if ! wait "$host_pid"; then
  echo "Linux timem-web did not shut down cleanly after SIGTERM" >&2
  exit 1
fi
host_pid=""

python3 - "$diagnostics/last-exit.json" <<'PY_JSON'
import json
import pathlib
import sys
path = pathlib.Path(sys.argv[1])
record = json.loads(path.read_text())
assert record["exit_reason"] == "sigterm", record
assert record["graceful"] is True, record
assert any(
    event.get("name") == "graceful_shutdown_completed"
    for event in record.get("recent_lifecycle_events", [])
), record
PY_JSON
if find "$diagnostics/current-runs" -maxdepth 1 -type f -name '*.json' | grep -q .; then
  echo "Linux Web left a running marker after graceful shutdown" >&2
  exit 1
fi
if [ "$(stat -c '%a' "$diagnostics/last-exit.json")" != "600" ]; then
  echo "Linux Web exit diagnostics are not mode 600" >&2
  stat -c '%a %n' "$diagnostics/last-exit.json" >&2
  exit 1
fi

echo "linux_web_platform_smoke: ok"
