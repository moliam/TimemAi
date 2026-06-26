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

echo "install_logic_test: ok"
