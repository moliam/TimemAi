#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${TIMEM_SHELL_INSTALL_DIR:-$HOME/.local/bin}"
ENV_TEMPLATE="$ROOT_DIR/env_template"
BIN_NAME="timem-native-rs"
COMMAND_NAME="timem"
OLD_WRAPPER_NAME="timem-shell"
MIN_RUST_VERSION="1.78.0"

cd "$ROOT_DIR"

detect_os() {
  case "$(uname -s)" in
    Darwin) echo "macos" ;;
    Linux) echo "linux" ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      echo "unsupported_windows"
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
      if command -v cc >/dev/null 2>&1 && command -v make >/dev/null 2>&1 && command -v curl >/dev/null 2>&1; then
        return
      fi
      echo "Linux build tools are required: cc, make, curl."
      if command -v apt-get >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with apt-get..."
        install_with_sudo_if_available apt-get update
        install_with_sudo_if_available apt-get install -y build-essential curl ca-certificates
      elif command -v dnf >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with dnf..."
        install_with_sudo_if_available dnf install -y gcc gcc-c++ make curl ca-certificates
      elif command -v yum >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with yum..."
        install_with_sudo_if_available yum install -y gcc gcc-c++ make curl ca-certificates
      elif command -v pacman >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with pacman..."
        install_with_sudo_if_available pacman -Sy --needed --noconfirm base-devel curl ca-certificates
      elif command -v zypper >/dev/null 2>&1; then
        echo "Installing Linux build dependencies with zypper..."
        install_with_sudo_if_available zypper install -y gcc gcc-c++ make curl ca-certificates
      else
        echo "error: unsupported Linux package manager." >&2
        echo "Install cc, make, curl, and ca-certificates manually, then rerun ./install.sh." >&2
        exit 1
      fi
      if ! command -v cc >/dev/null 2>&1 || ! command -v make >/dev/null 2>&1 || ! command -v curl >/dev/null 2>&1; then
        echo "error: build dependencies are still missing after install attempt." >&2
        echo "Install cc, make, curl, and ca-certificates manually, then rerun ./install.sh." >&2
        exit 1
      fi
      ;;
    unsupported_windows)
      echo "error: Windows is not supported yet. Timem shell currently supports macOS and Linux." >&2
      exit 1
      ;;
    *)
      echo "error: unsupported OS. Timem shell currently supports macOS and Linux." >&2
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

main() {
  OS_KIND="$(detect_os)"
  ensure_build_dependencies "$OS_KIND"
  ensure_rust

  cargo build -p timem_shell --release

  mkdir -p "$INSTALL_DIR"
  cp "$ROOT_DIR/target/release/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"

  cat > "$INSTALL_DIR/$COMMAND_NAME" <<SH
#!/usr/bin/env bash
set -euo pipefail

exec "\$(dirname "\$0")/timem-native-rs" "\$@"
SH
  rm -f "$INSTALL_DIR/$OLD_WRAPPER_NAME"
  chmod +x "$INSTALL_DIR/$BIN_NAME" "$INSTALL_DIR/$COMMAND_NAME"

  echo "Installed:"
  echo "  binary:  $INSTALL_DIR/$BIN_NAME"
  echo "  command: $INSTALL_DIR/$COMMAND_NAME"
  echo "  env template: $ENV_TEMPLATE"
  echo "  uninstall: $ROOT_DIR/uninstall.sh"
  echo
  echo "Next steps:"
  echo "  1. Create a private env file from the template:"
  echo "     cp $ENV_TEMPLATE $ROOT_DIR/env"
  echo "  2. Edit $ROOT_DIR/env and uncomment the values you need."
  echo "  3. Ensure $INSTALL_DIR is in PATH."
  echo "  4. Load env: source /path/to/your/env"
  echo "  5. Run: $COMMAND_NAME"
  echo
  echo "Update later from this git clone:"
  echo "  git pull --ff-only"
  echo "  ./install.sh"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
