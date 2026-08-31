# Timem Test Strategy

This project uses layered tests. A feature is not considered protected when it
only has a helper-function assertion; state-machine features need an end-to-end
path and repeated edge regression coverage.

The authoritative feature-to-test ledger is
`docs/feature-test-management.md`. When adding or changing a feature, update
that ledger in the same change so the feature, test suites, covered boundaries,
and remaining supplement decisions stay visible.
Host-specific release checks that cannot run in default CI are listed in
`docs/manual-release-smoke.md`.

## Two Quality Axes

Every feature test must be reviewed against these product-facing axes:

1. Agent Core interaction correctness

   The runtime/model loop must advance correctly. These tests prove protocol
   parsing, model repair, action execution, memory/scratch/chat behavior,
   prompt shrink, cache planning, model service errors, audit, cancellation, and
   multi-round state transitions. Prefer `agent_core` unit/integration tests
   and `agent_core::session_runtime` fake-model tests for this axis.

2. UI display correctness

   Shell and Web must accurately and clearly represent what the runtime is
   doing. Shell tests prove observation rendering, status/token lines,
   config/banner layout, input editing, paste recovery, menus, elapsed time,
   cancellation prompts, and model free_talk/action semantics. Web tests prove
   session isolation, scoped topics, active-turn supplements, inline decisions,
   cancellation pressure, attachments, history paging, per-session profiles,
   cwd changes, context compaction display, final-answer telemetry, and bounded
   rendering. Prefer shell render contracts plus real pseudo-TTY smoke/stress,
   and Web host tests, frontend reducer/render tests, production builds, and
   release browser smoke.

A behavior that crosses both axes needs tests on both sides. For example,
model-output parsing must prove that Agent Core can execute the action, Shell
can render the structured topic, and Web can render the same structured topic
without leaking it into another Session.

## Four Coverage Dimensions

Each release-ready feature should be protected by roughly four independent
checks. If a dimension is not applicable, record that residual decision in
`docs/feature-test-management.md`.

1. Normal path: the expected user flow works end to end.
2. Boundary path: limits, empty values, long values, wrapping, id ranges,
   thresholds, or narrow terminal widths behave correctly.
3. Error path: malformed model output, model service errors, cancellation, permission
   denial, missing fields, or invalid input fails safely.
4. Stress / repetition path: multi-turn sessions, repeated edge regression,
   concurrent state, pseudo-TTY smoke/stress, per-session worker paths, or
   race-prone paths stay stable.

## Required Layers

- Function tests: pure parsing, formatting, prompt cache planning, model service
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
- Web runtime smoke: compiled release Web binary exercised through authenticated
  HTTP, cookie reopen, API, and WebSocket paths in both default loopback and
  explicit public mode, including clean shutdown, same-port restart, and token
  rotation. Real cross-machine browser evidence supplements but never replaces
  this CI gate.
- Real TTY stress: compiled release binary driven through a pseudo terminal
  while a fake model service causes repeated model/action redraws, long
  Thought/Action rows, and queued next questions during active work.
- Web host integration: real `CoreSessionWorker` instances publish concurrent
  topics through the Web runtime in `applications/timem` (Cargo package
  `timem`), proving session isolation, request correlation,
  completion telemetry, work-instruction decisions, bounded host state, and
  independent per-session runtime profiles. Profile tests use two real workers,
  verify lifecycle model service/protocol/context values, ensure global
  defaults are not mutated, and assert that API keys never enter snapshots or
  topics.
  Same-Session tests also create separate Context/Worker identities, verify the
  child inherits the owning Session profile/environment, reject scope mismatch,
  and prove a child finishing cannot mark a still-running primary worker ready.
- Web frontend: Vitest protects session-aware reducers and behavior-level rendering decisions;
  Vite production build is regenerated in CI and must match the tracked bundle.
  Linux CI runs real Chrome against that built bundle, and release review also includes broader browser smoke for scrolling, composer
  docking, session creation/rename, persistent theme/font/text-size choices,
  GFM tables/task lists, syntax-highlighted copyable code blocks, responsive
  overflow, working-turn input, and concurrent activity. Turn-flow
  coverage verifies that task/supplement/approval input stays in one user frame,
  process events remain in a bounded scrollable frame, stable event ids prevent
  replay duplicates, and final Markdown plus token/time telemetry appears below
  the process frame without a reload. Long-history coverage feeds snapshots
  beyond 200 turns and 500 events per turn, bursts 1,500 events through five
  independent sessions, progressively mounts 24 tasks at a time, verifies
  prepend scroll anchoring, and proves new user tasks remain visible after the
  DOM window begins rotating. Multi-round fixture coverage also proves
  live task totals, latest-call context usage, lifecycle-provided context limits,
  cross-session token isolation, and Session-creation profile overrides. A live
  Aliyun browser smoke submits turns to two Sessions with different models and
  verifies that both complete in their own conversation.
- Web delivery reliability: deterministic Host and frontend tests cover one-shot
  socket delivery, correlated accepted/committed/rejected control responses,
  no browser persistence/replay, reconnect snapshot recovery, hard-bounded
  process-local command deduplication and command/event queues, absence of
  per-command state files, strictly ordered in-memory semantic delivery,
  snapshot recovery after gaps or lag, same-Session FIFO, global mutation
  exclusion, and memory epoch barriers. Real-Chrome acceptance starts from a
  non-zero Host cursor, counts WebSocket connections, sends bursty
  Thought/Action-style progress, and fails on any unjustified reconnect or
  runtime-error/reconnect notice. Valid UI activity and rerenders must retain a
  single connection while the Host and protocol remain healthy. The normative
  case list is `docs/web_reliability_test_matrix.md`.
- Turn concurrency stress: a focused stress entry runs the four real concurrent
  Turn scenarios with seeded replay, resource convergence checks, and latency
  percentiles. It uses hundreds/thousands of iterations per scenario rather than
  the two-iteration generic edge loop.
- Performance guard: `scripts/performance_guard.sh` first verifies the exact expected Rust
  performance-test inventory so a stale filter cannot pass after discovering zero tests, then runs
  bounded hot-path checks for large prompt rendering, topic fan-out, observation panel
  rendering with long rows, Web action-lifecycle coalescing, browser event
  burst draining, and the joint long-scroll/warm-Session-cache invalidation
  contract. Thresholds are intentionally broad enough for CI
  stability, but tight enough to catch accidental full static-prompt
  re-expansion, quadratic row trimming, or topic fan-out regressions.
- Repeated edge regression: high-risk state machines run multiple times in CI
  through `scripts/edge_regression.sh`.
- Runtime I/O guard: `scripts/runtime_io_guard.py` instruments the release
  Shell's existing real-TTY stress story after startup. It observes only the
  Timem process tree during a two-second idle interval and a real model/action
  turn, and fails when average physical reads plus writes exceed 500,000 B/s.
  Linux reads `/proc/<pid>/io`; macOS uses `proc_pid_rusage`. The JSON report is
  uploaded by CI. Compilation, the test harness, and the fake model server are
  deliberately outside the measurement.
- Storage maintenance trigger guard: ordinary history/audit work does not scan
  the MEM. Segmented stores enforce capacity through small manifests at write
  boundaries. Full temporary-data reconciliation becomes due after six hours
  of cumulative Timem runtime across restarts; stopped time does not count. A
  tiny per-MEM counter is checkpointed every 15 minutes, while audit appends add
  a maintenance hint only when an existing 16 MiB segment rolls. Due work runs
  only while all Sessions are idle and is serialized against browser mutations;
  Settings policy saves and the user-visible Top-files list remain explicit
  maintenance triggers.

## Turn Concurrency Stress Standard

Turn lifecycle/final-answer/input-admission changes require a small number of heavy stress scenarios, not a large list of shallow synchronous cases. The normative design and budgets are in `docs/turn-state-projection-architecture.md` section 13.

Required properties:

- real independent Core/model and user/Host threads or tasks;
- real `CoreSessionWorker`, Pod command/projection, persistence, WebSocket, and release-browser paths where applicable;
- a controllable fake model may sleep briefly, but sleeps never establish ordering or causality; barriers/test hooks hit exact race windows;
- seeded jitter and hundreds to thousands of iterations inside each test binary;
- command/attachment ownership, final projection, resource cleanup, and user-visible latency percentiles are all asserted;
- failures print a replayable seed and named stage trace.

The implementation gate must add and run four heavy scenarios: PromptCut/terminal ownership, Stop/Start lifecycle storm, WebSocket reconnect/FIFO ownership, and real-Chrome interaction latency. PR CI runs at least 300 iterations per core scenario, Linux/macOS release certification at least 1,000, and scheduled/manual soak runs 10,000 or ten minutes. Do not replace these with repeated pure reducers or by running the entire workspace thousands of times.

Latency evidence follows the same rule as Web performance tracing: use monotonic elapsed durations within one clock domain and command-correlated named stages. Timestamp order is not causality, and browser/server wall clocks must not be subtracted. Fake-model delay and intentional reconnect backoff are reported separately from Timem-added latency.

## Feature Coverage Matrix

| Feature area | Function / unit coverage | Integration / E2E coverage | Repeated edge coverage |
|---|---|---|---|
| Model service config, protocol, URL, output/input limits | `model_service_config_from_sources`, `parse_cli_args_reads_model_service_and_limits`, protocol endpoint-default tests, protocol adapter tests | startup banner and `/config` real TTY smoke including protocol switching and explicit endpoint preservation | full CI |
| Model response parsing and errors | OpenAI-compatible, OpenAI Responses, Anthropic usage/error tests | `truncated_native_sse_recovery_guides_small_tool_iteration_to_correct_answer` plus truncated native-tool argument recovery tests; transient model service error retry session test; protocol repair session test with audit assertions | edge regression session group |
| Prompt cache planning | `prompt_cache_strategy_*`, prefix-cache simulator tests with bounded lookback, model request cache-control tests, Anthropic cache read/create usage tests, `scripts/kvc_replay_test.sh`, `scripts/kvc_replay.py` local audit replay | `session_turn_preserves_incremental_prompt_cache_plan_across_rounds`, `session_turn_preserves_cache_plan_with_json_response_protocol`, `session_turn_preserves_cache_plan_with_xml_response_protocol`, request audit redaction/hash tests | full CI runs JSON/XML replay fixture coverage; run local audit replay before cache-strategy releases |
| Prompt delta/slice rendering | prompt segmentation, multi-slice core tests, focused response-repair slice tests | shrink session E2E | edge regression shrink group |
| Forced shrink | core shrink threshold, stale observed-token invalidation, static-dominant guard | `session_turn_forced_shrink_runs_to_final_without_repeated_shrink` | edge regression shrink + session groups |
| Scratch notes and context compact offload | scratch write/read/query/delete, context_compact discard/offload refs, invalid refs, missing fields | `session_turn_scratch_context_offload_records_id_and_continues` | session group |
| Durable memory | query/update/delete, expected version, SQL read surface | realistic multi-turn memory story | memory concurrency + realistic story groups |
| Multi-CLI memory conflicts | mem guard cross-process and same-version conflict tests | realistic story exercises shared storage shape | memory concurrency group |
| Chat history | persisted query, delete, SQL time-window, current prompt fallback | realistic story | full CI |
| Bash actions | approval risk, normal shell, background jobs, documented `ask/approve` parsing | bash approval session E2E | shell job group |
| Runtime self tool | `self_tool::tests::*`, manifest/registry/executor tests, `path`/`cwd`/`params` schema and conditional `new_path` validation, complete relevant path/parameter output, URL credential/query redaction, sensitive and arbitrary-env exclusion | core action replay for paths and effective Session/Core params; cwd success/failure, relative resolution, subsequent local-tool cwd, structured `context_state.cwd`, Web Session synchronization, and config-change notice coalescing/isolation | full CI |
| User scenario replay | focused core replay tests for coding, memory QA, self QA/env update, and file-writing output | `scenario_coding_inspects_project_and_reports_from_shell_evidence`, `scenario_memory_qa_retrieves_durable_and_raw_chat_before_answering`, `scenario_self_qa_and_runtime_env_update_stays_bounded`, `scenario_file_writing_outputs_artifact_and_verifies_content` | full CI |
| Background jobs | `run_bash` pid start, timeout-to-running, exit update, per-model-request running table with tool-call attribution, PID-reuse identity rejection, and registered command-tool job tests | realistic story where applicable | shell/tool job groups |
| Multi-turn replay story | protocol parsing, memory/scratch/shrink primitives | `session_replay_story_covers_repair_memory_scratch_shrink_and_observation_rendering` | full CI |
| Session worker lifecycle | lifecycle topic/accessor, worker channel tests | `session_worker_emits_lifecycle_runs_turn_and_accepts_mid_turn_supplement`, `worker_option_returns_late_supplement_after_preserving_the_first_final_answer`, `session_worker_rename_emits_updated_identity_topic`, `session_worker_shutdown_cancels_pending_host_decision`, `core_lifecycle_topic_round_trips_worker_identity_workspace_and_context` | full CI |
| Round limit continuation | core continuation tests | `session_turn_round_limit_continue_recharges_and_finishes_same_task` | session group |
| Cancellation | cancel before model call, command cancellation tests | real TTY Ctrl+C smoke | real TTY smoke |
| Interactive input | CJK width, paste placeholder, Shift+Enter, control stripping, true multiline submitted-line redraw row counts, thinking-time next-question queue capture | real TTY multiline/paste/config/workspace smokes plus local fake-model-server supplement smoke and stress smoke | real TTY smoke/stress in CI |
| Observation panel | observation event/rendering tests | thinking view tests including retry, repair-count status, model-response topics, and global working-worker count | full CI |
| Profiling | profiler aggregation and storage tests | `session_turn_records_cached_tokens_in_profiler_and_latest_usage`, `/prof` real TTY smoke | real TTY smoke |
| Runtime performance and disk I/O | `performance_guard_large_context_prompt_render_is_bounded`, `performance_guard_many_overlay_capabilities_render_is_bounded`, `performance_guard_topic_generation_for_many_actions_is_bounded`, `performance_guard_many_observation_events_render_bounded`, idle-maintenance trigger tests | `scripts/runtime_io_guard.py` measures a fully started idle interval plus real TTY model/action work at ≤500,000 B/s average; Settings-only Top scan, cumulative-runtime checkpoints, audit-roll hints, and idle reconciliation are separately protected | `scripts/performance_guard.sh` plus runtime I/O report in full CI |
| Audit and secrets | append audit, action grouping, redaction tests, sensitive scan | session tests assert turn/action/retry/repair audit records | sensitive scan + full CI |
| Install/update scripts | install logic tests, install run-hint contract | CI script syntax and install logic | full CI |
| Local Web host and UI | host auth/path/config/session tests, frontend session reducers and rendering contracts | real concurrent core workers, work-instruction decision flow, structured cwd updates, production Vite build, real browser smoke | Linux/macOS full CI plus release browser review |

## CI Gates

`scripts/ci.sh` must run:

1. shell script syntax checks
2. module boundary check via `scripts/module_boundary_check.sh`
3. install logic tests
4. contract checks and static prompt expansion snapshot checks
5. sensitive scan over tracked files
6. KVC replay script check
7. `cargo fmt --check`
8. clippy warning gate via `scripts/clippy_check.sh`
9. `cargo test --workspace`
10. Web dependency license scan, frontend functional tests, reproducible production build, and Linux real-Chrome acceptance
11. dedicated performance guard via `scripts/performance_guard.sh`
12. repeated edge regression via `scripts/edge_regression.sh`
13. CLI and Web release builds
14. cross-host resume smoke
15. real TTY smoke through `expect`, including the 500,000 B/s average runtime I/O gate
16. whitespace check

`scripts/edge_regression.sh` defaults to two iterations. Increase pressure with:

```bash
TIMEM_EDGE_ITERATIONS=5 scripts/edge_regression.sh
```

When adding a new feature, add it to the matrix and include at least one test in
the lowest practical layer plus an end-to-end or repeated edge test when the
feature crosses runtime state, model actions, UI, storage, shell, or model service
boundaries.
