#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "module boundary: semantic layout and reusable Core"
python3 scripts/architecture_guard.py

if grep -RInE 'timem_session|timem_in_process|timem_shell|timem_web|bridges/|interfaces/(shell|web)' core/agent/Cargo.toml core/agent/src >/tmp/timem_module_boundary_core_refs.txt; then
  echo "error: agent_core must not reference an Interface or host" >&2
  cat /tmp/timem_module_boundary_core_refs.txt >&2
  exit 1
fi

if grep -RInE 'reedline|crossterm|termimad|nu-ansi-term|nu_ansi_term|syntect|unicode-width|unicode-segmentation' core/agent/Cargo.toml core/agent/src >/tmp/timem_module_boundary_ui_refs.txt; then
  echo "error: agent_core must not depend on terminal/UI rendering concepts" >&2
  cat /tmp/timem_module_boundary_ui_refs.txt >&2
  exit 1
fi

python3 - <<'PY_BOUNDARY'
from pathlib import Path
checks = [(
    Path("timem_web/src"),
    {Path("timem_web/src/os/unix.rs")},
    ("libc::getppid", "tokio::signal::unix", "SignalKind::interrupt", "SignalKind::terminate", "SignalKind::hangup"),
    "Web parent-process and shutdown-signal primitives belong in timem_web/src/os/unix.rs",
)]
violations = []
for root, allowed, needles, message in checks:
    for path in root.rglob("*.rs"):
        if path in allowed:
            continue
        source = path.read_text(errors="replace")
        for needle in needles:
            if needle in source:
                violations.append(f"{message}: {path} contains {needle}")
if violations:
    raise SystemExit("\n".join(violations))
PY_BOUNDARY


if grep -nE 'agent_core|timem_platform|timem_shell|timem_web|host_projection|bridges/|interfaces/' core/ui_contract/Cargo.toml >/tmp/timem_module_boundary_ui_contract_refs.txt; then
  echo "error: core/ui_contract must remain a data-only inward contract" >&2
  cat /tmp/timem_module_boundary_ui_contract_refs.txt >&2
  exit 1
fi

if ! grep -nF 'agent_core = { path = "../../core/agent" }' bridges/in_process/Cargo.toml >/dev/null; then
  echo "error: bridges/in_process must depend inward on agent_core" >&2
  exit 1
fi

if grep -nE 'timem_shell|timem_web|host_projection|interfaces/' bridges/in_process/Cargo.toml >/tmp/timem_module_boundary_in_process_refs.txt; then
  echo "error: bridges/in_process must not depend on an Interface or host" >&2
  cat /tmp/timem_module_boundary_in_process_refs.txt >&2
  exit 1
fi

if ! grep -nF 'timem_in_process = { path = "../../bridges/in_process" }' interfaces/shell/Cargo.toml >/dev/null ||
   ! grep -nF 'run_in_process_turn(' interfaces/shell/src/main.rs >/dev/null; then
  echo "error: interfaces/shell must enter synchronous Turns through bridges/in_process" >&2
  exit 1
fi

if grep -nF 'run_session_turn(' interfaces/shell/src/main.rs >/tmp/timem_module_boundary_shell_direct_turn.txt; then
  echo "error: interfaces/shell must not bypass bridges/in_process for synchronous Turns" >&2
  cat /tmp/timem_module_boundary_shell_direct_turn.txt >&2
  exit 1
fi

if ! grep -nF 'agent_core = { path = "../agent" }' core/session/Cargo.toml >/dev/null ||
   ! grep -nF 'timem_ui_contract = { path = "../ui_contract" }' core/session/Cargo.toml >/dev/null; then
  echo "error: core/session must depend inward on Agent and UI contracts" >&2
  exit 1
fi

if grep -nE 'timem_shell|timem_web|host_projection|bridges/|interfaces/' core/session/Cargo.toml >/tmp/timem_module_boundary_session_refs.txt; then
  echo "error: core/session must not depend on a Bridge, Interface, or host" >&2
  cat /tmp/timem_module_boundary_session_refs.txt >&2
  exit 1
fi

if ! grep -nF 'agent_core = { path = "../core/agent" }' timem_web/Cargo.toml >/dev/null ||
   ! grep -nF 'timem_session = { path = "../core/session" }' timem_web/Cargo.toml >/dev/null; then
  echo "error: timem_web must depend on Agent and Session through their crate boundaries" >&2
  exit 1
fi

for boundary in bridges/in_process/module_boundary.md core/agent/module_boundary.md core/session/module_boundary.md core/platform/module_boundary.md core/ui_contract/module_boundary.md interfaces/shell/module_boundary.md interfaces/web/module_boundary.md timem_web/module_boundary.md; do
  test -f "$boundary" || { echo "error: missing module boundary: $boundary" >&2; exit 1; }
done

echo "module boundary: ok"
