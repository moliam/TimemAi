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

required_patterns=(
  "session_turn_forced_shrink_runs_to_final_without_repeated_shrink"
  "session_turn_truncated_output_expands_limit_and_retries_same_turn"
  "session_turn_round_limit_continue_recharges_and_finishes_same_task"
  "session_turn_bash_approval_executes_action_then_finishes_with_audit"
  "session_turn_scratch_context_offload_records_id_and_continues"
  "session_replay_story_covers_repair_memory_scratch_shrink_and_observation_rendering"
  "successful_prompt_shrink_invalidates_stale_observed_prompt_tokens"
  "forced_shrink_is_not_reissued_when_dynamic_context_cannot_reduce_enough"
  "memory_update_concurrent_same_version_conflicts_allow_only_one_winner"
  "mem_guard_keeps_concurrent_memory_updates_from_losing_records"
  "run_bash_can_start_and_poll_background_job"
  "shell_job_status_waits_for_model_chosen_timeout_before_running_result"
  "ci_realistic_multiturn_memory_tools_security_and_shrink_story"
  "run_multiline_paste_cancel_smoke"
  "run_edited_paste_recovery_ctrl_c_smoke"
  "run_edited_paste_recovery_esc_smoke"
  "run_edited_paste_recovery_return_to_edit_smoke"
  "run_shift_enter_cancel_smoke"
  "run_wrapped_edit_cancel_smoke"
  "run_config_value_cancel_smoke"
  "run_config_provider_switch_smoke"
  "run_workspace_add_cancel_smoke"
  "raw_multiline_paste_requires_confirmation_before_model_submit"
  "queued_paste_fallback_handles_crlf_boundary_without_extra_blank_line"
)

for pattern in "${required_patterns[@]}"; do
  if ! search_regex "$pattern" agent_core timem_shell scripts docs; then
    echo "missing required test/contract pattern: $pattern" >&2
    exit 1
  fi
done

ci_required=(
  "cargo test --workspace"
  "scripts/edge_regression.sh"
  "scripts/real_tty_smoke.expect"
  "scripts/sensitive_scan.sh --current"
  "scripts/update_static_prompt_snapshot.sh --check"
)

for pattern in "${ci_required[@]}"; do
  if ! search_fixed "$pattern" scripts/ci.sh; then
    echo "missing required CI gate: $pattern" >&2
    exit 1
  fi
done

for file in resources/capabilities/tools/*.yaml; do
  if ! awk '/^example_json: \|/{in_example=1; next} /^kind: /{in_example=0} in_example && /"args"[[:space:]]*:/{found=1} END{exit found ? 0 : 1}' "$file"; then
    echo "tool manifest example_json must include args: $file" >&2
    exit 1
  fi
  if awk '/^example_json: \|/{in_example=1; next} /^kind: /{in_example=0} in_example && /"input"[[:space:]]*:/{found=1} END{exit found ? 0 : 1}' "$file"; then
    echo "tool manifest example_json must use args, not input: $file" >&2
    exit 1
  fi
done

legacy_action_input_hits="$(
  rg -n 'next_actions.*"input"[[:space:]]*:' \
    agent_core/tests timem_shell/src/session_runtime.rs timem_shell/src/observation.rs timem_shell/src/lib.rs \
    | rg -v 'allow_legacy_input_negative_test' || true
)"
if [ -n "$legacy_action_input_hits" ]; then
  echo "mock model outputs must use args, not legacy input:" >&2
  echo "$legacy_action_input_hits" >&2
  exit 1
fi
if ! search_fixed "allow_legacy_input_negative_test" agent_core/tests/core_tests.rs; then
  echo "missing explicit negative test marker for legacy input rejection" >&2
  exit 1
fi
string_args_hits="$(
  rg -n '"args"[[:space:]]*:[[:space:]]*"' \
    agent_core/tests timem_shell/src/session_runtime.rs timem_shell/src/observation.rs timem_shell/src/lib.rs resources docs README.md CHANGELOG.md scripts \
    | rg -v 'allow_string_args_negative_test' \
    | rg -v 'response_v1_summary.json' \
    | rg -v 'docs/static-prompt-expanded.md' \
    || true
)"
if [ -n "$string_args_hits" ]; then
  echo "mock model outputs and docs must use object args, not string args:" >&2
  echo "$string_args_hits" >&2
  exit 1
fi
if ! search_fixed "allow_string_args_negative_test" agent_core/tests/core_tests.rs; then
  echo "missing explicit negative test marker for string args rejection" >&2
  exit 1
fi

private_fixture_hits="$(
  rg -n '默默|李默|儿子|son birthday|6月12|蓝色雨伞|绿色雨衣|fangchang|/Users/limo3|/Users/fangchang|v0\.6 发布检查|AURORA' \
    agent_core/tests timem_shell/src resources docs README.md CHANGELOG.md scripts \
    | rg -v 'scripts/test_contract_check.sh' \
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
)

for pattern in "${feature_doc_required[@]}"; do
  if ! search_fixed "$pattern" "$feature_doc"; then
    echo "missing required feature management item: $pattern" >&2
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

for id in $(seq 1 27); do
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

static_prompt_snapshot="docs/static-prompt-expanded.md"
if [ ! -f "$static_prompt_snapshot" ]; then
  echo "missing expanded static prompt snapshot: $static_prompt_snapshot" >&2
  exit 1
fi

static_prompt_snapshot_required=(
  "Expanded Static Prompt Snapshot"
  "read-only audit snapshot"
  "not read by Timem at runtime"
  "[BEGIN SEGMENT 0: prompt_0]"
  "#### \`run_bash\`"
  "#### \`memmgr\`"
  "**Usage**"
  "**Result**"
  '"args": {'
  '"command": "'
  '"fields"'
  '"status?"'
)

for pattern in "${static_prompt_snapshot_required[@]}"; do
  if ! search_fixed "$pattern" "$static_prompt_snapshot"; then
    echo "missing required static prompt snapshot item: $pattern" >&2
    exit 1
  fi
done

static_prompt_snapshot_forbidden=(
  "\"output\": {"
  "Background job id when background=true."
  "\"output_file\""
  "\"status_file\""
  "\"approval_status\""
  "\"static_prefix_policy\""
  "static prefix is immutable global guidance"
  "\"ui_status\""
  "ui_label"
  "ui_visible"
  "\"tool_policy\""
  "\"sql_tables\""
  "\"bash_safety\""
  '"$id"'
)

for pattern in "${static_prompt_snapshot_forbidden[@]}"; do
  if search_fixed "$pattern" "$static_prompt_snapshot"; then
    echo "forbidden verbose schema dump in static prompt snapshot: $pattern" >&2
    exit 1
  fi
done

workflow=".github/workflows/ci.yml"
if [ ! -f "$workflow" ]; then
  echo "missing GitHub Actions workflow: $workflow" >&2
  exit 1
fi

workflow_required=(
  "push:"
  "pull_request:"
  "scripts/ci.sh"
  "ubuntu-latest"
  "macos-latest"
  "expect"
)

for pattern in "${workflow_required[@]}"; do
  if ! search_fixed "$pattern" "$workflow"; then
    echo "missing required workflow item: $pattern" >&2
    exit 1
  fi
done

echo "test_contract_check: ok"
