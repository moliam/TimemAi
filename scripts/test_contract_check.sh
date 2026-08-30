#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

search_regex() {
  local pattern="$1"
  shift
  if command -v rg >/dev/null 2>&1; then
    rg -q -- "$pattern" "$@"
  else
    grep -R -q --exclude-dir=target --exclude-dir=.git -- "$pattern" "$@"
  fi
}

search_fixed() {
  local pattern="$1"
  shift
  if command -v rg >/dev/null 2>&1; then
    rg -q -F -- "$pattern" "$@"
  else
    grep -R -F -q --exclude-dir=target --exclude-dir=.git -- "$pattern" "$@"
  fi
}

search_lines_regex() {
  local pattern="$1"
  shift
  if command -v rg >/dev/null 2>&1; then
    rg -n -- "$pattern" "$@"
  else
    grep -R -n -E --exclude-dir=target --exclude-dir=.git -- "$pattern" "$@"
  fi
}

# This script checks repository/CI contracts. Behavioral coverage belongs to
# executable tests; test names and source-file placement are not evidence.

ci_required=(
  "cargo test --workspace"
  "pnpm --dir interfaces/web test"
  "pnpm --dir interfaces/web build"
  "pnpm --dir interfaces/web test:browser"
  "cargo build --locked --release --bin timem"
  "scripts/edge_regression.sh"
  "scripts/real_tty_smoke.expect"
  "scripts/real_tty_supplement_smoke.expect"
  "python3 scripts/runtime_io_guard.py"
  "scripts/sensitive_scan.sh --current"
  "python3 scripts/web_ui_matrix_check.py"
  "scripts/update_static_prompt_snapshot.sh --check"
  "scripts/clippy_check.sh"
  "scripts/performance_guard.sh"
  "scripts/cross_host_resume_smoke.sh"
  "scripts/web_license_check.sh"
  "scripts/version_consistency_check.sh"
  "python3 scripts/architecture_guard.py --self-test"
  "scripts/module_boundary_check.sh"
)

for pattern in "${ci_required[@]}"; do
  if ! search_fixed "$pattern" scripts/ci.sh; then
    echo "missing required CI gate: $pattern" >&2
    exit 1
  fi
done

windows_ci_required=(
  "runs-on: windows-latest"
  "./scripts/windows_install_logic_test.ps1"
  "cargo check --workspace --all-targets --locked"
  "cargo test --workspace --locked -- --test-threads=1"
  "cache-dependency-path: interfaces/web/pnpm-lock.yaml"
  "pnpm --dir interfaces/web test"
  "pnpm --dir interfaces/web build"
  "git diff --exit-code -- interfaces/web/dist"
  "cargo build --locked --release --bin timem"
)
for pattern in "${windows_ci_required[@]}"; do
  if ! search_fixed "$pattern" .github/workflows/ci.yml; then
    echo "missing required Windows CI gate: $pattern" >&2
    exit 1
  fi
done
if search_fixed "web_ui/timem-web" .github/workflows/ci.yml; then
  echo "Windows CI must use the semantic interfaces/web layout" >&2
  exit 1
fi


runtime_io_guard_required=(
  "DEFAULT_LIMIT_BPS = 500_000"
  '"workload": "real_tty_stress"'
  "TIMEM_RUNTIME_IO_START_FILE"
  "TIMEM_RUNTIME_IO_END_FILE"
  "TEMPORARY_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60)"
  "TEMPORARY_MAINTENANCE_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(15 * 60)"
  "temporary_maintenance_state.json"
  "api_audit_maintenance_hint_path"
  "spawn_idle_temporary_maintenance_loop"
  "has_live_mem_work"
  "mem_temporary_items_list"
)
for pattern in "${runtime_io_guard_required[@]}"; do
  if ! search_fixed "$pattern" scripts/runtime_io_guard.py scripts/real_tty_stress.expect applications/timem/src/server.rs interfaces/web/src/main.tsx; then
    echo "missing Timem runtime I/O guard contract: $pattern" >&2
    exit 1
  fi
done

shell_lib_forbidden_wrappers=(
  "pub fn audit_path("
  "pub fn action_audit_path("
  "pub fn memory_path("
  "pub fn data_root("
  "pub fn workspace_config_path("
  "pub fn load_workspace_dirs("
  "pub fn save_workspace_dirs("
  "pub fn supporting_context("
)

for pattern in "${shell_lib_forbidden_wrappers[@]}"; do
  if search_fixed "$pattern" interfaces/shell/src/lib.rs; then
    echo "timem_shell must not re-expose core runtime layout/context wrapper: $pattern" >&2
    exit 1
  fi
done

shell_lib_forbidden_core_internals=(
  "prepare_model_request"
  "prepare_model_http_request"
  "model_http_error_message"
  "prompt_cache_plan_audit"
  "plan_prompt_cache"
  "plan_incremental_cache"
  "prompt_parts_from_rendered_prompt"
  "split_old_and_new_delta"
  "split_prompt"
  "stable_text_fingerprint"
  "CacheControl"
  "PromptBlock"
  "StructuredOutputHint"
)

for pattern in "${shell_lib_forbidden_core_internals[@]}"; do
  if search_fixed "$pattern" interfaces/shell/src/lib.rs; then
    echo "timem_shell must not re-export model cache core internals: $pattern" >&2
    exit 1
  fi
done

shell_src_forbidden_execution=(
  'Command::new("curl")'
  'Command::new("/bin/sh")'
  "call_model_with_cancel"
  "run_command_with_cancel"
  "execute_one_bash"
)

for pattern in "${shell_src_forbidden_execution[@]}"; do
  if search_fixed "$pattern" interfaces/shell/src; then
    echo "timem_shell must not implement model transport/tool execution: $pattern" >&2
    exit 1
  fi
done

for file in resources/capabilities/tools/*.yaml; do
  if awk '/^example_json: \|/{in_example=1; next} /^kind: /{in_example=0} in_example && /"(action|args|input)"[[:space:]]*:/{found=1} END{exit found ? 0 : 1}' "$file"; then
    echo "tool manifest example_json must use single-key tool objects, not action/args/input: $file" >&2
    exit 1
  fi
  tool_id="$(awk '/^id: /{print $2; exit}' "$file")"
  if [ -n "$tool_id" ] && ! awk -v id="$tool_id" '/^example_json: \|/{in_example=1; next} /^kind: /{in_example=0} in_example && $0 ~ "\"" id "\"[[:space:]]*:"{found=1} END{exit found ? 0 : 1}' "$file"; then
    echo "tool manifest example_json must include its tool id as the action object key: $file" >&2
    exit 1
  fi
done

if search_lines_regex '"(action|args)"[[:space:]]*:' README.md; then
  echo "README action examples must use current single-key tool objects, not action/args:" >&2
  search_lines_regex '"(action|args)"[[:space:]]*:' README.md >&2
  exit 1
fi

if search_regex '(^|[^<])!\[CDATA\[' resources/protocol/xml; then
  echo "XML protocol docs must spell CDATA as <![CDATA[, not ![CDATA[:" >&2
  search_lines_regex '(^|[^<])!\[CDATA\[' resources/protocol/xml >&2
  exit 1
fi

legacy_action_input_hits="$(
  search_lines_regex 'next_actions.*"input"[[:space:]]*:' \
    core/agent/tests core/session/tests core/agent/src/session_runtime.rs interfaces/shell/src/observation.rs interfaces/shell/src/lib.rs \
    | grep -v 'allow_legacy_input_negative_test' || true
)"
if [ -n "$legacy_action_input_hits" ]; then
  echo "mock model outputs must use args, not legacy input:" >&2
  echo "$legacy_action_input_hits" >&2
  exit 1
fi
if ! search_fixed "allow_legacy_input_negative_test" core/agent/tests/core_tests.rs; then
  echo "missing explicit negative test marker for legacy input rejection" >&2
  exit 1
fi
string_args_hits="$(
  search_lines_regex '"args"[[:space:]]*:[[:space:]]*"' \
    core/agent/tests core/session/tests core/agent/src/session_runtime.rs interfaces/shell/src/observation.rs interfaces/shell/src/lib.rs resources docs README.md CHANGELOG.md scripts \
    | grep -v 'allow_string_args_negative_test' \
    | grep -v 'response_schema_summary.json' \
    || true
)"
if [ -n "$string_args_hits" ]; then
  echo "mock model outputs and docs must use object args, not string args:" >&2
  echo "$string_args_hits" >&2
  exit 1
fi
if ! search_fixed "allow_string_args_negative_test" core/agent/tests/core_tests.rs; then
  echo "missing explicit negative test marker for string args rejection" >&2
  exit 1
fi

private_fixture_hits="$(
  search_lines_regex '默默|李默|儿子|son birthday|6月12|蓝色雨伞|绿色雨衣|fangchang|/Users/limo3|/Users/fangchang|v0\.6 发布检查|AURORA' \
    core/agent/tests core/session/tests interfaces/shell/src resources docs README.md CHANGELOG.md scripts \
    | grep -v 'scripts/test_contract_check.sh' \
    || true
)"
if [ -n "$private_fixture_hits" ]; then
  echo "tests/docs must not contain private real-user fixture data:" >&2
  echo "$private_fixture_hits" >&2
  exit 1
fi

feature_doc="docs/feature-test-management.md"
if [ ! -f "$feature_doc" ]; then
  echo "missing feature/test management document: $feature_doc" >&2
  exit 1
fi

feature_doc_required=(
  "Feature and Test Management"
  "Maintenance Rules"
  "Agent Core interaction correctness"
  "UI display correctness"
  "Feature Coverage Matrix"
  "Per-Feature Coverage Floor"
  "Normal path"
  "Boundary path"
  "Error path"
  "Stress/repetition path"
  "Current Supplement Decisions"
  "every new feature"
  "F32"
  "Local Web host and assistant-ui experience"
  "F37"
  "Reliable Web command and event delivery"
  "docs/manual-release-smoke.md"
)

for pattern in "${feature_doc_required[@]}"; do
  if ! search_fixed "$pattern" "$feature_doc"; then
    echo "missing required feature management item: $pattern" >&2
    exit 1
  fi
done

reliability_doc="docs/web_reliability_test_matrix.md"
if [ ! -f "$reliability_doc" ]; then
  echo "missing Web delivery reliability contract: $reliability_doc" >&2
  exit 1
fi

reliability_doc_required=(
  "command_id"
  "core_accepted"
  "event_seq"
  "Ordered delivery and snapshot recovery"
  "Strict exactly-once behavior cannot be promised"
  "four Sessions"
)

for pattern in "${reliability_doc_required[@]}"; do
  if ! search_fixed "$pattern" "$reliability_doc"; then
    echo "missing required Web reliability item: $pattern" >&2
    exit 1
  fi
done

release_management_doc="docs/release-management.md"
if [ ! -f "$release_management_doc" ]; then
  echo "missing release management document: $release_management_doc" >&2
  exit 1
fi

release_management_required=(
  "Never move or overwrite a published tag"
  "scripts/version_consistency_check.sh"
  "Ubuntu and macOS"
  'run `timem`'
  'pull request from the release branch into `main`'
)

for pattern in "${release_management_required[@]}"; do
  if ! search_fixed "$pattern" "$release_management_doc"; then
    echo "missing required release management item: $pattern" >&2
    exit 1
  fi
done

manual_smoke_doc="docs/manual-release-smoke.md"
if [ ! -f "$manual_smoke_doc" ]; then
  echo "missing manual release smoke document: $manual_smoke_doc" >&2
  exit 1
fi

web_ui_matrix_doc="docs/web-ui-feature-test-matrix.md"
if [ ! -f "$web_ui_matrix_doc" ]; then
  echo "missing Web UI feature/test matrix: $web_ui_matrix_doc" >&2
  exit 1
fi

web_ui_matrix_required=(
  "Web UI Feature-Test Matrix"
  "| Authenticated Web host |"
  "| Session creation and naming |"
  "| Per-session runtime profile |"
  "| Multi-session topic isolation |"
  "| Worker hierarchy and state |"
  "| Stop/cancel under human pressure |"
  "Send during active work"
  "| Stale supplement recovery |"
  "| Attachments |"
  "| Inline decisions |"
  "| Work instructions |"
  "| Current cwd display |"
  "| Turn process rendering |"
  "| Final answer rendering |"
  "| Usage and context status |"
  "| History and resume |"
  "| Mem switching |"
  "| Appearance |"
  "| Scroll and bounded rendering |"
  "| Diagnostics and host errors |"
  "| Release packaging |"
)

for pattern in "${web_ui_matrix_required[@]}"; do
  if ! search_fixed "$pattern" "$web_ui_matrix_doc"; then
    echo "missing required Web UI feature/test matrix item: $pattern" >&2
    exit 1
  fi
done

# Behavioral coverage is enforced by executable Rust, Vitest, smoke, and
# real-browser tests. Do not treat test-name strings as proof of coverage.

manual_smoke_required=(
  "Manual Release Smoke"
  "Web Browser Matrix"
  "Safari"
  "Firefox"
  "Terminal Emulator Matrix"
  "Clean-Machine Install"
  "Live Model Service Smoke"
)

for pattern in "${manual_smoke_required[@]}"; do
  if ! search_fixed "$pattern" "$manual_smoke_doc"; then
    echo "missing required manual release smoke item: $pattern" >&2
    exit 1
  fi
done

test_strategy_doc="docs/test-strategy.md"
if [ ! -f "$test_strategy_doc" ]; then
  echo "missing test strategy document: $test_strategy_doc" >&2
  exit 1
fi

test_strategy_required=(
  "Two Quality Axes"
  "Four Coverage Dimensions"
  "Agent Core interaction correctness"
  "UI display correctness"
  "A behavior that crosses both axes needs tests on both sides"
  "Normal path"
  "Boundary path"
  "Error path"
  "Stress / repetition path"
)

for pattern in "${test_strategy_required[@]}"; do
  if ! search_fixed "$pattern" "$test_strategy_doc"; then
    echo "missing required test strategy item: $pattern" >&2
    exit 1
  fi
done

for id in $(seq 1 33); do
  feature_id="$(printf 'F%02d' "$id")"
  if ! search_fixed "| $feature_id |" "$feature_doc"; then
    echo "missing required feature row: $feature_id" >&2
    exit 1
  fi
done

if [ ! -f CHANGELOG.md ]; then
  echo "missing CHANGELOG.md" >&2
  exit 1
fi

changelog_required=(
  "# Changelog"
  "## [Unreleased]"
)

for pattern in "${changelog_required[@]}"; do
  if ! search_fixed "$pattern" CHANGELOG.md; then
    echo "missing required changelog item: $pattern" >&2
    exit 1
  fi
done

if ! search_fixed "scripts/update_static_prompt_snapshot.sh --check" scripts/ci.sh; then
  echo "static prompt expansion generator must remain a CI gate" >&2
  exit 1
fi

if ! search_fixed "scripts/clippy_check.sh" docs/test-strategy.md docs/feature-test-management.md scripts/ci.sh; then
  echo "clippy warning gate must remain documented and wired into CI" >&2
  exit 1
fi

workflow=".github/workflows/ci.yml"
if [ ! -f "$workflow" ]; then
  echo "missing GitHub Actions workflow: $workflow" >&2
  exit 1
fi

workflow_required=(
  "push:"
  "pull_request:"
  "scripts/ci.sh"
  '"1.0"'
  "ubuntu-latest"
  "macos-latest"
  "expect"
  "Upload Timem runtime I/O report"
  "target/runtime-io-guard/report.json"
)

for pattern in "${workflow_required[@]}"; do
  if ! search_fixed "$pattern" "$workflow"; then
    echo "missing required workflow item: $pattern" >&2
    exit 1
  fi
done

echo "test_contract_check: ok"
