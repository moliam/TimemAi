#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "== shell scripts syntax =="
bash -n install.sh uninstall.sh scripts/bootstrap_assistant_ui.sh scripts/clippy_check.sh scripts/install_logic_test.sh scripts/sensitive_scan.sh scripts/test_contract_check.sh scripts/edge_regression.sh scripts/update_static_prompt_snapshot.sh scripts/kvc_replay_test.sh scripts/performance_guard.sh scripts/module_boundary_check.sh scripts/cross_host_resume_smoke.sh scripts/web_runtime_lifecycle_smoke.sh scripts/web_public_runtime_smoke.sh scripts/linux_web_platform_smoke.sh scripts/web_license_check.sh scripts/version_consistency_check.sh scripts/ci.sh
python3 -m py_compile scripts/fake_openai_server.py scripts/web_ui_matrix_check.py
python3 scripts/fake_openai_server.py --self-test

echo "== release version consistency =="
scripts/version_consistency_check.sh

echo "== module boundary =="
scripts/module_boundary_check.sh

echo "== install script logic =="
scripts/install_logic_test.sh

echo "== test contract check =="
scripts/test_contract_check.sh

echo "== Web UI feature/test matrix =="
python3 scripts/web_ui_matrix_check.py

echo "== static prompt snapshot =="
scripts/update_static_prompt_snapshot.sh --check

echo "== sensitive scan self-test =="
scripts/sensitive_scan.sh --self-test

echo "== sensitive scan: current tree =="
scripts/sensitive_scan.sh --current

echo "== kvc replay script =="
scripts/kvc_replay_test.sh

echo "== rust format =="
cargo fmt --all -- --check

echo "== rust clippy warnings =="
scripts/clippy_check.sh

echo "== rust tests =="
cargo test --workspace --locked -- --test-threads=1

if [[ "$(uname -s)" == "Linux" ]]; then
  echo "== Linux OS interface tests =="
  cargo test -p agent_core --lib --locked 'os::tests::linux_' -- --test-threads=1

  echo "== Linux run_bash supervision tests =="
  for test_name in \
    shell_lifecycle_validation_rejects_unmanaged_background_without_wait \
    shell_lifecycle_validation_rejects_explicit_detach \
    shutdown_and_session_cancel_refuse_pid_identity_mismatch \
    timeout_job_reports_pid_and_later_exit_update \
    timed_out_job_remains_cancellable_after_launcher_exits \
    watcher_waits_for_managed_process_group_after_launcher_exits \
    normal_bash_cancel_terminates_the_entire_process_group
  do
    cargo test -p agent_core --lib --locked \
      "shell_exec::tests::$test_name" -- --exact --test-threads=1
  done
fi

echo "== rust documentation =="
cargo doc --workspace --all-features --no-deps --locked

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

echo "== Web runtime lifecycle smoke =="
scripts/web_runtime_lifecycle_smoke.sh

echo "== Web public runtime smoke =="
scripts/web_public_runtime_smoke.sh

if [[ "$(uname -s)" == "Linux" ]]; then
  echo "== Linux Timem Web platform smoke =="
  scripts/linux_web_platform_smoke.sh
fi

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
