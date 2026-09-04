#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# shellcheck disable=SC1091
source "$ROOT_DIR/install.sh"

assert_version_at_least() {
  local actual="$1"
  local required="$2"
  if ! rust_version_at_least "$actual" "$required"; then
    echo "expected $actual >= $required" >&2
    exit 1
  fi
}

assert_version_below() {
  local actual="$1"
  local required="$2"
  if rust_version_at_least "$actual" "$required"; then
    echo "expected $actual < $required" >&2
    exit 1
  fi
}

assert_version_at_least "1.78.0" "1.78.0"
assert_version_at_least "1.96.0" "1.78.0"
assert_version_at_least "2.0.0" "1.99.9"
assert_version_below "1.77.9" "1.78.0"
assert_version_below "0.99.0" "1.0.0"

case "$(detect_os)" in
  macos|linux|windows|unsupported) ;;
  *)
    echo "detect_os returned an unexpected value" >&2
    exit 1
    ;;
esac

atomic_test_dir="$(mktemp -d "${TMPDIR:-/tmp}/timem-install-test.XXXXXX")"
trap 'rm -rf "$atomic_test_dir"' EXIT
printf 'old binary\n' > "$atomic_test_dir/destination"
old_inode="$(ls -di "$atomic_test_dir/destination" | awk '{print $1}')"
printf 'new binary\n' > "$atomic_test_dir/source"
chmod 600 "$atomic_test_dir/source"
install_binary_atomically "$atomic_test_dir/source" "$atomic_test_dir/destination"
new_inode="$(ls -di "$atomic_test_dir/destination" | awk '{print $1}')"
if [ "$old_inode" = "$new_inode" ]; then
  echo "binary install should replace the destination inode instead of overwriting it in place" >&2
  exit 1
fi
if [ "$(cat "$atomic_test_dir/destination")" != "new binary" ]; then
  echo "atomic binary install did not preserve source contents" >&2
  exit 1
fi
if [ ! -x "$atomic_test_dir/destination" ]; then
  echo "atomic binary install should make the installed file executable" >&2
  exit 1
fi
if find "$atomic_test_dir" -maxdepth 1 -name 'destination.tmp.*' | grep -q .; then
  echo "atomic binary install left a temporary file behind" >&2
  exit 1
fi

printf 'legacy web executable\n' > "$atomic_test_dir/timem-web"
printf 'legacy shell executable\n' > "$atomic_test_dir/timem-native-rs"
printf 'legacy shell wrapper\n' > "$atomic_test_dir/timem-shell"
install_web_alias "$atomic_test_dir/timem-web"
rm -f "$atomic_test_dir/timem-native-rs" "$atomic_test_dir/timem-shell"
if [ -e "$atomic_test_dir/timem-native-rs" ] || [ -e "$atomic_test_dir/timem-shell" ]; then
  echo "upgrade should remove legacy independent Shell artifacts" >&2
  exit 1
fi
if [ ! -L "$atomic_test_dir/timem-web" ]; then
  echo "compatibility entry should be a symbolic link, not a second executable copy" >&2
  exit 1
fi
if [ "$(readlink "$atomic_test_dir/timem-web")" != "timem" ]; then
  echo "compatibility entry should point relatively to the unified timem executable" >&2
  exit 1
fi

install_prompt="$(
  INSTALL_DIR="/example/bin"
  RESOURCE_DIR="/example/share/timem/resources"
  ROOT_DIR="/example/source"
  COMMAND_NAME="timem"
  WEB_ALIAS_NAME="timem-web"
  PATH="/usr/bin:/bin"
  NO_COLOR=1
  print_install_success
)"

for expected in \
  "TimemAi installation complete." \
  "Installed: /example/bin/timem" \
  "Run Timem:  timem" \
  "Shell mode: timem --shell" \
  "Update:    git pull --ff-only && ./install.sh" \
  "Uninstall: /example/source/uninstall.sh" \
  "Note: add /example/bin to PATH"; do
  if ! grep -Fq "$expected" <<< "$install_prompt"; then
    echo "install prompt is missing concise CLI guidance: $expected" >&2
    exit 1
  fi
done
if grep -Fq $'\033[' <<< "$install_prompt"; then
  echo "non-interactive install output must not contain ANSI color escapes" >&2
  exit 1
fi
if [ "$(wc -l <<< "$install_prompt" | tr -d ' ')" -gt 12 ]; then
  echo "install success output should remain concise" >&2
  exit 1
fi

online_prompt="$(
  INSTALL_DIR="/example/bin"
  COMMAND_NAME="timem"
  PATH="/example/bin:/usr/bin:/bin"
  NO_COLOR=1
  TIMEM_INSTALL_SOURCE_KIND=online
  TIMEM_INSTALL_VERSION=v9.8.7
  print_install_success
)"
for expected in \
  "Version:   v9.8.7" \
  "Update:    rerun the same one-line install command" \
  "Uninstall: curl -fsSL https://raw.githubusercontent.com/moliam/TimemAi/main/uninstall.sh | bash"; do
  if ! grep -Fq "$expected" <<< "$online_prompt"; then
    echo "online install prompt is missing guidance: $expected" >&2
    exit 1
  fi
done
for forbidden in "Env template:" "Update later from this git clone" "/timem-online-install."; do
  if grep -Fq "$forbidden" <<< "$online_prompt"; then
    echo "online install prompt contains stale or temporary guidance: $forbidden" >&2
    exit 1
  fi
done

if ! grep -q 'cargo fetch --locked' "$ROOT_DIR/install.sh"; then
  echo "install script should fetch Rust crate dependencies from Cargo.lock before building" >&2
  exit 1
fi

if ! grep -q 'cargo build --locked --release --bin timem' "$ROOT_DIR/install.sh"; then
  echo "install script should build the unified release executable with locked dependencies" >&2
  exit 1
fi

if grep -q 'cargo build --locked .*timem_shell' "$ROOT_DIR/install.sh"; then
  echo "install script must not build a second Shell executable" >&2
  exit 1
fi

if ! grep -Fq 'target/release/$COMMAND_NAME' "$ROOT_DIR/install.sh"; then
  echo "install script should install the unified Timem executable" >&2
  exit 1
fi
if ! grep -Fq 'install_web_alias "$INSTALL_DIR/$WEB_ALIAS_NAME"' "$ROOT_DIR/install.sh"; then
  echo "install script should create the compatibility alias without duplicating the executable" >&2
  exit 1
fi

if ! grep -q 'pkg-config' "$ROOT_DIR/install.sh"; then
  echo "install script should cover Linux pkg-config for native Rust crate builds" >&2
  exit 1
fi

if ! grep -Fq 'powershell -ExecutionPolicy Bypass -File .\install.ps1' "$ROOT_DIR/install.sh"; then
  echo "install.sh should direct Windows users to the native PowerShell installer" >&2
  exit 1
fi

if [ ! -f "$ROOT_DIR/install.ps1" ] || [ ! -f "$ROOT_DIR/uninstall.ps1" ] || [ ! -f "$ROOT_DIR/scripts/windows_install_logic_test.ps1" ]; then
  echo "Windows delivery scripts must be shipped" >&2
  exit 1
fi

if ! grep -Fq 'interfaces\web\dist\index.html' "$ROOT_DIR/install.ps1"; then
  echo "Windows installer must use the semantic interfaces/web frontend layout" >&2
  exit 1
fi

if ! grep -Fq 'cargo fetch --locked' "$ROOT_DIR/docs/install-and-configuration.md"; then
  echo "detailed install documentation should explain locked Cargo dependency fetching" >&2
  exit 1
fi


resource_source="$atomic_test_dir/reminder_tips.json"
resource_destination="$atomic_test_dir/share/timem/resources/reminder_tips.json"
printf '{"schedules":[]}\n' > "$resource_source"
install_resource_atomically "$resource_source" "$resource_destination"

if [ "$(cat "$resource_destination")" != '{"schedules":[]}' ]; then
  echo "atomic resource install did not preserve source contents" >&2
  exit 1
fi
if [ -x "$resource_destination" ]; then
  echo "installed reminder tips resource must not be executable" >&2
  exit 1
fi
if find "$(dirname "$resource_destination")" -maxdepth 1 -name 'reminder_tips.json.tmp.*' | grep -q .; then
  echo "atomic resource install left a temporary file behind" >&2
  exit 1
fi
if ! grep -Fq 'install_resource_atomically "$REMINDER_TIPS_SOURCE" "$RESOURCE_DIR/reminder_tips.json"' "$ROOT_DIR/install.sh"; then
  echo "install script should install reminder tips into the shared resources directory" >&2
  exit 1
fi
if ! grep -Fq 'rm -f "$RESOURCE_DIR/reminder_tips.json"' "$ROOT_DIR/uninstall.sh"; then
  echo "uninstall script should remove the installed reminder tips resource" >&2
  exit 1
fi
if grep -Eq 'rm .*TIMEM_CONFIG_DIR|rm .*reminder_tips_config_path' "$ROOT_DIR/uninstall.sh"; then
  echo "uninstall script must not remove user reminder tips overrides" >&2
  exit 1
fi

echo "install_logic_test: ok"
