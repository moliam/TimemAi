#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "== shell scripts syntax =="
bash -n install.sh uninstall.sh scripts/install_logic_test.sh scripts/sensitive_scan.sh scripts/test_contract_check.sh scripts/edge_regression.sh scripts/ci.sh

echo "== install script logic =="
scripts/install_logic_test.sh

echo "== test contract check =="
scripts/test_contract_check.sh

echo "== sensitive scan: current tree =="
scripts/sensitive_scan.sh --current

echo "== rust format =="
cargo fmt --check

echo "== rust tests =="
cargo test --workspace

echo "== repeated edge regression =="
scripts/edge_regression.sh

echo "== release build =="
cargo build -p timem_shell --release

echo "== real TTY smoke =="
if command -v expect >/dev/null 2>&1; then
  scripts/real_tty_smoke.expect
else
  echo "error: expect is required for real TTY smoke" >&2
  exit 1
fi

echo "== whitespace check =="
git diff --check

echo "ci: ok"
