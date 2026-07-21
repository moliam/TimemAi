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
  "turn_user_entries_are_persisted_with_raw_text_and_semantic_kind"
  "sorts restored entries and events within one turn by creation time"
  "shell_resume_uses_stored_session_cwd_for_core_prompt_context"
  "shell_resume_applies_stored_session_env_but_keeps_cli_override_precedence"
  "shell_can_resume_web_style_session_history"
  "formatted_response_trailer_parser_preserves_assistant_heading"
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
  "python3 scripts/web_ui_matrix_check.py"
  "scripts/update_static_prompt_snapshot.sh --check"
  "scripts/clippy_check.sh"
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
  "docs/manual-release-smoke.md"
)

for pattern in "${feature_doc_required[@]}"; do
  if ! search_fixed "$pattern" "$feature_doc"; then
    echo "missing required feature management item: $pattern" >&2
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

web_ui_required_test_names=(
  "rapid_submit_during_an_active_turn_is_treated_as_a_supplement"
  "repeated_user_sends_during_an_active_turn_are_ordered_supplements"
  "active_turn_supplement_consumes_pending_attachments_into_the_same_turn"
  "failed_active_turn_supplement_does_not_drop_pending_attachments"
  "stale_supplement_after_cancel_completion_starts_a_new_turn"
  "stale_supplement_after_cancel_consumes_pending_attachments_as_a_new_task"
  "duplicate_cancel_commands_are_idempotent_for_one_active_turn"
  "uses synchronous pending guards for rapid repeated browser clicks"
  "guards one browser draft submission while preserving text typed during the pending send"
  "keeps drafts and pending send guards isolated by session"
  "prunes stale drafts and pending send locks when a snapshot swaps out sessions"
  "recovers from an in-flight old-mem send after a mem snapshot swaps sessions"
  "moves the active session to a live session when a snapshot swaps out the old one"
  "moves active selection to a live session when a reconnect or mem snapshot swaps sessions"
  "uses session terminology consistently for the creation workflow"
  "supports agent rename and a distinct animated working state"
  "expands each session into its scoped worker status list"
  "accepts lifecycle topics that introduce a new scoped worker and context"
  "binds assistant-ui running state to the authoritative session lifecycle"
  "creates sessions with independent runtime environment overrides"
  "does not send new tasks or supplements while a mem switch is pending"
  "does not rename a session while mem switching or another rename is pending"
  "locks old-session interactions while a mem switch snapshot is pending"
  "does not send while cancellation is still in flight"
  "keeps draft text and releases the pending guard when cancellation blocks a reserved send"
  "sends a new task after a cancelled active turn is marked finished"
  "keeps rapid repeated sends during a working turn as separate supplements"
  "keeps a human click storm bounded and session scoped"
  "lets users remove pending attachments without losing access to long file names"
  "keeps working-turn input visually consistent with a normal send"
  "restores task, supplement, and approval user entries inside one turn"
  "moves submitted files from the composer into a compact user attachment list"
  "queues concurrent decisions by session and request id without cross-session replacement"
  "clears only the resumed workers decision within a shared session"
  "uses an explicit session-created event and session-scoped inline decisions"
  "does not turn work-instruction bookkeeping into user-visible activity"
  "pairs duplicate concurrent actions in order without collapsing either invocation"
  "pairs action lifecycle events even when input object key order changes"
  "pairs action lifecycle events when nested input object key order changes"
  "applies a model response only to the session named by the core topic"
  "applies a structured cwd update only to the matching session"
  "rejects core topics scoped to an unknown context before mutating a session"
  "keeps a matched agent working without changing unrelated sessions"
  "renders context compaction outside chat messages with a reduced-motion fallback"
  "keeps context compaction as a typed system activity with token metrics"
  "uses the Markdown highlighter for final answers and Bash activity commands"
  "renders GFM and highlighted code with a copy affordance"
  "groups each task into user input, bounded process, and separate final delivery"
  "uses frame styling without repeating user or session identity labels"
  "coalesces tool lifecycles and renders tools as compact subordinate rows"
  "replaces an action start with its terminal lifecycle event"
  "shows the live session cwd in navigation and above the composer"
  "uses only the selected session's latest real provider usage for context"
  "renders live task usage and session context without replacing final telemetry"
  "attaches completion telemetry only to the matching final answer"
  "persists theme, font, and text-size appearance without changing core state"
  "removes the access token from the visible URL while retaining the session credential"
  "public_web_launch_keeps_token_auth_and_reports_bind_mode"
  "static_web_entry_requires_token_or_authenticated_cookie"
  "shows the runtime bind host and public-token mode from the server snapshot"
  "shows host and session errors outside the default-hidden diagnostic panel"
  "defaults the diagnostic activity panel to hidden"
  "bounds a reconnect snapshot with many turns and high-frequency events"
  "bounds newly appended turns without changing chronological order"
  "keeps repeated live event bursts bounded and isolated across sessions"
)

for pattern in "${web_ui_required_test_names[@]}"; do
  if ! search_fixed "$pattern" timem_web/tests web_ui/timem-web/tests; then
    echo "missing required Web UI regression test implementation: $pattern" >&2
    exit 1
  fi
done

manual_smoke_required=(
  "Manual Release Smoke"
  "Web Browser Matrix"
  "Safari"
  "Firefox"
  "Terminal Emulator Matrix"
  "Clean-Machine Install"
  "Live Provider Smoke"
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
)

for pattern in "${workflow_required[@]}"; do
  if ! search_fixed "$pattern" "$workflow"; then
    echo "missing required workflow item: $pattern" >&2
    exit 1
  fi
done

echo "test_contract_check: ok"
