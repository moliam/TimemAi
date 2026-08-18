#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${1:-$ROOT_DIR/target/release/timem-web}"

if [ ! -x "$binary" ]; then
  echo "missing executable: $binary" >&2
  echo "run: cargo build --locked -p timem_web --release" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required for the Web runtime lifecycle smoke" >&2
  exit 1
fi

test_root="$(mktemp -d "${TMPDIR:-/tmp}/timem-web-lifecycle.XXXXXX")"
host_pid=""
launcher_pid=""
cleanup() {
  if [ -n "$launcher_pid" ] && kill -0 "$launcher_pid" >/dev/null 2>&1; then
    kill -TERM "$launcher_pid" >/dev/null 2>&1 || true
    wait "$launcher_pid" >/dev/null 2>&1 || true
  fi
  if [ -n "$host_pid" ] && kill -0 "$host_pid" >/dev/null 2>&1; then
    kill -TERM "$host_pid" >/dev/null 2>&1 || true
    wait "$host_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$test_root"
}
trap cleanup EXIT

wait_for_url() {
  local log_path="$1"
  local attempt
  local url
  for ((attempt = 1; attempt <= 200; attempt++)); do
    if ! kill -0 "$host_pid" >/dev/null 2>&1; then
      echo "timem-web exited before becoming ready" >&2
      sed -n '1,120p' "$log_path" >&2
      return 1
    fi
    url="$(sed -n 's/^Timem Web is ready at //p' "$log_path" | head -n 1)"
    if [ -n "$url" ]; then
      printf '%s\n' "$url"
      return 0
    fi
    sleep 0.05
  done
  echo "timem-web did not become ready in time" >&2
  sed -n '1,120p' "$log_path" >&2
  return 1
}

stop_host() {
  local stopped_pid="$host_pid"
  kill -TERM "$stopped_pid"
  if ! wait "$stopped_pid"; then
    echo "timem-web did not shut down cleanly" >&2
    return 1
  fi
  host_pid=""
}

first_log="$test_root/first.log"
"$binary" --no-open --data-dir "$test_root/data" --space lifecycle >"$first_log" 2>&1 &
host_pid=$!
first_url="$(wait_for_url "$first_log")"
first_authority="${first_url#http://}"
first_authority="${first_authority%%/*}"
first_port="${first_authority##*:}"
first_token="${first_url##*token=}"
curl_common=(
  --fail
  --silent
  --show-error
  --retry 10
  --retry-connrefused
  --retry-delay 0
  --connect-timeout 1
  --max-time 10
)

# Each request is a new client connection, matching repeated browser opens
# after a tab has been closed. The URL token must not be consumed or revoked.
for attempt in 1 2 3; do
  curl "${curl_common[@]}" "$first_url" >"$test_root/page-$attempt.html"
  grep -q '<div id="root">' "$test_root/page-$attempt.html"
done
curl "${curl_common[@]}" \
  "http://127.0.0.1:$first_port/api/health?token=$first_token" \
  | grep -q '"ok":true'

stop_host

# Restart immediately with the same data and port. This proves graceful
# shutdown releases the listener and the per-memory journal ownership lock.
second_log="$test_root/second.log"
"$binary" --no-open --port "$first_port" --data-dir "$test_root/data" --space lifecycle >"$second_log" 2>&1 &
host_pid=$!
second_url="$(wait_for_url "$second_log")"
second_token="${second_url##*token=}"
if [ "$first_token" = "$second_token" ]; then
  echo "a restarted runtime must rotate its access token" >&2
  exit 1
fi
curl "${curl_common[@]}" "$second_url" >"$test_root/restarted.html"
grep -q '<div id="root">' "$test_root/restarted.html"

old_status="$(curl --silent --show-error --connect-timeout 1 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  "http://127.0.0.1:$first_port/?token=$first_token")"
if [ "$old_status" != "401" ]; then
  echo "the previous runtime token remained authorized after restart: HTTP $old_status" >&2
  exit 1
fi

stop_host

# Simulate the terminal/launcher shell crashing without forwarding SIGHUP,
# SIGTERM, or Ctrl+C to timem-web. The Host must notice that its original
# parent changed, perform the normal runtime shutdown, and release both its
# listener and per-memory ownership lock.
launcher_log="$test_root/launcher-crash.log"
host_pid_file="$test_root/launcher-host.pid"
launcher_script="$test_root/launcher.sh"
cat >"$launcher_script" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

binary="$1"
host_pid_file="$2"
port="$3"
data_dir="$4"

"$binary" --no-open --port "$port" \
  --data-dir "$data_dir" --space launcher-crash &
child_pid=$!
printf '%s\n' "$child_pid" >"$host_pid_file"
wait "$child_pid"
EOF
chmod +x "$launcher_script"

"$launcher_script" "$binary" "$host_pid_file" "$first_port" \
  "$test_root/data" >"$launcher_log" 2>&1 &
launcher_pid=$!

for _ in $(seq 1 100); do
  [ -s "$host_pid_file" ] && break
  sleep 0.05
done
if [ ! -s "$host_pid_file" ]; then
  echo "launcher did not publish the timem-web child PID" >&2
  exit 1
fi
host_pid="$(cat "$host_pid_file")"
launcher_url="$(wait_for_url "$launcher_log")"
launcher_token="${launcher_url##*token=}"
curl "${curl_common[@]}"   "http://127.0.0.1:$first_port/api/health?token=$launcher_token"   | grep -q '"ok":true'

kill -KILL "$launcher_pid"
wait "$launcher_pid" >/dev/null 2>&1 || true
launcher_pid=""

# Start the replacement immediately, while the old Host may still be inside
# its 250 ms parent-exit detection and graceful-cleanup window. The replacement
# must wait for ownership handoff instead of falsely reusing a dying instance.
old_host_pid="$host_pid"
restart_log="$test_root/launcher-restart.log"
"$binary" --no-open --port "$first_port"   --data-dir "$test_root/data" --space launcher-crash >"$restart_log" 2>&1 &
host_pid=$!
restart_url="$(wait_for_url "$restart_log")"

for _ in $(seq 1 100); do
  if ! kill -0 "$old_host_pid" >/dev/null 2>&1; then
    break
  fi
  sleep 0.05
done
if kill -0 "$old_host_pid" >/dev/null 2>&1; then
  echo "the old timem-web Host survived after its launcher shell was killed" >&2
  sed -n '1,160p' "$launcher_log" >&2
  exit 1
fi

if [ "$host_pid" = "$old_host_pid" ]; then
  echo "the immediate restart did not create a fresh timem-web process" >&2
  exit 1
fi
curl "${curl_common[@]}" "$restart_url" >"$test_root/launcher-restarted.html"
grep -q '<div id="root">' "$test_root/launcher-restarted.html"
stop_host

echo "web_runtime_lifecycle_smoke: ok"
