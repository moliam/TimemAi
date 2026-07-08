#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

ITERATIONS="${TIMEM_EDGE_ITERATIONS:-2}"

case "$ITERATIONS" in
  ''|*[!0-9]*)
    echo "error: TIMEM_EDGE_ITERATIONS must be a positive integer" >&2
    exit 2
    ;;
esac

if [ "$ITERATIONS" -lt 1 ]; then
  echo "error: TIMEM_EDGE_ITERATIONS must be >= 1" >&2
  exit 2
fi

for i in $(seq 1 "$ITERATIONS"); do
  echo "== edge regression iteration $i/$ITERATIONS: session runtime =="
  cargo test -p timem_shell session_turn_ -- --nocapture

  echo "== edge regression iteration $i/$ITERATIONS: shrink core =="
  cargo test -p agent_core successful_prompt_shrink_invalidates_stale_observed_prompt_tokens -- --nocapture
  cargo test -p agent_core forced_shrink_is_not_reissued_when_dynamic_context_cannot_reduce_enough -- --nocapture

  echo "== edge regression iteration $i/$ITERATIONS: memory concurrency =="
  cargo test -p agent_core memory_update_concurrent_same_version_conflicts_allow_only_one_winner -- --nocapture
  cargo test -p agent_core mem_guard_keeps_concurrent_memory_updates_from_losing_records -- --nocapture

  echo "== edge regression iteration $i/$ITERATIONS: shell jobs =="
  cargo test -p agent_core background_job_reports_pid_and_running_list_until_exit -- --nocapture
  cargo test -p agent_core run_bash_background_job_enters_running_list_and_later_emits_exit_update -- --nocapture
  cargo test -p agent_core running_job_list_is_injected_when_discard_references_running_job_delta -- --nocapture
  cargo test -p agent_core timeout_job_is_reported_running_and_model_can_kill_by_pid -- --nocapture

  echo "== edge regression iteration $i/$ITERATIONS: realistic story =="
  cargo test -p agent_core ci_realistic_multiturn_memory_tools_security_and_shrink_story -- --nocapture
done

echo "edge_regression: ok"
