#!/usr/bin/env bash
# Package prebuilt Timem release archives (zip/tar.gz + .sha256 sidecar).
# Usage:
#   scripts/package_release_assets.sh --self-test
#   scripts/package_release_assets.sh <target-triple> <release-bin-dir> <out-dir>
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat >&2 <<'USAGE'
usage:
  scripts/package_release_assets.sh --self-test
  scripts/package_release_assets.sh <target-triple> <release-bin-dir> <out-dir>
USAGE
  exit 2
}

workspace_version() {
  local version
  version="$(sed -nE 's/^version = "([^"]+)"$/\1/p' Cargo.toml | head -n 1)"
  if [ -z "$version" ]; then
    echo "unable to read workspace version from Cargo.toml" >&2
    exit 1
  fi
  printf '%s' "$version"
}

pick_python() {
  if command -v python3 >/dev/null 2>&1; then command -v python3
  elif command -v python >/dev/null 2>&1; then command -v python
  else
    echo "python3 (or python) is required to create zip archives" >&2
    exit 1
  fi
}

make_zip() {
  local directory="$1" out="$2"
  shift 2
  local py
  py="$(pick_python)"
  (
    cd "$directory"
    "$py" - "$out" "$@" <<'PYEOF'
import sys
import zipfile

out, names = sys.argv[1], sys.argv[2:]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
    for name in names:
        archive.write(name, name)
PYEOF
  )
}

checksum_file() {
  local target_file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$target_file"
  else
    shasum -a 256 "$target_file"
  fi
}

stage_common() {
  local stage="$1" bin_dir="$2"
  case "$3" in
    windows)
      if [ ! -f "$bin_dir/timem.exe" ]; then
        echo "missing $bin_dir/timem.exe (build first: cargo build --locked --release --bin timem --target $4)" >&2
        exit 1
      fi
      cp "$bin_dir/timem.exe" "$stage/timem.exe"
      printf '@echo off\r\n"%%~dp0timem.exe" %%*\r\nexit /b %%ERRORLEVEL%%\r\n' > "$stage/timem-web.cmd"
      ;;
    unix)
      if [ ! -f "$bin_dir/timem" ]; then
        echo "missing $bin_dir/timem (build first: cargo build --locked --release --bin timem --target $4)" >&2
        exit 1
      fi
      cp "$bin_dir/timem" "$stage/timem"
      chmod +x "$stage/timem"
      ln -s timem "$stage/timem-web"
      ;;
  esac
  cp resources/reminder_tips.json "$stage/reminder_tips.json"
}

write_readme() {
  local stage="$1" version="$2"
  cat > "$stage/README-ASSETS.txt" <<EOF
TimemAi $version prebuilt package

Contents:
- timem (timem.exe on Windows): the unified Timem executable (Web is the
  default mode; --shell selects the terminal UI)
- timem-web: compatibility alias forwarding to timem (symlink on Unix,
  timem-web.cmd on Windows)
- reminder_tips.json: shipped default reminder schedules; user-level overrides
  under the OS configuration directory are never touched

Install:
1. Extract this archive anywhere, for example ~/timem-bin.
2. Add that directory to PATH.
3. Run: timem    (the browser opens automatically)
   Terminal mode: timem --shell

Update: open Settings -> System in the Web UI and run the built-in upgrade
(Windows replaces the running executable automatically). Alternatively exit
Timem and extract a newer archive over the old directory. MEM workspaces
(~/.timem), Sessions, credentials, and user configuration are preserved.

Verify integrity against the .sha256 sidecar next to this archive.
EOF
}

package() {
  local target="$1" bin_dir="$2" out_dir="$3"
  local version stage asset sumfile digest kind
  version="$(workspace_version)"
  mkdir -p "$out_dir"
  # Absolute-ize early: tar/zip run from inside the staging directory and a
  # relative out_dir would resolve the archive path under .stage-* instead.
  out_dir="$(cd "$out_dir" && pwd)"
  stage="$out_dir/.stage-$target"
  rm -rf "$stage"
  mkdir -p "$stage"

  case "$target" in
    *-windows-*)
      kind="windows"
      stage_common "$stage" "$bin_dir" "$kind" "$target"
      write_readme "$stage" "$version"
      assets="$out_dir/timem-v$version-$target.zip $out_dir/timem-v$version-$target.tar.gz"
      make_zip "$stage" "$out_dir/timem-v$version-$target.zip" timem.exe timem-web.cmd reminder_tips.json README-ASSETS.txt
      (cd "$stage" && tar -czf "$out_dir/timem-v$version-$target.tar.gz" timem.exe timem-web.cmd reminder_tips.json README-ASSETS.txt)
      ;;
    *-apple-darwin | *-linux-*)
      kind="unix"
      stage_common "$stage" "$bin_dir" "$kind" "$target"
      write_readme "$stage" "$version"
      assets="$out_dir/timem-v$version-$target.tar.gz"
      (cd "$stage" && tar -czf "$out_dir/timem-v$version-$target.tar.gz" timem timem-web reminder_tips.json README-ASSETS.txt)
      ;;
    *)
      echo "unsupported target triple: $target" >&2
      exit 1
      ;;
  esac

  for asset in $assets; do
    digest="$(cd "$out_dir" && checksum_file "$(basename "$asset")")"
    sumfile="$asset.sha256"
    printf '%s\n' "$digest" > "$sumfile"
    echo "packaged: $asset"
    echo "checksum: $sumfile"
  done
  rm -rf "$stage"
}

self_test() {
  local tmp py
  py="$(pick_python)"
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/timem-release-assets-test.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT

  mkdir -p "$tmp/windows-bin" "$tmp/unix-bin" "$tmp/out"
  printf 'fake windows binary\r\n' > "$tmp/windows-bin/timem.exe"
  printf '#!/bin/sh\necho fake unix binary\n' > "$tmp/unix-bin/timem"

  package x86_64-pc-windows-msvc "$tmp/windows-bin" "$tmp/out" >/dev/null
  package aarch64-apple-darwin "$tmp/unix-bin" "$tmp/out" >/dev/null

  local version zip_asset tar_asset
  version="$(workspace_version)"
  zip_asset="$tmp/out/timem-v$version-x86_64-pc-windows-msvc.zip"
  tar_asset="$tmp/out/timem-v$version-aarch64-apple-darwin.tar.gz"

  "$py" - "$zip_asset" <<'PYEOF'
import sys
import zipfile

expected = {"timem.exe", "timem-web.cmd", "reminder_tips.json", "README-ASSETS.txt"}
with zipfile.ZipFile(sys.argv[1]) as archive:
    names = set(archive.namelist())
if names != expected:
    raise SystemExit(f"zip member mismatch: {sorted(names)}")
PYEOF

  local tar_names
  tar_names="$(tar -tzf "$tar_asset" | sort | tr '\n' ' ')"
  if [ "$tar_names" != "README-ASSETS.txt reminder_tips.json timem timem-web " ]; then
    echo "tar member mismatch: $tar_names" >&2
    exit 1
  fi

  local win_tar_names
  win_tar_names="$(tar -tzf "$tmp/out/timem-v$version-x86_64-pc-windows-msvc.tar.gz" | sort | tr '\n' ' ')"
  if [ "$win_tar_names" != "README-ASSETS.txt reminder_tips.json timem-web.cmd timem.exe " ]; then
    echo "windows tar member mismatch: $win_tar_names" >&2
    exit 1
  fi

  local expected_digest actual_digest
  expected_digest="$(awk '{print $1}' "$zip_asset.sha256")"
  actual_digest="$(cd "$tmp/out" && checksum_file "$(basename "$zip_asset")" | awk '{print $1}')"
  if [ "$expected_digest" != "$actual_digest" ]; then
    echo "sha256 sidecar mismatch for $zip_asset" >&2
    exit 1
  fi

  if [ -e "$tmp/out/.stage-x86_64-pc-windows-msvc" ] || [ -e "$tmp/out/.stage-aarch64-apple-darwin" ]; then
    echo "packaging left staging directories behind" >&2
    exit 1
  fi

  rm -rf "$tmp"
  trap - EXIT
  echo "package_release_assets self-test: ok"
}

if [ "${1:-}" = "--self-test" ]; then
  self_test
elif [ $# -eq 3 ]; then
  package "$1" "$2" "$3"
else
  usage
fi
