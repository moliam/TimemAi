#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${1:-$ROOT_DIR/target/release/timem-web}"

if [ ! -x "$binary" ]; then
  echo "missing executable: $binary" >&2
  echo "run: cargo build --locked -p timem_web --release" >&2
  exit 1
fi
for command in curl node; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "$command is required for the public Web runtime smoke" >&2
    exit 1
  fi
done

test_root="$(mktemp -d "${TMPDIR:-/tmp}/timem-web-public.XXXXXX")"
host_pid=""
cleanup() {
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
      echo "public timem-web exited before becoming ready" >&2
      sed -E 's/token=[^[:space:]]+/token=[REDACTED]/g' "$log_path" >&2
      return 1
    fi
    url="$(sed -n 's/^Timem Web is ready at //p' "$log_path" | head -n 1)"
    if [ -n "$url" ]; then
      printf '%s\n' "$url"
      return 0
    fi
    sleep 0.05
  done
  echo "public timem-web did not become ready in time" >&2
  sed -E 's/token=[^[:space:]]+/token=[REDACTED]/g' "$log_path" >&2
  return 1
}

assert_token_shape() {
  local token="$1"
  if [[ ! "$token" =~ ^[[:xdigit:]]{16}$ ]]; then
    echo "public Web runtime token must be exactly 16 hexadecimal characters" >&2
    exit 1
  fi
}

stop_host() {
  local stopped_pid="$host_pid"
  kill -TERM "$stopped_pid"
  if ! wait "$stopped_pid"; then
    echo "public timem-web did not shut down cleanly" >&2
    return 1
  fi
  host_pid=""
}

assert_websocket_hello() {
  local ws_url="$1"
  node -e '
    const url = process.argv[1];
    const timer = setTimeout(() => {
      console.error("public WebSocket hello timed out");
      process.exit(1);
    }, 10000);
    const socket = new WebSocket(url);
    socket.addEventListener("message", (message) => {
      let payload;
      try {
        payload = JSON.parse(String(message.data));
      } catch (error) {
        console.error(`invalid public WebSocket message: ${error}`);
        process.exit(1);
      }
      if (payload.type !== "hello" || !payload.snapshot?.server?.public_access) {
        console.error("public WebSocket did not return a public hello snapshot");
        process.exit(1);
      }
      clearTimeout(timer);
      socket.close();
      process.exit(0);
    });
    socket.addEventListener("error", () => {
      console.error("public WebSocket connection failed");
      process.exit(1);
    });
  ' "$ws_url"
}

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

first_log="$test_root/first.log"
"$binary" --public --public-host 127.0.0.1 --no-open \
  --data-dir "$test_root/data" --space public-lifecycle >"$first_log" 2>&1 &
host_pid=$!
first_url="$(wait_for_url "$first_log")"
first_authority="${first_url#http://}"
first_authority="${first_authority%%/*}"
first_port="${first_authority##*:}"
first_token="${first_url##*token=}"
assert_token_shape "$first_token"

case "$first_url" in
  "http://127.0.0.1:$first_port/?token="*) ;;
  *) echo "public URL did not use the configured advertised host" >&2; exit 1 ;;
esac
grep -q 'Public mode is enabled.' "$first_log"
grep -q 'The server is bound to 0.0.0.0.' "$first_log"

unauthorized_status="$(curl --silent --show-error --connect-timeout 1 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:$first_port/")"
wrong_token_status="$(curl --silent --show-error --connect-timeout 1 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  "http://127.0.0.1:$first_port/?token=invalid")"
if [ "$unauthorized_status" != "401" ] || [ "$wrong_token_status" != "401" ]; then
  echo "public Web entry did not reject missing/invalid credentials" >&2
  exit 1
fi

curl "${curl_common[@]}" --dump-header "$test_root/headers" \
  --output "$test_root/page.html" "$first_url"
grep -q '<div id="root">' "$test_root/page.html"
grep -Eiq '^set-cookie: timem_web_token=.*Path=/; SameSite=Strict; HttpOnly' \
  "$test_root/headers"
cookie="$(sed -nE 's/^[Ss]et-[Cc]ookie:[[:space:]]*([^;]+).*/\1/p' \
  "$test_root/headers" | head -n 1 | tr -d '\r')"
if [ -z "$cookie" ]; then
  echo "public Web entry did not establish an authenticated cookie" >&2
  exit 1
fi

# A browser removes the token from the visible URL after the first response.
# The HttpOnly cookie must keep later page and API connections authenticated.
for attempt in 1 2 3; do
  curl "${curl_common[@]}" --header "Cookie: $cookie" \
    "http://127.0.0.1:$first_port/" >"$test_root/cookie-page-$attempt.html"
  grep -q '<div id="root">' "$test_root/cookie-page-$attempt.html"
done
curl "${curl_common[@]}" --header "Cookie: $cookie" \
  "http://127.0.0.1:$first_port/api/health" | grep -q '"ok":true'
assert_websocket_hello "ws://127.0.0.1:$first_port/ws?token=$first_token"

stop_host

# Public mode must release its listener and journal ownership on shutdown, and
# rotate credentials when the same data root and port are started again.
second_log="$test_root/second.log"
"$binary" --public --public-host 127.0.0.1 --no-open --port "$first_port" \
  --data-dir "$test_root/data" --space public-lifecycle >"$second_log" 2>&1 &
host_pid=$!
second_url="$(wait_for_url "$second_log")"
second_token="${second_url##*token=}"
assert_token_shape "$second_token"
if [ "$first_token" = "$second_token" ]; then
  echo "a restarted public runtime must rotate its access token" >&2
  exit 1
fi

old_status="$(curl --silent --show-error --connect-timeout 1 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  "http://127.0.0.1:$first_port/?token=$first_token")"
if [ "$old_status" != "401" ]; then
  echo "the previous public runtime token remained authorized: HTTP $old_status" >&2
  exit 1
fi
curl "${curl_common[@]}" "$second_url" >"$test_root/restarted.html"
grep -q '<div id="root">' "$test_root/restarted.html"
assert_websocket_hello "ws://127.0.0.1:$first_port/ws?token=$second_token"

stop_host
echo "web_public_runtime_smoke: ok"
