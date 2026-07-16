#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "== shell scripts syntax =="
bash -n install.sh uninstall.sh scripts/bootstrap_assistant_ui.sh scripts/install_logic_test.sh scripts/sensitive_scan.sh scripts/test_contract_check.sh scripts/edge_regression.sh scripts/update_static_prompt_snapshot.sh scripts/kvc_replay_test.sh scripts/performance_guard.sh scripts/module_boundary_check.sh scripts/cross_host_resume_smoke.sh scripts/web_license_check.sh scripts/ci.sh
python3 -m py_compile scripts/fake_openai_provider.py

echo "== module boundary =="
scripts/module_boundary_check.sh

echo "== install script logic =="
scripts/install_logic_test.sh

echo "== test contract check =="
scripts/test_contract_check.sh

echo "== static prompt snapshot =="
scripts/update_static_prompt_snapshot.sh --check

echo "== sensitive scan: current tree =="
scripts/sensitive_scan.sh --current

echo "== kvc replay script =="
scripts/kvc_replay_test.sh

echo "== rust format =="
cargo fmt --check

echo "== rust tests =="
cargo test --workspace

echo "== web dependencies =="
pnpm --dir web_ui/timem-web install --frozen-lockfile

echo "== web dependency licenses =="
scripts/web_license_check.sh

echo "== web tests =="
pnpm --dir web_ui/timem-web test

echo "== web production build =="
pnpm --dir web_ui/timem-web build
git diff --exit-code -- web_ui/timem-web/dist

echo "== performance guard =="
scripts/performance_guard.sh

echo "== repeated edge regression =="
scripts/edge_regression.sh

echo "== release build =="
cargo build --locked -p timem_shell -p timem_web --release

echo "== cross-host resume smoke =="
scripts/cross_host_resume_smoke.sh

echo "== real TTY smoke =="
if command -v expect >/dev/null 2>&1; then
  scripts/real_tty_smoke.expect
  scripts/real_tty_supplement_smoke.expect
  scripts/real_tty_stress.expect
else
  echo "error: expect is required for real TTY smoke" >&2
  exit 1
fi

echo "== whitespace check =="
git diff --check

echo "ci: ok"
