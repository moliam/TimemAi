#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT_DIR/install.sh"

test_root="$(mktemp -d "${TMPDIR:-/tmp}/timem-online-install-test.XXXXXX")"
trap 'rm -rf "$test_root"' EXIT
fixture="$test_root/fixture/custom-fork-v9.8.7"
mkdir -p "$fixture/interfaces/web/dist" "$fixture/resources"
printf 'lock\n' > "$fixture/Cargo.lock"
printf '[workspace]\n' > "$fixture/Cargo.toml"
printf 'web\n' > "$fixture/interfaces/web/dist/index.html"
printf '{"schedules":[]}\n' > "$fixture/resources/reminder_tips.json"
printf 'template\n' > "$fixture/env_template"
cat > "$fixture/install.sh" <<'INSTALL'
#!/usr/bin/env bash
set -euo pipefail
echo 'the release archive installer must not be executed' >&2
exit 97
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

mkdir -p "$test_root/fake-bin"
cat > "$test_root/fake-bin/cargo" <<'CARGO'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  --version)
    echo 'cargo 1.90.0'
    ;;
  fetch)
    ;;
  build)
    mkdir -p target/release
    printf '#!/usr/bin/env bash
echo timem fixture
' > target/release/timem
    chmod +x target/release/timem
    ;;
  *)
    echo "unexpected cargo arguments: $*" >&2
    exit 2
    ;;
esac
CARGO
chmod +x "$test_root/fake-bin/cargo"

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
for mode in file pipe; do
  install_dir="$test_root/install-$mode/bin"
  resource_dir="$test_root/install-$mode/share/timem/resources"
  output="$test_root/output-$mode.txt"
  if [ "$mode" = file ]; then
    PATH="$test_root/fake-bin:$PATH" \
    TIMEM_INSTALL_CURL="$test_root/fake-curl" \
    TIMEM_ONLINE_TEST_ARCHIVE="$test_root/release.tar.gz" \
    TIMEM_SHELL_INSTALL_DIR="$install_dir" \
    TIMEM_RESOURCES_DIR="$resource_dir" \
    TMPDIR="$test_root" \
      bash "$test_root/install.sh" > "$output"
  else
    PATH="$test_root/fake-bin:$PATH" \
    TIMEM_INSTALL_CURL="$test_root/fake-curl" \
    TIMEM_ONLINE_TEST_ARCHIVE="$test_root/release.tar.gz" \
    TIMEM_SHELL_INSTALL_DIR="$install_dir" \
    TIMEM_RESOURCES_DIR="$resource_dir" \
    TMPDIR="$test_root" \
      bash < "$ROOT_DIR/install.sh" > "$output"
  fi
  [ -x "$install_dir/timem" ]
  [ -L "$install_dir/timem-web" ]
  [ -f "$resource_dir/reminder_tips.json" ]
  grep -Fq 'Version:   v9.8.7' "$output"
  grep -Fq 'Run Timem:  timem' "$output"
  grep -Fq 'Update:    rerun the same one-line install command' "$output"
  if grep -Fq 'Update later from this git clone' "$output" || grep -Fq "$fixture" "$output"; then
    echo 'online success output leaked stale checkout guidance or temporary paths' >&2
    exit 1
  fi
done

if find "$test_root" -maxdepth 1 -type d -name 'timem-online-install.*' | grep -q .; then
  echo 'online installer left its temporary directory behind' >&2
  exit 1
fi

echo 'online_install_logic_test: ok'
