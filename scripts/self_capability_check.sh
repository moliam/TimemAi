#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

run_exact() {
  local package="$1"
  local target_kind="$2"
  local target_name="$3"
  local test_name="$4"
  local -a target_args=()
  case "$target_kind" in
    lib) target_args=(--lib) ;;
    test) target_args=(--test "$target_name") ;;
    *) echo "error: unsupported target kind: $target_kind" >&2; exit 2 ;;
  esac

  local listed count
  listed="$(cargo test -p "$package" "${target_args[@]}" --locked -- --list)"
  count="$(printf '%s\n' "$listed" | sed -n 's/: test$//p' | grep -Fxc "$test_name" || true)"
  if [[ "$count" -ne 1 ]]; then
    echo "error: required self-capability test must exist exactly once: $package $test_name (found $count)" >&2
    exit 1
  fi
  cargo test -p "$package" "${target_args[@]}" --locked "$test_name" -- --exact --test-threads=1
}

run_dimension() {
  echo "== Timem self-capability: $1 =="
}

run_dimension "who / authoritative identity and UI projection"
run_exact timem_session lib "" tests::session_worker_prompt_includes_identity_runtime_surface_and_command_target
run_exact timem_shell lib "" tests::shell_renders_worker_identity_from_lifecycle_topic
run_exact timem lib "" server::tests::real_concurrent_workers_route_final_topics_to_matching_web_sessions

run_dimension "where / platform, cwd, and matching local tools"
run_exact agent_core lib "" capability::tests::platform_profiles_select_only_the_matching_local_command_tool
run_exact agent_core lib "" capability::tests::builtin_run_bash_description_includes_dynamic_os_and_bash_versions
run_exact timem lib "" server::tests::successful_cwd_action_updates_only_its_session_and_reconnect_snapshot

run_dimension "what / restart recovery and direct continuation"
run_exact agent_core lib "" prompt_component_tests::direct_resume_prompt_follows_the_interruption_note_in_component_order
run_exact timem lib "" server::tests::direct_resume_host_constructs_hidden_turn_and_shared_model_input
run_exact timem lib "" server::tests::runtime_restart_requires_each_restored_session_to_resolve_cwd_before_work

run_dimension "how / prompt catalog and executable capability parity"
run_exact agent_core lib "" capability::tests::tool_catalog_is_injected_for_the_active_protocol
run_exact agent_core lib "" tool_registry::tests::builtin_registry_lists_all_compiled_tool_callbacks
run_exact agent_core lib "" capability::tests::host_profile_without_bash_keeps_native_commands_but_filters_run_bash

run_dimension "long work / bounded context and resumable compaction"
run_exact agent_core test core_tests long_context_forces_shrink_at_ninety_percent_window_with_compaction_instruction
run_exact agent_core lib "" session_runtime::tests::session_turn_scratch_context_offload_records_id_and_continues
run_exact agent_core lib "" prompt_component_tests::native_context_compact_persists_summary_after_discarding_all_old_deltas

run_dimension "runtime completeness / terminal ordering and late input ownership"
run_exact agent_core lib "" session_runtime::tests::cancelled_turn_projection_has_one_token_and_authoritative_terminal_order
run_exact agent_core lib "" shell_exec::tests::concurrent_refresh_delivers_each_terminal_update_exactly_once
run_exact timem_session lib "" tests::worker_option_returns_late_supplement_after_preserving_the_first_final_answer
run_exact timem lib "" server::tests::final_answer_is_preserved_before_unconsumed_supplement_starts_a_new_turn

echo "self_capability_check: ok"
