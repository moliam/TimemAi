# Timem Web Host

`timem-web` is Timem's local browser host. It is not a separate agent runtime:
it creates and manages `agent_core` session workers, serves the embedded
assistant-ui frontend, and forwards structured core topics to the browser.

```bash
timem-web
# or, from source:
cargo run -p timem_web
```

## Runtime Model

- Binds only to `127.0.0.1`.
- Chooses a port in `12345..=23456`; `--port` selects a specific port.
- Generates a per-process access token for HTTP/WebSocket requests.
- Opens the authenticated local page when a local graphical session is detected;
  SSH/headless sessions print the URL without launching a browser.
- Keeps provider calls, prompt building, memory, tools, and response parsing in
  `agent_core`.

## Sessions

A Web Session owns a runtime profile/env snapshot plus its Context and Worker
registries. Today the UI creates one default Context and one primary Worker per
Session, but the host already routes child-worker topics through the primary
conversation. Different Sessions can use different model/provider settings.

Session state, chat history, and resume metadata are persisted through
`agent_core::session_store` so Shell and Web can continue the same mem-space
work.

## Frontend Assets

The production frontend is built from `web_ui/timem-web` and tracked under
`web_ui/timem-web/dist` because `build.rs` embeds those files into this Rust
binary. Release users do not need Node or a separate assistant-ui checkout.

Before changing frontend behavior, follow
[`web_ui/README.md`](../web_ui/README.md), then run at least:

```bash
pnpm --dir web_ui/timem-web test --run
pnpm --dir web_ui/timem-web build
cargo test -p timem_web
```

Read [`module_boundary.md`](module_boundary.md) before changing host/core
responsibilities.
