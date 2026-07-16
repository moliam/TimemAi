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

required_patterns=(
  "session_turn_forced_shrink_runs_to_final_without_repeated_shrink"
  "session_turn_truncated_output_expands_limit_and_retries_same_turn"
  "session_turn_truncated_output_stop_sets_structured_stop_reason"
  "session_turn_round_limit_continue_recharges_and_finishes_same_task"
  "session_turn_round_limit_stop_sets_structured_stop_reason"
  "session_turn_bash_approval_executes_action_then_finishes_with_audit"
  "session_turn_scratch_context_offload_records_id_and_continues"
  "session_replay_story_covers_repair_memory_scratch_shrink_and_observation_rendering"
  "successful_prompt_shrink_invalidates_stale_observed_prompt_tokens"
  "forced_shrink_is_not_reissued_when_dynamic_context_cannot_reduce_enough"
  "memory_update_concurrent_same_version_conflicts_allow_only_one_winner"
  "mem_guard_keeps_concurrent_memory_updates_from_losing_records"
  "run_bash_can_start_and_poll_background_job"
  "timeout_job_is_reported_running_and_model_can_kill_by_pid"
  "running_job_list_is_injected_when_discard_references_running_job_delta"
  "running_job_list_is_injected_when_offload_references_running_job_delta"
  "running_job_list_is_injected_when_compact_references_running_job_delta"
  "running_job_list_is_not_injected_when_discard_refs_unrelated_delta"
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
  "real_tty_supplement_smoke"
  "raw_multiline_paste_requires_confirmation_before_model_submit"
  "queued_paste_fallback_handles_crlf_boundary_without_extra_blank_line"
  "config_report_is_ui_neutral_and_groups_effective_values"
  "config_menu_report_is_ui_neutral_command_data"
  "workspace_menu_report_is_ui_neutral_command_data"
  "runtime_status_snapshot_groups_retry_state_for_host_rendering"
  "runtime_status_snapshot_keeps_memory_activity_structured_for_host_rendering"
  "turn_ui_decision_requests_are_structured_and_ui_neutral"
  "stale_context_decision_request_is_structured_and_ui_neutral"
  "config_apply_report_is_ui_neutral_command_data"
  "performance_guard_large_context_prompt_render_is_bounded"
  "performance_guard_topic_generation_for_many_actions_is_bounded"
  "performance_guard_many_observation_events_render_bounded"
  "session_turn_preserves_cache_plan_with_xml_response_protocol"
  "restored_web_turns_follow_history_time_not_turn_id_lexical_order"
  "restored_web_turns_preserve_user_entry_kinds"
)

for pattern in "${required_patterns[@]}"; do
  if ! search_regex "$pattern" agent_core timem_shell scripts docs; then
    echo "missing required test/contract pattern: $pattern" >&2
    exit 1
  fi
done

inline_test_hits="$(
  search_lines_regex '#\[test\]' \
    agent_core/src timem_shell/src resources/capabilities/tools \
    || true
)"
if [ -n "$inline_test_hits" ]; then
  echo "test functions must live under a crate tests directory, not src:" >&2
  echo "$inline_test_hits" >&2
  exit 1
fi

ci_required=(
  "cargo test --workspace"
  "pnpm --dir web_ui/timem-web test"
  "pnpm --dir web_ui/timem-web build"
  "cargo build --locked -p timem_shell -p timem_web --release"
  "scripts/edge_regression.sh"
  "scripts/real_tty_smoke.expect"
  "scripts/real_tty_supplement_smoke.expect"
  "scripts/sensitive_scan.sh --current"
  "scripts/update_static_prompt_snapshot.sh --check"
  "scripts/performance_guard.sh"
  "scripts/cross_host_resume_smoke.sh"
  "scripts/web_license_check.sh"
)

for pattern in "${ci_required[@]}"; do
  if ! search_fixed "$pattern" scripts/ci.sh; then
    echo "missing required CI gate: $pattern" >&2
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
  if search_fixed "$pattern" timem_shell/src/lib.rs; then
    echo "timem_shell must not re-expose core runtime layout/context wrapper: $pattern" >&2
    exit 1
  fi
done

shell_lib_forbidden_core_internals=(
  "prepare_provider_request"
  "prepare_provider_http_request"
  "provider_http_error_message"
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
  if search_fixed "$pattern" timem_shell/src/lib.rs; then
    echo "timem_shell must not re-export provider/cache core internals: $pattern" >&2
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
  if search_fixed "$pattern" timem_shell/src; then
    echo "timem_shell must not implement provider/tool execution: $pattern" >&2
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

legacy_action_input_hits="$(
  search_lines_regex 'next_actions.*"input"[[:space:]]*:' \
    agent_core/tests agent_core/src/session_runtime.rs timem_shell/src/observation.rs timem_shell/src/lib.rs \
    | grep -v 'allow_legacy_input_negative_test' || true
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
  search_lines_regex '"args"[[:space:]]*:[[:space:]]*"' \
    agent_core/tests agent_core/src/session_runtime.rs timem_shell/src/observation.rs timem_shell/src/lib.rs resources docs README.md CHANGELOG.md scripts \
    | grep -v 'allow_string_args_negative_test' \
    | grep -v 'response_schema_summary.json' \
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
  search_lines_regex '默默|李默|儿子|son birthday|6月12|蓝色雨伞|绿色雨衣|fangchang|/Users/limo3|/Users/fangchang|v0\.6 发布检查|AURORA' \
    agent_core/tests timem_shell/src resources docs README.md CHANGELOG.md scripts \
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

for id in $(seq 1 28); do
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
)

for pattern in "${workflow_required[@]}"; do
  if ! search_fixed "$pattern" "$workflow"; then
    echo "missing required workflow item: $pattern" >&2
    exit 1
  fi
done

echo "test_contract_check: ok"
