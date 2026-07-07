# Timem Shell Test Strategy

This project uses layered tests. A feature is not considered protected when it
only has a helper-function assertion; state-machine features need an end-to-end
path and repeated edge regression coverage.

The authoritative feature-to-test ledger is
`docs/feature-test-management.md`. When adding or changing a feature, update
that ledger in the same change so the feature, test suites, covered boundaries,
and remaining supplement decisions stay visible.

## Two Quality Axes

Every feature test must be reviewed against two product-facing axes:

1. Agent Core interaction correctness

   The runtime/model loop must advance correctly. These tests prove protocol
   parsing, model repair, action execution, memory/scratch/chat behavior,
   prompt shrink, cache planning, provider errors, audit, cancellation, and
   multi-round state transitions. Prefer `agent_core` unit/integration tests
   and `agent_core::session_runtime` fake-model tests for this axis.

2. UI display correctness

   The shell UI must accurately and clearly represent what the runtime is doing.
   These tests prove observation rendering, status/token lines, config/banner
   layout, input editing, paste recovery, menus, elapsed time, cancellation
   prompts, and that model free_talk/progress/action semantics are rendered in
   the intended UI surfaces without leaking raw protocol names. Prefer render
   contract tests plus real pseudo-TTY smoke for this axis.

A behavior that crosses both axes needs tests on both sides. For example,
model-output parsing must prove that Agent Core can execute the action and that
the UI can render the same model output as the correct intent/action display.

## Four Coverage Dimensions

Each release-ready feature should be protected by roughly four independent
checks. If a dimension is not applicable, record that residual decision in
`docs/feature-test-management.md`.

1. Normal path: the expected user flow works end to end.
2. Boundary path: limits, empty values, long values, wrapping, id ranges,
   thresholds, or narrow terminal widths behave correctly.
3. Error path: malformed model output, provider errors, cancellation, permission
   denial, missing fields, or invalid input fails safely.
4. Stress / repetition path: multi-turn sessions, repeated edge regression,
   concurrent state, pseudo-TTY smoke/stress, per-session worker paths, or
   race-prone paths stay stable.

## Required Layers

- Function tests: pure parsing, formatting, prompt cache planning, provider
  payload shaping, token/status rendering, path normalization, and redaction.
- Unit tests: `agent_core` actions and storage behavior with real temp files.
- Integration tests: complete `agent_core::session_runtime` turns with a fake model client,
  real `AgentCore`, real action execution, real audit writes, and UI decisions.
- Replay story tests: scripted multi-turn user/model conversations that exercise
  normal replies, malformed model recovery, memory retrieve, scratch offload,
  context discard, audit writes, and observation rendering in one end-to-end
  path.
- Real TTY smoke: compiled release binary driven through a pseudo terminal for
  input/editor/menu behavior.
- Real TTY stress: compiled release binary driven through a pseudo terminal
  while a fake provider causes repeated model/action redraws, long
  Thought/Action rows, and mid-turn user supplements.
- Performance guard: `scripts/performance_guard.sh` runs bounded hot-path
  checks for large prompt rendering, topic fan-out, and observation panel
  rendering with long rows. Thresholds are intentionally broad enough for CI
  stability, but tight enough to catch accidental full static-prompt
  re-expansion, quadratic row trimming, or topic fan-out regressions.
- Repeated edge regression: high-risk state machines run multiple times in CI
  through `scripts/edge_regression.sh`.

## Feature Coverage Matrix

| Feature area | Function / unit coverage | Integration / E2E coverage | Repeated edge coverage |
|---|---|---|---|
| Provider config, protocol, URL, output/input limits | `provider_config_from_env`, `parse_cli_args`, provider switch default-reset tests, protocol adapter tests | startup banner and `/config` real TTY smoke including provider switch/default URL validation | full CI |
| Provider response parsing and errors | OpenAI-compatible, OpenAI Responses, Anthropic usage/error tests | truncated output expansion session test; transient provider error retry session test; protocol repair session test with audit assertions | edge regression session group |
| Prompt cache planning | `prompt_cache_strategy_*`, prefix-cache simulator tests with bounded lookback, provider request cache-control tests, Anthropic cache read/create usage tests, `scripts/kvc_replay_test.sh`, `scripts/kvc_replay.py` local audit replay | `session_turn_preserves_incremental_prompt_cache_plan_across_rounds`, `session_turn_preserves_cache_plan_with_json_response_protocol`, `session_turn_preserves_cache_plan_with_markdown_response_protocol`, `session_turn_preserves_cache_plan_with_xml_response_protocol`, request audit redaction/hash tests | full CI runs JSON/Markdown/XML replay fixture coverage; run local audit replay before cache-strategy releases |
| Prompt delta/slice rendering | prompt segmentation, multi-slice core tests, focused response-repair slice tests | shrink session E2E | edge regression shrink group |
| Forced shrink | core shrink threshold, stale observed-token invalidation, static-dominant guard | `session_turn_forced_shrink_runs_to_final_without_repeated_shrink` | edge regression shrink + session groups |
| Scratch notes and context offload | scratch write/read/query/delete, invalid refs, missing fields | `session_turn_scratch_context_offload_records_id_and_continues` | session group |
| Durable memory | query/update/delete, expected version, SQL read surface | realistic multi-turn memory story | memory concurrency + realistic story groups |
| Multi-CLI memory conflicts | mem guard cross-process and same-version conflict tests | realistic story exercises shared storage shape | memory concurrency group |
| Chat history | persisted query, delete, SQL time-window, current prompt fallback | realistic story | full CI |
| Bash actions | approval risk, normal shell, background jobs, documented `ask/approve` parsing | bash approval session E2E | shell job group |
| Runtime self tool | `self_tool::tests::*`, manifest/registry/executor tests, sensitive/protected env denial tests | core action replay for env/path/about/process plus UI observation tests | full CI |
| User scenario replay | focused core replay tests for coding, memory QA, self QA/env update, and file-writing output | `scenario_coding_inspects_project_and_reports_from_shell_evidence`, `scenario_memory_qa_retrieves_durable_and_raw_chat_before_answering`, `scenario_self_qa_and_runtime_env_update_stays_bounded`, `scenario_file_writing_outputs_artifact_and_verifies_content` | full CI |
| Background jobs | `run_bash` pid start, timeout-to-running, exit update, shrink-time running list, and registered command-tool job tests | realistic story where applicable | shell/tool job groups |
| Multi-turn replay story | protocol parsing, memory/scratch/shrink primitives | `session_replay_story_covers_repair_memory_scratch_shrink_and_observation_rendering` | full CI |
| Session worker lifecycle | lifecycle topic/accessor, worker channel tests | `session_worker_emits_lifecycle_runs_turn_and_accepts_mid_turn_supplement`, `session_worker_rename_emits_updated_identity_topic`, `session_worker_shutdown_cancels_pending_host_decision`, `core_lifecycle_topic_round_trips_worker_identity_workspace_and_context` | full CI |
| Round limit continuation | core continuation tests | `session_turn_round_limit_continue_recharges_and_finishes_same_task` | session group |
| Cancellation | cancel before provider call, command cancellation tests | real TTY Ctrl+C smoke | real TTY smoke |
| Interactive input | CJK width, paste placeholder, Shift+Enter, control stripping, true multiline submitted-line redraw row counts, thinking-time user supplement capture | real TTY multiline/paste/config/workspace smokes plus local fake-provider supplement smoke and stress smoke | real TTY smoke/stress in CI |
| Observation panel | observation event/rendering tests | thinking view tests including retry, repair-count status, model-response topics, and global working-worker count | full CI |
| Profiling | profiler aggregation and storage tests | `session_turn_records_cached_tokens_in_profiler_and_latest_usage`, `/prof` real TTY smoke | real TTY smoke |
| Runtime performance | `performance_guard_large_context_prompt_render_is_bounded`, `performance_guard_topic_generation_for_many_actions_is_bounded`, `performance_guard_many_observation_events_render_bounded` | real TTY stress covers redraw under fake-provider delay and mid-turn supplement | `scripts/performance_guard.sh` in full CI |
| Audit and secrets | append audit, action grouping, redaction tests, sensitive scan | session tests assert turn/action/retry/repair audit records | sensitive scan + full CI |
| Install/update scripts | install logic tests, install run-hint contract | CI script syntax and install logic | full CI |

## CI Gates

`scripts/ci.sh` must run:

1. shell script syntax checks
2. module boundary check via `scripts/module_boundary_check.sh`
3. install logic tests
4. sensitive scan over tracked files
5. `cargo fmt --check`
6. `cargo test --workspace`
7. performance guard via `scripts/performance_guard.sh`
8. repeated edge regression via `scripts/edge_regression.sh`
9. release build
10. real TTY smoke through `expect`
11. whitespace check

`scripts/edge_regression.sh` defaults to two iterations. Increase pressure with:

```bash
TIMEM_EDGE_ITERATIONS=5 scripts/edge_regression.sh
```

When adding a new feature, add it to the matrix and include at least one test in
the lowest practical layer plus an end-to-end or repeated edge test when the
feature crosses runtime state, model actions, UI, storage, shell, or provider
boundaries.
