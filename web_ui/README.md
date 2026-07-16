# Timem Web UI

`web_ui/timem-web` is the browser presentation layer for Timem. It uses
assistant-ui primitives for the chat surface and renders structured events from
`timem_web` / `agent_core`.

The UI owns:

- session list, rename, mem-space display, and session switching
- composer behavior, attachments, active-turn supplements, and inline decisions
- process frames for free talk, actions, repairs, context compaction, and
  runtime requests
- final answer Markdown rendering, code highlighting, token/time telemetry,
  themes, fonts, and responsive layout

The UI must not implement provider calls, prompt parsing, memory/tool execution,
or command approval policy. Those are core/host responsibilities.

## Development

Install dependencies once:

```bash
pnpm --dir web_ui/timem-web install --frozen-lockfile
```

Run checks after UI changes:

```bash
pnpm --dir web_ui/timem-web test --run
pnpm --dir web_ui/timem-web build
cargo test -p timem_web
```

Commit application source, tests, lockfile updates, and rebuilt `dist` assets
together. Do not commit `node_modules` or the optional upstream source checkout
under `web_ui/vendor`.

## Design Contract

The browser reducer is deliberately session-aware. Every WebSocket event must be
scoped by `session_id`, and worker/context scoped core topics must be rejected
when they do not belong to the target Session. Tests in
`web_ui/timem-web/tests` cover active-turn supplements, duplicate cancel/submit
pressure, concurrent sessions, inline decisions, attachments, bounded event
windows, rendering contracts, and long-history behavior.

Read [`module_boundary.md`](module_boundary.md) before changing Web/core
responsibilities.
