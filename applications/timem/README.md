# Timem Application

`applications/timem` is the product composition root and owns the `timem` Cargo
package and executable. Web is the default mode; `timem --shell` selects the
terminal Interface.

```bash
cargo run -p timem --
cargo run -p timem -- --shell
```

The Application assembles concrete Core, Bridge, and Interface implementations.
It owns process startup, mode selection, dependency construction, browser asset
embedding, and top-level shutdown. It is not a reusable dependency of lower
layers and does not own duplicate Agent or Session semantics.

## Composition

The Application assembles:

- `interfaces/shell` for terminal interaction;
- `bridges/in_process` for typed same-process Session access;
- `bridges/http_websocket` for browser HTTP/WebSocket transport;
- `core/session` and `core/agent` for orchestration and Turn execution;
- the production bundle generated from `interfaces/web`.

Shell reaches Core through the in-process Bridge. The browser reaches the Host
through the HTTP/WebSocket Bridge. Reusable communication belongs in a Bridge,
reusable semantics in Core, and presentation in an Interface.

## Web host

The Web host:

- binds to `127.0.0.1` unless `--public` is explicit;
- uses tokenless access only for loopback-local mode;
- requires a rotating access token for public browser, API, upload, and WebSocket access;
- selects an automatic port unless `--port` is provided;
- opens a browser only when a local graphical session is available;
- keeps credentials, model calls, prompts, tools, memory, and response parsing
  on the Host.

A Session owns its runtime profile, Contexts, Workers, history, and projections.
Browser snapshots redact secrets. Command acknowledgement is transport control;
Core-derived projections and events remain authoritative for visible state.

## Frontend assets

`build.rs` embeds the tracked `interfaces/web/dist` bundle in the executable.
Release builds therefore do not require Node.js or a separate frontend service.
Web source changes must commit source, tests, lockfile changes when applicable,
and rebuilt `dist` assets together.

Read [`module_boundary.md`](module_boundary.md),
[`interfaces/web/README.md`](../../interfaces/web/README.md), and the repository
[architecture](../../docs/architecture.md) before changing ownership.

Focused validation:

```bash
cargo test -p timem
pnpm --dir interfaces/web test
pnpm --dir interfaces/web build
git diff --exit-code -- interfaces/web/dist
```
