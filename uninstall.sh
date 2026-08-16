#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="${TIMEM_SHELL_INSTALL_DIR:-$HOME/.local/bin}"
RESOURCE_DIR="${TIMEM_RESOURCES_DIR:-$(dirname "$INSTALL_DIR")/share/timem/resources}"
BIN_NAME="timem-native-rs"
WEB_BIN_NAME="timem-web"
COMMAND_NAME="timem"
OLD_WRAPPER_NAME="timem-shell"

for arg in "$@"; do
  case "$arg" in
    -h|--help)
      echo "Usage: ./uninstall.sh"
      echo
      echo "Removes Timem binaries and the installed resources under $RESOURCE_DIR."
      echo "Private env files are user-managed and are not removed."
      exit 0
      ;;
    *)
      echo "error: unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

rm -f "$INSTALL_DIR/$BIN_NAME" "$INSTALL_DIR/$COMMAND_NAME" "$INSTALL_DIR/$WEB_BIN_NAME" "$INSTALL_DIR/$OLD_WRAPPER_NAME"
rm -f "$RESOURCE_DIR/reminder_tips.json"
rmdir "$RESOURCE_DIR" 2>/dev/null || true
rmdir "$(dirname "$RESOURCE_DIR")" 2>/dev/null || true

echo "Uninstalled Timem CLI and Web binaries from $INSTALL_DIR."
echo "Private env files and user reminder_tips.json overrides were not removed."
echo "Rust toolchain is not removed. If you installed Rust only for Timem shell, remove it with: rustup self uninstall"
