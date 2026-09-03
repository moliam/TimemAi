#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT_DIR/install.sh"

test_root="$(mktemp -d "${TMPDIR:-/tmp}/timem-online-install-test.XXXXXX")"
trap 'rm -rf "$test_root"' EXIT
fixture="$test_root/fixture/custom-fork-v9.8.7"
mkdir -p "$fixture/interfaces/web/dist"
printf 'lock\n' > "$fixture/Cargo.lock"
printf 'web\n' > "$fixture/interfaces/web/dist/index.html"
cat > "$fixture/install.sh" <<'INSTALL'
#!/usr/bin/env bash
set -euo pipefail
printf 'installed from fixture\n' > "${TIMEM_ONLINE_TEST_MARKER:?}"
INSTALL
chmod +x "$fixture/install.sh"
tar -czf "$test_root/release.tar.gz" -C "$test_root/fixture" custom-fork-v9.8.7

cat > "$test_root/fake-curl" <<'CURL'
#!/usr/bin/env bash
set -euo pipefail
if printf '%s\n' "$@" | grep -Fq '/releases/latest'; then
  printf '%s' 'https://github.com/moliam/TimemAi/releases/tag/v9.8.7'
  exit 0
fi
output=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then
    output="$2"
    shift 2
  else
    shift
  fi
done
cp "${TIMEM_ONLINE_TEST_ARCHIVE:?}" "$output"
CURL
chmod +x "$test_root/fake-curl"

[ "$(ONLINE_CURL_BIN="$test_root/fake-curl" resolve_online_version latest)" = "v9.8.7" ]
[ "$(resolve_online_version v2.0.0)" = "v2.0.0" ]
if (resolve_online_version '../main') >/dev/null 2>&1; then
  echo 'unsafe release version should be rejected' >&2
  exit 1
fi
if (validate_online_repository 'owner/repo/extra') >/dev/null 2>&1; then
  echo 'unsafe repository should be rejected' >&2
  exit 1
fi

cp "$ROOT_DIR/install.sh" "$test_root/install.sh"
marker="$test_root/installed"
TIMEM_INSTALL_CURL="$test_root/fake-curl" \
TIMEM_ONLINE_TEST_ARCHIVE="$test_root/release.tar.gz" \
TIMEM_ONLINE_TEST_MARKER="$marker" \
TMPDIR="$test_root" \
  bash "$test_root/install.sh"
[ "$(cat "$marker")" = 'installed from fixture' ]

pipe_marker="$test_root/installed-from-pipe"
TIMEM_INSTALL_CURL="$test_root/fake-curl" \
TIMEM_ONLINE_TEST_ARCHIVE="$test_root/release.tar.gz" \
TIMEM_ONLINE_TEST_MARKER="$pipe_marker" \
TMPDIR="$test_root" \
  bash < "$ROOT_DIR/install.sh"
[ "$(cat "$pipe_marker")" = 'installed from fixture' ]

if find "$test_root" -maxdepth 1 -type d -name 'timem-online-install.*' | grep -q .; then
  echo 'online installer left its temporary directory behind' >&2
  exit 1
fi

echo 'online_install_logic_test: ok'
