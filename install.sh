#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${TIMEM_SHELL_INSTALL_DIR:-$HOME/.local/bin}"
RESOURCE_DIR="${TIMEM_RESOURCES_DIR:-$(dirname "$INSTALL_DIR")/share/timem/resources}"
REMINDER_TIPS_SOURCE="$ROOT_DIR/resources/reminder_tips.json"
ENV_TEMPLATE="$ROOT_DIR/env_template"
COMMAND_NAME="timem"
WEB_ALIAS_NAME="timem-web"
OLD_BIN_NAME="timem-native-rs"
OLD_WRAPPER_NAME="timem-shell"
MIN_RUST_VERSION="1.78.0"

cd "$ROOT_DIR"

detect_os() {
  case "$(uname -s)" in
    Darwin) echo "macos" ;;
    Linux) echo "linux" ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      echo "windows"
      ;;
    *)
      echo "unsupported"
      ;;
  esac
}

install_with_sudo_if_available() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    return 1
  fi
}

ensure_build_dependencies() {
  local os="$1"
  case "$os" in
    macos)
      if ! xcode-select -p >/dev/null 2>&1; then
        echo "macOS build tools are required. Installing Command Line Tools..."
        xcode-select --install || true
        echo "After Command Line Tools finish installing, rerun ./install.sh." >&2
        exit 1
      fi
      if ! command -v curl >/dev/null 2>&1; then
        echo "error: curl is required. Install it with Homebrew or Xcode Command Line Tools, then rerun ./install.sh." >&2
        exit 1
      fi
      ;;
    linux)
      if command -v cc >/dev/null 2>&1 && command -v make >/dev/null 2>&1 && command -v curl >/dev/null 2>&1 && command -v pkg-config >/dev/null 2>&1; then
        return
      fi
      echo "Linux build tools are required: cc, make, curl, pkg-config."
      if command -v apt-get >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with apt-get..."
        install_with_sudo_if_available apt-get update
        install_with_sudo_if_available apt-get install -y build-essential curl ca-certificates pkg-config
      elif command -v dnf >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with dnf..."
        install_with_sudo_if_available dnf install -y gcc gcc-c++ make curl ca-certificates pkgconf-pkg-config
      elif command -v yum >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with yum..."
        install_with_sudo_if_available yum install -y gcc gcc-c++ make curl ca-certificates pkgconfig
      elif command -v pacman >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with pacman..."
        install_with_sudo_if_available pacman -Sy --needed --noconfirm base-devel curl ca-certificates pkgconf
      elif command -v zypper >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with zypper..."
        install_with_sudo_if_available zypper install -y gcc gcc-c++ make curl ca-certificates pkg-config
      else
        echo "error: unsupported Linux package manager." >&2
        echo "Install cc, make, curl, pkg-config, and ca-certificates manually, then rerun ./install.sh." >&2
        exit 1
      fi
      if ! command -v cc >/dev/null 2>&1 || ! command -v make >/dev/null 2>&1 || ! command -v curl >/dev/null 2>&1 || ! command -v pkg-config >/dev/null 2>&1; then
        echo "error: build dependencies are still missing after install attempt." >&2
        echo "Install cc, make, curl, pkg-config, and ca-certificates manually, then rerun ./install.sh." >&2
        exit 1
      fi
      ;;
    windows)
      echo "error: On Windows, run the native PowerShell installer instead:" >&2
      echo '  powershell -ExecutionPolicy Bypass -File .\install.ps1' >&2
      exit 1
      ;;
    *)
      echo "error: unsupported OS. Use the Timem installer for Windows, macOS, or Linux." >&2
      exit 1
      ;;
  esac
}

ensure_rust() {
  if command -v cargo >/dev/null 2>&1 && rust_version_at_least "$(cargo --version | awk '{print $2}')" "$MIN_RUST_VERSION"; then
    return
  fi

  if [ "${TIMEM_SHELL_SKIP_RUST_INSTALL:-0}" = "1" ]; then
    echo "error: cargo >= $MIN_RUST_VERSION is required. Install or update Rust from https://rustup.rs/ first." >&2
    exit 1
  fi

  if command -v rustup >/dev/null 2>&1; then
    echo "Updating Rust toolchain with rustup; Cargo >= $MIN_RUST_VERSION is required for Cargo.lock v4..."
    rustup update stable
    rustup default stable
  elif command -v curl >/dev/null 2>&1; then
    echo "Rust/cargo not found or too old. Installing Rust toolchain with rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  else
    echo "error: curl is required to install Rust automatically." >&2
    echo "Install Rust manually from https://rustup.rs/ or rerun after installing curl." >&2
    exit 1
  fi

  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1090
    . "$HOME/.cargo/env"
  fi

  if ! command -v cargo >/dev/null 2>&1 || ! rust_version_at_least "$(cargo --version | awk '{print $2}')" "$MIN_RUST_VERSION"; then
    echo "error: cargo >= $MIN_RUST_VERSION is required after Rust setup." >&2
    exit 1
  fi
}

rust_version_at_least() {
  local actual="${1%%-*}"
  local required="$2"
  local actual_major actual_minor actual_patch required_major required_minor required_patch
  IFS=. read -r actual_major actual_minor actual_patch <<< "$actual"
  IFS=. read -r required_major required_minor required_patch <<< "$required"
  actual_major="${actual_major:-0}"
  actual_minor="${actual_minor:-0}"
  actual_patch="${actual_patch:-0}"
  required_major="${required_major:-0}"
  required_minor="${required_minor:-0}"
  required_patch="${required_patch:-0}"
  if [ "$actual_major" -ne "$required_major" ]; then
    [ "$actual_major" -gt "$required_major" ]
    return
  fi
  if [ "$actual_minor" -ne "$required_minor" ]; then
    [ "$actual_minor" -gt "$required_minor" ]
    return
  fi
  [ "$actual_patch" -ge "$required_patch" ]
}

fetch_rust_dependencies() {
  echo "Fetching Rust crate dependencies from Cargo.lock..."
  echo "Cargo will download crates such as termimad automatically; no manual crate install is needed."
  if ! cargo fetch --locked; then
    echo "error: failed to fetch Rust crate dependencies." >&2
    echo "Check network access to crates.io, then rerun ./install.sh." >&2
    exit 1
  fi
}

build_release_binary() {
  echo "Building the unified Timem CLI (Web by default, Shell with --shell)..."
  if [ ! -f "$ROOT_DIR/interfaces/web/dist/index.html" ]; then
    echo "error: embedded Timem Web assets are missing from this source package." >&2
    exit 1
  fi
  if ! cargo build --locked --release --bin timem; then
    echo "error: release build failed." >&2
    echo "If this is a fresh machine, rerun ./install.sh after confirming Rust and system build dependencies installed successfully." >&2
    exit 1
  fi
}

install_binary_atomically() {
  local source="$1"
  local destination="$2"
  local temporary

  temporary="$(mktemp "${destination}.tmp.XXXXXX")"
  if ! cp "$source" "$temporary" || ! chmod 755 "$temporary" || ! mv -f "$temporary" "$destination"; then
    rm -f "$temporary"
    echo "error: failed to install $destination atomically." >&2
    return 1
  fi
}


install_web_alias() {
  local destination="$1"
  local temporary="${destination}.tmp.$$"

  rm -f "$temporary"
  if ! ln -s "$COMMAND_NAME" "$temporary" || ! mv -f "$temporary" "$destination"; then
    rm -f "$temporary"
    echo "error: failed to install compatibility alias $destination atomically." >&2
    return 1
  fi
}

install_resource_atomically() {
  local source="$1"
  local destination="$2"
  local temporary

  mkdir -p "$(dirname "$destination")"
  temporary="$(mktemp "${destination}.tmp.XXXXXX")"
  if ! cp "$source" "$temporary" || ! chmod 644 "$temporary" || ! mv -f "$temporary" "$destination"; then
    rm -f "$temporary"
    echo "error: failed to install resource $destination atomically." >&2
    return 1
  fi
}

print_install_success() {
  echo
  echo "TimemAi installation complete."
  echo
  echo "Installed program:"
  echo "  Timem CLI:               $INSTALL_DIR/$COMMAND_NAME"
  echo "  Compatibility alias:     $INSTALL_DIR/$WEB_ALIAS_NAME -> $COMMAND_NAME"
  echo
  echo "Installed support files:"
  echo "  Resources:    $RESOURCE_DIR"
  echo "  Env template: $ENV_TEMPLATE"
  echo "  Uninstaller:  $ROOT_DIR/uninstall.sh"
  echo
  echo "Start Timem Web (default):"
  echo "  1. Ensure $INSTALL_DIR is in PATH."
  echo "  2. Run: $COMMAND_NAME"
  echo "  3. Your browser should open automatically. Configure the model and API key in Timem Web, then start chatting."
  echo
  echo "No env file is required to open Timem Web."
  echo "For remote access on a trusted network, run: $COMMAND_NAME --public"
  echo
  echo "Optional terminal workflow:"
  echo "  Run: $COMMAND_NAME --shell"
  echo "  To provide environment defaults, copy $ENV_TEMPLATE to a private file, edit it, then source it before launch."
  echo
  echo "Update later from this git clone:"
  echo "  git pull --ff-only"
  echo "  ./install.sh"
}

main() {
  OS_KIND="$(detect_os)"
  ensure_build_dependencies "$OS_KIND"
  ensure_rust

  fetch_rust_dependencies
  build_release_binary

  mkdir -p "$INSTALL_DIR"
  install_binary_atomically "$ROOT_DIR/target/release/$COMMAND_NAME" "$INSTALL_DIR/$COMMAND_NAME"
  install_resource_atomically "$REMINDER_TIPS_SOURCE" "$RESOURCE_DIR/reminder_tips.json"

  install_web_alias "$INSTALL_DIR/$WEB_ALIAS_NAME"
  rm -f "$INSTALL_DIR/$OLD_BIN_NAME" "$INSTALL_DIR/$OLD_WRAPPER_NAME"
  chmod +x "$INSTALL_DIR/$COMMAND_NAME"

  print_install_success
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
