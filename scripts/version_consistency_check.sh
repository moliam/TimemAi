#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

workspace_version="$(sed -nE 's/^version = "([^"]+)"$/\1/p' Cargo.toml | head -n 1)"
frontend_version="$(node -p "require('./interfaces/web/package.json').version")"

if [ -z "$workspace_version" ]; then
  echo "unable to read workspace package version from Cargo.toml" >&2
  exit 1
fi

if [ "$workspace_version" != "$frontend_version" ]; then
  echo "version mismatch: Cargo workspace=$workspace_version frontend=$frontend_version" >&2
  exit 1
fi

if [ ! -f "docs/release-notes-v${workspace_version}.md" ]; then
  echo "missing release notes: docs/release-notes-v${workspace_version}.md" >&2
  exit 1
fi
if ! grep -Fq "## [${workspace_version}]" CHANGELOG.md; then
  echo "CHANGELOG.md does not contain release ${workspace_version}" >&2
  exit 1
fi

for package in agent_core timem_shell timem; do
  if ! awk -v package="$package" -v version="$workspace_version" '
    $0 == "name = \"" package "\"" { found = 1; in_package = 1; next }
    in_package && /^name = / { in_package = 0 }
    in_package && $0 == "version = \"" version "\"" { matched = 1 }
    END { exit !(found && matched) }
  ' Cargo.lock; then
    echo "Cargo.lock does not record $package at workspace version $workspace_version" >&2
    exit 1
  fi
done

echo "version_consistency: ok ($workspace_version)"
