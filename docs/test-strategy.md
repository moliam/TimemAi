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
   and `session_runtime` fake-model tests for this axis.

2. UI display correctness

   The shell UI must accurately and clearly represent what the runtime is doing.
   These tests prove observation rendering, status/token lines, config/banner
   layout, input editing, paste recovery, menus, elapsed time, cancellation
   prompts, and that internal protocol names or model-private thought are not
   leaked. Prefer render contract tests plus real pseudo-TTY smoke for this
   axis.

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
   concurrent state, pseudo-TTY smoke, or race-prone paths stay stable.

## Required Layers

- Function tests: pure parsing, formatting, prompt cache planning, provider
  payload shaping, token/status rendering, path normalization, and redaction.
- Unit tests: `agent_core` actions and storage behavior with real temp files.
- Integration tests: complete `session_runtime` turns with a fake model client,
  real `AgentCore`, real action execution, real audit writes, and UI decisions.
- Replay story tests: scripted multi-turn user/model conversations that exercise
  normal replies, malformed model recovery, memory retrieve, scratch offload,
  context shrink, audit writes, and observation rendering in one end-to-end
  path.
- Real TTY smoke: compiled release binary driven through a pseudo terminal for
  input/editor/menu behavior.
- Repeated edge regression: high-risk state machines run multiple times in CI
  through `scripts/edge_regression.sh`.

## Feature Coverage Matrix

| Feature area | Function / unit coverage | Integration / E2E coverage | Repeated edge coverage |
|---|---|---|---|
| Provider config, protocol, URL, output/input limits | `provider_config_from_env`, `parse_cli_args`, provider switch default-reset tests, protocol adapter tests | startup banner and `/config` real TTY smoke including provider switch/default URL validation | full CI |
| Provider response parsing and errors | OpenAI-compatible, OpenAI Responses, Anthropic usage/error tests | truncated output expansion session test; transient provider error retry session test; protocol repair session test with audit assertions | edge regression session group |
| Prompt cache planning | `prompt_cache_strategy_*`, provider request cache-control tests | `session_turn_preserves_incremental_prompt_cache_plan_across_rounds`, request audit redaction/hash tests | full CI |
| Prompt delta/slice rendering | prompt segmentation, multi-slice core tests, focused response-repair slice tests | shrink session E2E | edge regression shrink group |
| Forced shrink | core shrink threshold, stale observed-token invalidation, static-dominant guard | `session_turn_forced_shrink_runs_to_final_without_repeated_shrink` | edge regression shrink + session groups |
| Scratch notes and context offload | scratch write/read/query/delete, invalid refs, missing fields | `session_turn_scratch_context_offload_records_id_and_continues` | session group |
| Durable memory | query/update/delete, expected version, SQL read surface | realistic multi-turn memory story | memory concurrency + realistic story groups |
| Multi-CLI memory conflicts | mem guard cross-process and same-version conflict tests | realistic story exercises shared storage shape | memory concurrency group |
| Chat history | persisted query, delete, SQL time-window, current prompt fallback | realistic story | full CI |
| Bash actions | approval risk, foreground shell, readback, background jobs, documented `ask/approve` parsing | bash approval session E2E | shell job group |
| Runtime self tool | `self_tool::tests::*`, manifest/registry/executor tests, sensitive/protected env denial tests | core action replay for env/path/about/process plus UI observation tests | full CI |
| User scenario replay | focused core replay tests for coding, memory QA, self QA/env update, and file-writing output | `scenario_coding_inspects_project_and_reports_from_shell_evidence`, `scenario_memory_qa_retrieves_durable_and_raw_chat_before_answering`, `scenario_self_qa_and_runtime_env_update_stays_bounded`, `scenario_file_writing_outputs_artifact_and_verifies_readback` | full CI |
| Shell jobs | background start/poll/status timeout tests | realistic story where applicable | shell job group |
| Multi-turn replay story | protocol parsing, memory/scratch/shrink primitives | `session_replay_story_covers_repair_memory_scratch_shrink_and_observation_rendering` | full CI |
| Round limit continuation | core continuation tests | `session_turn_round_limit_continue_recharges_and_finishes_same_task` | session group |
| Cancellation | cancel before provider call, command cancellation tests | real TTY Ctrl+C smoke | real TTY smoke |
| Interactive input | CJK width, paste placeholder, Shift+Enter, control stripping, true multiline submitted-line redraw row counts | real TTY multiline/paste/config/workspace smokes | real TTY smoke in CI |
| Observation panel | observation event/rendering tests | thinking view tests including retry and repair-count status | full CI |
| Profiling | profiler aggregation and storage tests | `session_turn_records_cached_tokens_in_profiler_and_latest_usage`, `/prof` real TTY smoke | real TTY smoke |
| Audit and secrets | append audit, action grouping, redaction tests, sensitive scan | session tests assert turn/action/retry/repair audit records | sensitive scan + full CI |
| Install/update scripts | install logic tests, install run-hint contract | CI script syntax and install logic | full CI |

## CI Gates

`scripts/ci.sh` must run:

1. shell script syntax checks
2. install logic tests
3. sensitive scan over tracked files
4. `cargo fmt --check`
5. `cargo test --workspace`
6. repeated edge regression via `scripts/edge_regression.sh`
7. release build
8. real TTY smoke through `expect`
9. whitespace check

`scripts/edge_regression.sh` defaults to two iterations. Increase pressure with:

```bash
TIMEM_EDGE_ITERATIONS=5 scripts/edge_regression.sh
```

When adding a new feature, add it to the matrix and include at least one test in
the lowest practical layer plus an end-to-end or repeated edge test when the
feature crosses runtime state, model actions, UI, storage, shell, or provider
boundaries.
