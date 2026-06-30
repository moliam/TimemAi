#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

required_patterns=(
  "session_turn_forced_shrink_runs_to_final_without_repeated_shrink"
  "session_turn_truncated_output_expands_limit_and_retries_same_turn"
  "session_turn_round_limit_continue_recharges_and_finishes_same_task"
  "session_turn_bash_approval_executes_action_then_finishes_with_audit"
  "session_turn_scratch_context_offload_records_id_and_continues"
  "successful_prompt_shrink_invalidates_stale_observed_prompt_tokens"
  "forced_shrink_is_not_reissued_when_dynamic_context_cannot_reduce_enough"
  "memory_update_concurrent_same_version_conflicts_allow_only_one_winner"
  "mem_guard_keeps_concurrent_memory_updates_from_losing_records"
  "run_bash_can_start_and_poll_background_job"
  "shell_job_status_waits_for_model_chosen_timeout_before_running_result"
  "ci_realistic_multiturn_memory_tools_security_and_shrink_story"
  "run_multiline_paste_cancel_smoke"
  "run_shift_enter_cancel_smoke"
  "run_wrapped_edit_cancel_smoke"
  "run_config_value_cancel_smoke"
  "run_workspace_add_cancel_smoke"
  "raw_multiline_paste_requires_confirmation_before_model_submit"
  "queued_paste_fallback_handles_crlf_boundary_without_extra_blank_line"
)

for pattern in "${required_patterns[@]}"; do
  if ! rg -q -- "$pattern" agent_core timem_shell scripts docs; then
    echo "missing required test/contract pattern: $pattern" >&2
    exit 1
  fi
done

ci_required=(
  "cargo test --workspace"
  "scripts/edge_regression.sh"
  "scripts/real_tty_smoke.expect"
  "scripts/sensitive_scan.sh --current"
)

for pattern in "${ci_required[@]}"; do
  if ! rg -q -F -- "$pattern" scripts/ci.sh; then
    echo "missing required CI gate: $pattern" >&2
    exit 1
  fi
done

feature_doc="docs/feature-test-management.md"
if [ ! -f "$feature_doc" ]; then
  echo "missing feature/test management document: $feature_doc" >&2
  exit 1
fi

feature_doc_required=(
  "Feature and Test Management"
  "Maintenance Rules"
  "Feature Coverage Matrix"
  "Current Supplement Decisions"
  "every new feature"
)

for pattern in "${feature_doc_required[@]}"; do
  if ! rg -q -F -- "$pattern" "$feature_doc"; then
    echo "missing required feature management item: $pattern" >&2
    exit 1
  fi
done

for id in $(seq 1 25); do
  feature_id="$(printf 'F%02d' "$id")"
  if ! rg -q -F -- "| $feature_id |" "$feature_doc"; then
    echo "missing required feature row: $feature_id" >&2
    exit 1
  fi
done

echo "test_contract_check: ok"
