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

## Required Layers

- Function tests: pure parsing, formatting, prompt cache planning, provider
  payload shaping, token/status rendering, path normalization, and redaction.
- Unit tests: `agent_core` actions and storage behavior with real temp files.
- Integration tests: complete `session_runtime` turns with a fake model client,
  real `AgentCore`, real action execution, real audit writes, and UI decisions.
- Real TTY smoke: compiled release binary driven through a pseudo terminal for
  input/editor/menu behavior.
- Repeated edge regression: high-risk state machines run multiple times in CI
  through `scripts/edge_regression.sh`.

## Feature Coverage Matrix

| Feature area | Function / unit coverage | Integration / E2E coverage | Repeated edge coverage |
|---|---|---|---|
| Provider config, protocol, URL, output/input limits | `provider_config_from_env`, `parse_cli_args`, provider switch default-reset tests, protocol adapter tests | startup banner and `/config` real TTY smoke including provider switch/default URL validation | full CI |
| Provider response parsing and errors | OpenAI-compatible, OpenAI Responses, Anthropic usage/error tests | truncated output expansion session test | edge regression session group |
| Prompt cache planning | `prompt_cache_strategy_*`, provider request cache-control tests | request audit redaction/hash tests | full CI |
| Prompt delta/slice rendering | prompt segmentation and multi-slice core tests | shrink session E2E | edge regression shrink group |
| Forced shrink | core shrink threshold, stale observed-token invalidation, static-dominant guard | `session_turn_forced_shrink_runs_to_final_without_repeated_shrink` | edge regression shrink + session groups |
| Scratch notes and context offload | scratch write/read/query/delete, invalid refs, missing fields | `session_turn_scratch_context_offload_records_id_and_continues` | session group |
| Durable memory | query/update/delete, expected version, SQL read surface | realistic multi-turn memory story | memory concurrency + realistic story groups |
| Multi-CLI memory conflicts | mem guard cross-process and same-version conflict tests | realistic story exercises shared storage shape | memory concurrency group |
| Chat history | persisted query, delete, SQL time-window, current prompt fallback | realistic story | full CI |
| Bash actions | approval risk, foreground shell, readback, background jobs, documented `ask/approve` parsing | bash approval session E2E | shell job group |
| Shell jobs | background start/poll/status timeout tests | realistic story where applicable | shell job group |
| Round limit continuation | core continuation tests | `session_turn_round_limit_continue_recharges_and_finishes_same_task` | session group |
| Cancellation | cancel before provider call, command cancellation tests | real TTY Ctrl+C smoke | real TTY smoke |
| Interactive input | CJK width, paste placeholder, Shift+Enter, control stripping, true multiline submitted-line redraw row counts | real TTY multiline/paste/config/workspace smokes | real TTY smoke in CI |
| Observation panel | observation event/rendering tests | thinking view tests | full CI |
| Profiling | profiler aggregation and storage tests | `/prof` real TTY smoke | real TTY smoke |
| Audit and secrets | append audit, action grouping, redaction tests, sensitive scan | session tests assert turn/action audit records | sensitive scan + full CI |
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
