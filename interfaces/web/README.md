# Timem Web UI

`interfaces/web` is the browser presentation layer for Timem. It uses
assistant-ui primitives for the chat surface and renders structured events from
`timem_web` / `agent_core`.

The UI owns:

- session list, rename, mem-space display, and session switching
- composer behavior, attachments, queued next-turn questions, explicit active-turn supplements, and inline decisions
- process frames for free talk, actions, repairs, context compaction, and
  runtime requests
- final answer Markdown rendering, code highlighting, token/time telemetry,
  themes, fonts, and responsive layout

The UI must not implement model calls, prompt parsing, memory/tool execution,
or command approval policy. Those are core/host responsibilities.

## Development

Install dependencies once:

```bash
pnpm --dir interfaces/web install --frozen-lockfile
```

Run checks after UI changes:

```bash
pnpm --dir interfaces/web test --run
pnpm --dir interfaces/web build
cargo test -p timem_web
```

Commit application source, tests, lockfile updates, and rebuilt `dist` assets
together. Do not commit `node_modules` or the optional upstream source checkout
under `interfaces/web/vendor`.

## Design Contract

The browser reducer is deliberately session-aware. Every WebSocket event must be
scoped by `session_id`, and worker/context scoped core topics must be rejected
when they do not belong to the target Session. Tests in
`interfaces/web/tests` cover queued next-turn questions, explicit active-turn supplements, duplicate cancel/submit
pressure, concurrent sessions, inline decisions, attachments, bounded event
windows, rendering contracts, and long-history behavior.

Read [`module_boundary.md`](module_boundary.md) before changing Web/core
responsibilities.

## Shared model endpoints

The model label in the chat header opens a mem-scoped endpoint list shared by all Sessions. Users can add, edit, delete, and select endpoints. Selecting one applies its model, API/response protocols, Base URL, API key, maximum context window, and maximum output to the current idle Session. Endpoint editors offer context windows of `100K`, `200K`, or `1M`, and output limits of `10K`, `20K`, or `50K`; older saved endpoints load as `100K` / `10K`. Endpoint API keys are stored in the host memory directory with private file permissions and are redacted from snapshots and browser-persistent command queues.
