#!/usr/bin/env bash
# End-to-end self-upgrade smoke: run the release binary against a fake
# release server, apply the in-app upgrade, restart, and assert the
# replacement binary is running on the same port with the same workspace.
#
# Usage: scripts/self_upgrade_smoke.sh [path-to-timem-binary]
# The fake "newer" binary is built from this workspace's own sources and
# reports a bumped version via TIMEM_SMOKE_VERSION if the fixture flow needs
# one; the smoke primarily proves the replace/restart/handoff chain.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${1:-$ROOT_DIR/target/release/timem}"

if [ ! -x "$binary" ]; then
  echo "missing executable: $binary" >&2
  echo "run: cargo build --locked --release --bin timem" >&2
  exit 1
fi
for dep in curl python3; do
  if ! command -v "$dep" >/dev/null 2>&1; then
    echo "$dep is required for the self-upgrade smoke" >&2
    exit 1
  fi
done

test_root="$(mktemp -d "${TMPDIR:-/tmp}/timem-self-upgrade-smoke.XXXXXX")"
server_pid=""
host_pid=""
cleanup() {
  if [ -n "$server_pid" ] && kill -0 "$server_pid" >/dev/null 2>&1; then
    kill "$server_pid" >/dev/null 2>&1 || true
    wait "$server_pid" >/dev/null 2>&1 || true
  fi
  if [ -n "$host_pid" ] && kill -0 "$host_pid" >/dev/null 2>&1; then
    kill "$host_pid" >/dev/null 2>&1 || true
    wait "$host_pid" >/dev/null 2>&1 || true
  fi
  # Best effort: stop any process still using the smoke workspace (the
  # replacement handover keeps the same --space path). pkill is portable;
  # lsof is not present on every CI runner.
  if [ -n "${test_root:-}" ]; then
    pkill -f "$test_root/smoke-mem" >/dev/null 2>&1 || true
  fi
  rm -rf "$test_root"
}
trap cleanup EXIT

process_is_executing() {
  local pid="$1" state
  if ! kill -0 "$pid" >/dev/null 2>&1; then return 1; fi
  state="$(ps -o stat= -p "$pid" 2>/dev/null | awk 'NR == 1 { print $1 }')"
  case "$state" in Z*|X*|"") return 1 ;; *) return 0 ;; esac
}

wait_for_url() {
  local log_path="$1" attempt url
  for ((attempt = 1; attempt <= 400; attempt++)); do
    if [ -n "$host_pid" ] && ! process_is_executing "$host_pid"; then
      echo "timem-web exited before becoming ready" >&2
      sed -n '1,120p' "$log_path" >&2
      return 1
    fi
    url="$(sed -n '/^http:\/\/127\.0\.0\.1:[0-9][0-9]*\/$/p' "$log_path" | tail -n 1)"
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

curl_common=(--fail --silent --show-error --connect-timeout 1 --max-time 20)

wait_phase() {
  # wait_phase <base> <pattern> <label> — poll /api/upgrade/status until JSON matches.
  local base="$1" pattern="$2" label="$3" attempt body
  for ((attempt = 1; attempt <= 300; attempt++)); do
    body="$(curl "${curl_common[@]}" "$base/api/upgrade/status" || true)"
    if printf '%s' "$body" | grep -q "$pattern"; then
      return 0
    fi
    sleep 0.1
  done
  echo "upgrade never reached $label" >&2
  printf '%s\n' "$body" >&2
  return 1
}

# ---------------------------------------------------------------------------
# Fake release server: serves /releases/latest redirect, asset tar.gz, and
# the .sha256 sidecar built from the same binary under a bumped version tag.
# ---------------------------------------------------------------------------
fake_tag="v999.0.0"
server_root="$test_root/server"
release_dir="$server_root/releases/download/$fake_tag"
mkdir -p "$release_dir"

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64) target="aarch64-apple-darwin" ;;
  Darwin/x86_64) target="x86_64-apple-darwin" ;;
  Linux/x86_64) target="x86_64-unknown-linux-gnu" ;;
  *) echo "unsupported smoke platform: $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac

# Run an install copy so replace/backup files stay inside the test root.
install_dir="$test_root/install"
mkdir -p "$install_dir"
if [[ "$target" == *windows* ]]; then bin_name="timem.exe"; else bin_name="timem"; fi
cp "$binary" "$install_dir/$bin_name"
chmod +x "$install_dir/$bin_name"
binary="$install_dir/$bin_name"

archive="$release_dir/timem-$fake_tag-$target.tar.gz"
stage_dir="$test_root/stage"
mkdir -p "$stage_dir"
cp "$binary" "$stage_dir/$bin_name"
chmod +x "$stage_dir/$bin_name"
printf 'upgrade smoke README\n' > "$stage_dir/README-ASSETS.txt"
(cd "$stage_dir" && tar -czf "$archive" "$bin_name" README-ASSETS.txt)
case "$(uname -s)" in
  Darwin) digest="$(shasum -a 256 "$archive" | awk '{print $1}')" ;;
  *) digest="$(sha256sum "$archive" | awk '{print $1}')" ;;
esac
printf '%s  %s\n' "$digest" "$(basename "$archive")" > "$archive.sha256"

python3 - "$server_root" "$fake_tag" >"$test_root/server.log" 2>&1 <<'PYEOF' &
import http.server
import os
import socketserver
import sys
import urllib.parse

root, tag = sys.argv[1], sys.argv[2]

class Handler(http.server.SimpleHTTPRequestHandler):
    def translate_path(self, path):
        parsed = urllib.parse.urlsplit(path)
        rel = parsed.path.lstrip("/")
        return os.path.join(root, rel)

    def do_GET(self):
        if urllib.parse.urlsplit(self.path).path.rstrip("/") == "/releases/latest":
            self.send_response(302)
            self.send_header("Location", f"/releases/tag/{tag}")
            self.end_headers()
            return
        super().do_GET()

    def log_message(self, fmt, *args):
        pass

socketserver.TCPServer.allow_reuse_address = True
with socketserver.TCPServer(("127.0.0.1", 0), Handler) as httpd:
    print(httpd.server_address[1], flush=True)
    httpd.serve_forever()
PYEOF
server_pid=$!
for attempt in $(seq 1 100); do
  port_line="$(sed -n '1p' "$test_root/server.log" || true)"
  if [ -n "$port_line" ]; then break; fi
  sleep 0.05
done
server_port="${port_line:-}"
if [ -z "$server_port" ]; then
  echo "fake release server did not start" >&2
  cat "$test_root/server.log" >&2
  exit 1
fi
base_url="http://127.0.0.1:$server_port"

# ---------------------------------------------------------------------------
# 1) Boot the real binary against the fake release server.
# ---------------------------------------------------------------------------
first_log="$test_root/first.log"
TIMEM_UPGRADE_BASE_URL="$base_url" "$binary" --no-open --space "$test_root/smoke-mem" >"$first_log" 2>&1 &
host_pid=$!
first_url="$(wait_for_url "$first_log")"
first_authority="${first_url#http://}"
first_authority="${first_authority%%/*}"
first_port="${first_authority##*:}"
first_api="http://127.0.0.1:$first_port"

curl "${curl_common[@]}" "$first_url" >/dev/null

# ---------------------------------------------------------------------------
# 2) Check finds the fake newer release; apply stages and replaces the exe;
#    restart hands over to the replacement process on the same port.
# ---------------------------------------------------------------------------
curl "${curl_common[@]}" -X POST "$first_api/api/upgrade/check" >/dev/null
wait_phase "$first_api" '"update_available"' "update_available"

curl "${curl_common[@]}" -X POST "$first_api/api/upgrade/apply" >/dev/null
wait_phase "$first_api" '"pending_restart"' "pending_restart"

curl "${curl_common[@]}" -X POST "$first_api/api/upgrade/restart" >/dev/null

# Old process must exit after spawning the replacement. Graceful cleanup
# of workers can take a while; allow up to 120s.
for attempt in $(seq 1 1200); do
  if ! process_is_executing "$host_pid"; then break; fi
  sleep 0.1
done
if process_is_executing "$host_pid"; then
  echo "old process did not exit after restart request" >&2
  exit 1
fi
wait "$host_pid" 2>/dev/null || true
host_pid=""

# ---------------------------------------------------------------------------
# 3) The replacement binds the same port and serves the workspace again.
# ---------------------------------------------------------------------------
second_log="$test_root/second.log"
replacement_url="http://127.0.0.1:$first_port"
replacement_ok=""
for attempt in $(seq 1 1200); do
  if curl "${curl_common[@]}" "$replacement_url/api/health" 2>/dev/null | grep -q '"ok":true'; then
    replacement_ok=1
    break
  fi
  sleep 0.1
done
if [ -z "$replacement_ok" ]; then
  echo "replacement process did not take over on port $first_port" >&2
  pgrep -fl "$test_root/smoke-mem" >&2 || true
  for log in "$install_dir/.timem-restart.log" "$test_root/first.log"; do
    if [ -f "$log" ]; then
      echo "--- $log ---" >&2
      sed -n '1,80p' "$log" >&2
    fi
  done
  exit 1
fi

# The replacement process must be a fresh child of the replaced executable
# (its command line carries the smoke workspace path). Kill it via SIGTERM
# and confirm clean exit + workspace lock release by starting once more.
replacement_pid="$(pgrep -f "$test_root/smoke-mem" | head -n 1 || true)"
if [ -z "$replacement_pid" ]; then
  echo "could not identify the replacement process on port $first_port" >&2
  exit 1
fi
host_pid="$replacement_pid"
kill -TERM "$host_pid" 2>/dev/null || true
for attempt in $(seq 1 200); do
  if ! process_is_executing "$host_pid"; then break; fi
  sleep 0.1
done

third_log="$test_root/third.log"
"$binary" --no-open --port "$first_port" --space "$test_root/smoke-mem" >"$third_log" 2>&1 &
host_pid=$!
third_url="$(wait_for_url "$third_log")"
third_api="http://127.0.0.1:$first_port"
curl "${curl_common[@]}" "$third_api/api/health" | grep -q '"ok":true'
curl "${curl_common[@]}" "$third_api/api/upgrade/status" | grep -q '"phase":"idle"'

echo "self-upgrade smoke: ok"
