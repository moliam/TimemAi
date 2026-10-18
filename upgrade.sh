#!/usr/bin/env bash
# One-command online upgrade for TimemAi (macOS/Linux).
# Downloads the prebuilt release archive for this platform, verifies its
# checksum, and replaces the installed timem executable in place. MEM
# workspaces, Sessions, credentials, and user configuration are never touched.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/moliam/TimemAi/main/upgrade.sh | bash
# Pin a version:
#   curl -fsSL ... | TIMEM_VERSION=v2.5.0 bash
set -euo pipefail

REPOSITORY="${TIMEM_INSTALL_REPOSITORY:-moliam/TimemAi}"
REQUESTED_VERSION="${TIMEM_VERSION:-latest}"
CURL_BIN="${TIMEM_INSTALL_CURL:-curl}"

die() {
  echo "error: $*" >&2
  exit 1
}

validate_repository() {
  case "$1" in
    *[!A-Za-z0-9_.\/-]*|/*|*/|*//*|*/*/*|"")
      die "invalid GitHub repository: $1"
      ;;
  esac
}

validate_version() {
  case "$1" in
    *[!A-Za-z0-9._-]*|"")
      die "invalid release version: $1"
      ;;
  esac
}

detect_target() {
  if [ -n "${TIMEM_TARGET:-}" ]; then
    case "$TIMEM_TARGET" in
      x86_64-pc-windows-msvc|aarch64-apple-darwin|x86_64-apple-darwin|x86_64-unknown-linux-gnu)
        printf '%s' "$TIMEM_TARGET"
        return 0
        ;;
      *) return 1 ;;
    esac
  fi
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os:$arch" in
    Darwin:arm64|Darwin:aarch64) printf 'aarch64-apple-darwin' ;;
    Darwin:x86_64) printf 'x86_64-apple-darwin' ;;
    Linux:x86_64|Linux:amd64) printf 'x86_64-unknown-linux-gnu' ;;
    *) return 1 ;;
  esac
}

resolve_version() {
  if [ "$REQUESTED_VERSION" != "latest" ]; then
    validate_version "$REQUESTED_VERSION"
    printf '%s' "$REQUESTED_VERSION"
    return 0
  fi
  local url
  url="$("$CURL_BIN" -fsSL --location --output /dev/null --write-out '%{url_effective}' \
    "https://github.com/$REPOSITORY/releases/latest")" || die "failed to resolve the latest release"
  local resolved
  resolved="$(printf '%s' "$url" | awk -F/ '{ print $NF }')"
  case "$resolved" in
    v*) printf '%s' "$resolved" ;;
    *) die "GitHub did not resolve a latest release tag (got: $resolved)" ;;
  esac
}

checksum_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{ print $1 }'
  else
    die "neither sha256sum nor shasum is available"
  fi
}

cleanup() {
  if [ -n "${UPGRADE_TMP_DIR:-}" ] && [ -d "$UPGRADE_TMP_DIR" ]; then
    rm -rf "$UPGRADE_TMP_DIR"
  fi
}

main() {
  validate_repository "$REPOSITORY"

  local target version
  target="$(detect_target)" || die "unsupported platform: $(uname -s) $(uname -m).
Prebuilt archives cover x86_64-pc-windows-msvc, aarch64-apple-darwin,
x86_64-apple-darwin, and x86_64-unknown-linux-gnu. On other platforms upgrade
from source: https://github.com/$REPOSITORY#install-or-update"
  version="$(resolve_version)"

  local base asset archive sidecar expected actual
  base="https://github.com/$REPOSITORY/releases/download/$version"
  asset="timem-$version-$target.tar.gz"

  UPGRADE_TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/timem-upgrade.XXXXXX")"
  trap cleanup EXIT
  archive="$UPGRADE_TMP_DIR/$asset"
  sidecar="$archive.sha256"

  echo "Downloading TimemAi $version ($target)..."
  "$CURL_BIN" -fsSL --location --output "$archive" "$base/$asset" \
    || die "failed to download $base/$asset"
  "$CURL_BIN" -fsSL --location --output "$sidecar" "$base/$asset.sha256" \
    || die "failed to download $base/$asset.sha256"

  expected="$(awk 'NR==1 { print $1 }' "$sidecar")"
  case "$expected" in
    *[!A-Fa-f0-9]*|"") die "malformed checksum sidecar for $asset" ;;
  esac
  actual="$(checksum_of "$archive")"
  if [ "$expected" != "$actual" ]; then
    die "checksum mismatch for $asset
expected: $expected
actual:   $actual"
  fi
  echo "Checksum verified."

  mkdir "$UPGRADE_TMP_DIR/extract"
  tar -xzf "$archive" -C "$UPGRADE_TMP_DIR/extract" || die "failed to extract $asset"
  local staged="$UPGRADE_TMP_DIR/extract/timem"
  [ -f "$staged" ] || die "archive does not contain a timem executable"

  local current dest_dir stamp backup
  current="${TIMEM_UPGRADE_BIN_DIR:-}"
  if [ -z "$current" ]; then
    current="$(command -v timem 2>/dev/null || true)"
  fi
  [ -n "$current" ] || die "timem is not installed on PATH.
Install first: curl -fsSL https://raw.githubusercontent.com/moliam/TimemAi/main/install.sh | bash"
  dest_dir="$(dirname "$current")"
  [ -x "$dest_dir/timem" ] || die "$dest_dir/timem is not an executable; refusing to upgrade"

  stamp="$(date +%Y%m%d-%H%M%S)"
  backup="$dest_dir/timem.bak.$stamp"
  cp -p "$dest_dir/timem" "$backup" || die "failed to back up the current executable"

  local pending
  pending="$dest_dir/.timem.upgrade.$stamp"
  rm -f "$pending"
  cp "$staged" "$pending" && chmod 755 "$pending" && mv -f "$pending" "$dest_dir/timem" \
    || die "failed to replace $dest_dir/timem"

  if [ -L "$dest_dir/timem-web" ] || [ ! -e "$dest_dir/timem-web" ]; then
    ln -sfn timem "$dest_dir/timem-web" || true
  fi
  if [ -f "$dest_dir/../share/timem/resources/reminder_tips.json" ]; then
    if [ -f "$UPGRADE_TMP_DIR/extract/reminder_tips.json" ]; then
      cp "$UPGRADE_TMP_DIR/extract/reminder_tips.json" \
        "$dest_dir/../share/timem/resources/reminder_tips.json.tmp.$$" \
        && mv -f "$dest_dir/../share/timem/resources/reminder_tips.json.tmp.$$" \
        "$dest_dir/../share/timem/resources/reminder_tips.json" || true
    fi
  fi

  if "$dest_dir/timem" --help >/dev/null 2>&1; then
    echo "TimemAi upgraded to $version."
    echo "  binary:  $dest_dir/timem"
    echo "  backup:  $backup"
    echo "User data (MEM workspaces, Sessions, credentials, configuration) is untouched."
    echo "If Timem is running, restart it to use the new version."
  else
    echo "verification failed; rolling back" >&2
    cp -p "$backup" "$dest_dir/timem" || die "rollback failed; the backup remains at $backup"
    die "the downloaded executable failed its post-install check; rolled back to the previous version"
  fi
}

main
