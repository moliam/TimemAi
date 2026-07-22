#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "module boundary: agent_core stays reusable"

if grep -RInE 'timem_shell|timem_web|web_ui' agent_core/Cargo.toml agent_core/src >/tmp/timem_module_boundary_core_refs.txt; then
  echo "error: agent_core must not reference a host UI" >&2
  cat /tmp/timem_module_boundary_core_refs.txt >&2
  exit 1
fi

if grep -RInE 'reedline|crossterm|termimad|nu-ansi-term|nu_ansi_term|syntect|unicode-width|unicode-segmentation' agent_core/Cargo.toml agent_core/src >/tmp/timem_module_boundary_ui_refs.txt; then
  echo "error: agent_core must not depend on terminal/UI rendering concepts" >&2
  cat /tmp/timem_module_boundary_ui_refs.txt >&2
  exit 1
fi

if ! grep -nF 'agent_core = { path = "../agent_core" }' timem_shell/Cargo.toml >/dev/null; then
  echo "error: timem_shell must depend on agent_core through the crate boundary" >&2
  exit 1
fi

if ! grep -nF 'agent_core = { path = "../agent_core" }' timem_web/Cargo.toml >/dev/null; then
  echo "error: timem_web must depend on agent_core through the crate boundary" >&2
  exit 1
fi

if ! test -f agent_core/module_boundary.md || ! test -f timem_shell/module_boundary.md || ! test -f timem_web/module_boundary.md || ! test -f web_ui/module_boundary.md; then
  echo "error: core, shell, Web host, and Web UI must keep module_boundary.md" >&2
  exit 1
fi

echo "module boundary: ok"
