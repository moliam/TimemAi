#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "module boundary: semantic layout and reusable Core"
python3 scripts/architecture_guard.py

if grep -RInE 'timem_shell|timem_web|interfaces/(shell|web)' agent_core/Cargo.toml agent_core/src >/tmp/timem_module_boundary_core_refs.txt; then
  echo "error: agent_core must not reference an Interface or host" >&2
  cat /tmp/timem_module_boundary_core_refs.txt >&2
  exit 1
fi

if grep -RInE 'reedline|crossterm|termimad|nu-ansi-term|nu_ansi_term|syntect|unicode-width|unicode-segmentation' agent_core/Cargo.toml agent_core/src >/tmp/timem_module_boundary_ui_refs.txt; then
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

if ! grep -nF 'agent_core = { path = "../agent_core" }' timem_web/Cargo.toml >/dev/null; then
  echo "error: timem_web must depend on agent_core through the crate boundary" >&2
  exit 1
fi

for boundary in agent_core/module_boundary.md core/platform/module_boundary.md interfaces/shell/module_boundary.md interfaces/web/module_boundary.md timem_web/module_boundary.md; do
  test -f "$boundary" || { echo "error: missing module boundary: $boundary" >&2; exit 1; }
done

echo "module boundary: ok"
