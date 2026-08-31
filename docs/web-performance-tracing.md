# Web Performance Tracing

Timem Web records structured runtime diagnostics for send/supplement latency and
Session-switch rendering whenever the Web Host runs with `--debug`. It observes
existing behavior; it does not apply a performance optimization.

## Enable and reproduce

Start the Web Host with its existing debug switch:

```bash
cargo run --bin timem -- --debug --space .perf-investigation
```

`--debug` is the only enablement control. Without it, no PID diagnostic directory
or `runtime.log` is created. With it, the existing debug artifacts and
`runtime.log` share one PID diagnostic directory.

The startup output prints the exact file, for example:

```text
Timem Web runtime log: /tmp/timem-debug-12345-0/runtime.log
```

On macOS the temporary root remains `/tmp` to match the existing Web debug
layout. Each process gets one `timem-debug-<pid>-<sequence>` diagnostic directory.
`runtime.log` is the shared structured runtime-log surface: performance stages
are its first event family, and other bounded runtime log families may use the
same file later. Each line is one JSON object. The file is capped at 20 MiB and
is truncated/reused in place when the next record would exceed the cap. A
`log_wrapped` record marks a wrap when the marker and triggering record fit
together.

The diagnostic directory is temporary and follows the existing DebugStore
lifecycle. Copy `runtime.log` while the Host is still running, after completing
the reproduction. Normal Host shutdown removes the PID directory.

Use one controlled scenario per run:

1. Open one Session and submit an initial task.
2. While that task is active, send one explicit immediate supplement.
3. Wait until its updated turn is visible.
4. Switch to another Session and back once.
5. While the Host is still running, copy the printed `runtime.log` file.

Do not compare an instrumented run directly with an uninstrumented run as if the
trace had zero overhead. Use repeated instrumented runs for relative diagnosis.

## Correlation and stages

Every task/supplement measurement must be grouped by `fields.command_id`.
Session-switch measurements use their generated `session-switch-*` command ID.
Timestamp order alone is not evidence that one stage caused the next stage.
Browser reports use a separate HTTP request and can be written after a later
WebSocket stage. A remote browser clock can also differ from the Host clock.
Therefore, never subtract `timestamp_ms` across browser and server records. Use
browser `elapsed_ms` for browser end-to-end duration and the named server duration
fields for server-local segments.

Command stages:

| Stage | Meaning |
|---|---|
| `browser_send` | Browser prepared the command immediately before WebSocket send. |
| `server_received` | Host parsed the WebSocket command. |
| `server_execute_start` | Ordered command worker began execution; `queue_ms` measures accepted-to-start delay. |
| `turn_core_enqueue_start` / `turn_core_enqueue_finish` | Host began and completed admission of a newly published Web Turn into the primary Core worker; the finish record includes Host-local `elapsed_ms`. |
| `core_turn_started_consumed` | Host consumed Core's typed `TurnStarted` event and can activate/publish the authoritative Turn. |
| `core_model_request_consumed` | Host consumed a Core `ModelRequest`; includes producer-to-consumer `event_delay_ms` and prompt/API payload sizes. |
| `debug_prompt_persisted` | Debug prompt artifacts were persisted; includes state, render/write, total persistence, and emitted-to-persisted durations. |
| `debug_prompt_persist_failed` | Debug prompt persistence failed; includes the bounded failure reason. |
| `supplement_handle_start` | Host entered supplement handling. |
| `supplement_active_turn_observed` / `supplement_no_active_turn` | Host's active-turn branch decision. |
| `supplement_core_accepted` / `supplement_core_rejected_closed_turn` | Core accepted the supplement or reported that the turn had closed. |
| `closed_turn_settle_*` | Closed-turn recovery drained pending worker events, finished, or timed out. |
| `supplement_published` | Updated turn was persisted/published; includes append, publish, total, event, and user-entry measurements. |
| `server_execute_handled` | Command handler returned; includes execution duration and success. |
| `browser_turn_updated` | Matching command ID reached the browser's first authoritative `turn_updated`. |
| `browser_painted` | First animation frame after that matching update. |

Session-switch stages are `browser_session_selected` and
`browser_session_painted` with the same generated command ID.

### Initial capability negotiation

For a Session/model combination whose native tool-call capability is not yet
resolved, Core may perform a capability probe after `core_turn_started_consumed`
and before the first formal `core_model_request_consumed`. A large interval in
that sequence does not prove that Turn activation, Web delivery, or debug prompt
persistence caused the delay. Correlate provider audit evidence and compare a
subsequent request after the capability result is cached.

Likewise, `debug_prompt_persisted.record_total_ms` measures only the synchronous
debug persistence segment after the ModelRequest reaches the Host.
`emitted_to_persisted_ms` also includes event-queue delay. Neither field includes
time spent before Core emits that ModelRequest.

## Evidence-based interpretation

Prefer repeated examples with the same pattern. These deductions isolate
segments; they do not claim causality from sequence alone.

- Large `queue_ms` while handler/publish/browser deltas stay small predicts that
  command-worker contention or an earlier ordered command is the dominant segment.
- Small queue time but large `supplement_published.fields.append_ms`, reproduced
  with larger active turns, predicts that supplement append/core synchronization
  is the segment to profile next.
- Large `publish_ms` predicts semantic-envelope construction or in-memory broadcast cost.
  Confirm with host CPU/lock profiling and receiver lag; this path no longer reads or writes
  a semantic-event journal.
- Small server-local segments but large browser `browser_turn_updated.elapsed_ms`
  leaves delivery/event-queue/state reduction as a hypothesis. Confirm it with a
  browser Performance/WebSocket profile; do not derive the gap by subtracting
  cross-clock timestamps.
- A small update elapsed time but large `browser_painted.elapsed_ms -
  browser_turn_updated.elapsed_ms` predicts browser main-thread/render delay.
  Confirm with the browser Performance panel at the same action.
- `supplement_core_rejected_closed_turn` followed by large
  `closed_turn_settle_finish.elapsed_ms` predicts a completion-race recovery path,
  not ordinary active-turn supplement handling.
- A large Session selected-to-painted duration predicts Session projection/render
  work only; it says nothing about command execution. Compare a cold first open
  with repeated A/B/A switching: after both timelines have been visited, a large
  remaining duration disproves remounting as the sole cause and points to reveal
  layout, scroll restoration, or synchronous geometry work. A substantially
  faster warm return is the expected signature of the two-pane timeline cache.

Absence of a later stage is also evidence to classify: check authorization or
connection failure for missing browser records, command rejection for missing
execution records, and command-ID propagation for missing matching update/paint.

## Privacy and limits

The current performance events in `runtime.log` record IDs, stage names,
durations, counts, process ID, and timestamps. They do not record user message text, model output, API keys, tokens, attachment
contents, environment values, or endpoint URLs. Treat Session/turn/command IDs as
local diagnostic metadata and review the file before sharing it.

## Repeatable microbenchmarks

Run all performance guards:

```bash
scripts/performance_guard.sh
```

Run only the Web browser hot-path experiments:

```bash
TIMEM_PERF_GUARD=1 pnpm --dir interfaces/web exec vitest run tests/performance_guard.test.ts
```

The Web experiments print elapsed time for 20,000 action lifecycle events, a
50,000-event frame queue burst, and 50,000 combined scroll-invalidation/warm-A/B
cache cycles. The last guard also asserts that scroll invalidation requests no
geometry or floating-layout work while both warm Session panes remain cached.
These thresholds are intentionally broad CI regression alarms, not product
latency targets and not proof of browser rendering smoothness. Compare repeated
runs on the same machine/build. Use command-correlated JSONL stages and a browser
Performance profile to isolate render, reveal-layout, scroll-listener, or
synchronous-geometry work; timestamp order alone does not establish causality.
