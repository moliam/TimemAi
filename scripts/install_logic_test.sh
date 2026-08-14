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
  macos|linux|unsupported_windows|unsupported) ;;
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

if ! grep -q 'Run: $COMMAND_NAME"' "$ROOT_DIR/install.sh"; then
  echo "install prompt should recommend running timem directly after sourcing env" >&2
  exit 1
fi

if grep -q 'Run: $COMMAND_NAME --space' "$ROOT_DIR/install.sh"; then
  echo "install prompt should not require duplicate --space/--model options" >&2
  exit 1
fi

if ! grep -q 'cargo fetch --locked' "$ROOT_DIR/install.sh"; then
  echo "install script should fetch Rust crate dependencies from Cargo.lock before building" >&2
  exit 1
fi

if ! grep -q 'cargo build --locked -p timem_shell -p timem_web --release' "$ROOT_DIR/install.sh"; then
  echo "install script should build both release binaries with locked dependencies" >&2
  exit 1
fi

if ! grep -q 'target/release/$WEB_BIN_NAME' "$ROOT_DIR/install.sh"; then
  echo "install script should install the embedded Web UI binary" >&2
  exit 1
fi

if ! grep -q 'pkg-config' "$ROOT_DIR/install.sh"; then
  echo "install script should cover Linux pkg-config for native Rust crate builds" >&2
  exit 1
fi

if ! grep -q 'Cargo downloads Rust crates' "$ROOT_DIR/README.md"; then
  echo "README should explain that Cargo installs Rust crate dependencies automatically" >&2
  exit 1
fi

echo "install_logic_test: ok"
