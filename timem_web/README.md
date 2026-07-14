# Timem Web Host

`timem-web` is Timem's loopback-only browser host. It creates `agent_core`
session workers, exposes a token-protected WebSocket/API surface, and serves
the embedded React application from `web_ui/timem-web`.

```bash
cargo run -p timem_web
```

The host chooses a port in `12345..=23456`; `--port` selects a specific port in
that range. It prints the local URL once at startup. Do not expose that URL or
its token outside the local machine.

Before modifying frontend code, follow the rebuild and test sequence in
[`web_ui/README.md`](../web_ui/README.md). The frontend production assets are
tracked because `build.rs` embeds them into the Rust binary.
