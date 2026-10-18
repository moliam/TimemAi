#!/usr/bin/env bash
# Offline logic tests for upgrade.sh: no network access is performed. The
# "download" is served by a fake curl from a fixture archive packaged by
# scripts/package_release_assets.sh, so the exercised path is the real
# packaging layout.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

test_root="$(mktemp -d "${TMPDIR:-/tmp}/timem-upgrade-test.XXXXXX")"
trap 'rm -rf "$test_root"' EXIT

fail() {
  echo "upgrade_logic_test failed: $*" >&2
  exit 1
}

# --- fixture: a real package produced by the release packager ---
version="$(sed -nE 's/^version = "([^"]+)"$/\1/p' Cargo.toml | head -n 1)"
target="aarch64-apple-darwin"
asset="timem-v$version-$target.tar.gz"

mkdir -p "$test_root/fake-release-build"
printf '#!/bin/sh\necho fixture-timem-new\n' > "$test_root/fake-release-build/timem"
chmod +x "$test_root/fake-release-build/timem"
scripts/package_release_assets.sh "$target" "$test_root/fake-release-build" "$test_root/release" >/dev/null \
  || fail "packager could not build the fixture archive"
[ -f "$test_root/release/$asset" ] || fail "fixture archive missing"

# --- fake curl: serves fixture files for archive and sidecar requests ---
cat > "$test_root/fake-curl" <<CURL
#!/usr/bin/env bash
set -euo pipefail
output=""
url=""
while [ \$# -gt 0 ]; do
  case "\$1" in
    --output) output="\$2"; shift 2 ;;
    *) url="\$1"; shift ;;
  esac
done
case "\$url" in
  *.tar.gz) cp "$test_root/release/$asset" "\$output" ;;
  *.tar.gz.sha256) cp "$test_root/release/$asset.sha256" "\$output" ;;
  *) echo "fake-curl: unexpected url \$url" >&2; exit 64 ;;
esac
CURL
chmod +x "$test_root/fake-curl"

# --- common environment ---
export TIMEM_INSTALL_REPOSITORY="moliam/TimemAi"
export TIMEM_INSTALL_CURL="$test_root/fake-curl"
export TIMEM_VERSION="v$version"
export TIMEM_TARGET="$target"

make_fake_install() {
  local dir="$1"
  mkdir -p "$dir"
  printf '#!/bin/sh\necho fixture-timem-old\n' > "$dir/timem"
  chmod 755 "$dir/timem"
}

run_upgrade() {
  env PATH="$test_root/fake-bin:$PATH" bash "$ROOT_DIR/upgrade.sh" 2>&1
}

# Scenario 1: normal upgrade replaces the binary and keeps a backup.
mkdir -p "$test_root/fake-bin"
make_fake_install "$test_root/fake-bin"
export TIMEM_UPGRADE_BIN_DIR="$test_root/fake-bin/timem"
out="$(run_upgrade)" || fail "scenario 1 (normal upgrade): exit code $?"
echo "$out" | grep -q "TimemAi upgraded to v$version" || fail "scenario 1: success message missing"
[ "$(cat "$test_root/fake-bin/timem")" = "$(printf '#!/bin/sh\necho fixture-timem-new\n')" ] \
  || fail "scenario 1: binary was not replaced"
ls "$test_root/fake-bin"/timem.bak.* >/dev/null 2>&1 || fail "scenario 1: backup missing"

# Scenario 2: checksum mismatch aborts without touching the installation.
make_fake_install "$test_root/fake-bin"
printf '0000000000000000000000000000000000000000000000000000000000000000  %s\n' "$asset" \
  > "$test_root/release/$asset.sha256"
out="$(run_upgrade)" && fail "scenario 2: tampered checksum must fail" || true
echo "$out" | grep -q "checksum mismatch" || fail "scenario 2: mismatch not reported"
grep -q "fixture-timem-old" "$test_root/fake-bin/timem" || fail "scenario 2: installation was modified"
scripts/package_release_assets.sh "$target" "$test_root/fake-release-build" "$test_root/release" >/dev/null

# Scenario 3: a broken downloaded binary triggers rollback.
make_fake_install "$test_root/fake-bin"
printf '#!/bin/sh\nexit 42\n' > "$test_root/fake-release-build/timem"
chmod +x "$test_root/fake-release-build/timem"
scripts/package_release_assets.sh "$target" "$test_root/fake-release-build" "$test_root/release" >/dev/null
out="$(run_upgrade)" && fail "scenario 3: broken binary must fail" || true
echo "$out" | grep -q "rolling back" || fail "scenario 3: rollback not reported"
grep -q "fixture-timem-old" "$test_root/fake-bin/timem" || fail "scenario 3: rollback did not restore"

# Scenario 4: no existing installation refuses to run.
export TIMEM_UPGRADE_BIN_DIR=""
out="$(env PATH="$test_root/empty:/usr/bin:/bin" bash "$ROOT_DIR/upgrade.sh" 2>&1)" \
  && fail "scenario 4: missing installation must fail" || true
echo "$out" | grep -q "timem is not installed" || fail "scenario 4: missing installation not reported"

# Scenario 5: unsupported target triple refuses to run.
export TIMEM_UPGRADE_BIN_DIR="$test_root/fake-bin/timem"
out="$(env TIMEM_TARGET=riscv64-unknown-none PATH="$test_root/fake-bin:$PATH" bash "$ROOT_DIR/upgrade.sh" 2>&1)" \
  && fail "scenario 5: unsupported target must fail" || true
echo "$out" | grep -q "unsupported platform" || fail "scenario 5: unsupported target not reported"

echo "upgrade_logic_test: ok"
