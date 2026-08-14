# TimemAi 1.1.1

TimemAi 1.1.1 is the quality-hardening update for the recommended Timem Web
experience. Install TimemAi, run `timem-web`, and configure the selected
Session directly in the browser.

## Highlights

- **Simple Web-first setup:** the authenticated local page opens from the
  `timem-web` command; model, API protocol, Base URL, and API key are configured
  per Session in the Web UI.
- **Reliable concurrent Sessions:** command acknowledgements, durable browser
  queues, sequenced event replay, FIFO Session lanes, and isolated restore
  batches preserve user intent across multiple Sessions, tabs, reconnects, and
  Host restarts.
- **Atomic Session metadata:** concurrent Host and Web activity can no longer
  expose a partially written Session index or overwrite another Session's
  record. Index replacement is serialized, durable, atomic, and covered by a
  deterministic multi-instance regression test.
- **Complete browser workspace:** Session management, chat history, queued
  message editing and reordering, attachments, live Thought/Action status,
  context usage, Markdown and code rendering, MCP, and ToolRepo remain
  available without leaving the Web UI.
- **Security and performance:** credentials remain outside durable browser
  queues, public listening stays explicit, event and recovery storage remain
  bounded, and production compilation is warning-free.

## Verification

The exact release commit must pass the complete production gate locally and on
GitHub for Ubuntu and macOS. The gate covers Rust formatting and warning-free
Clippy, workspace and Web tests, deterministic concurrency/restart/failure
injection, frontend type checks and reproducible production builds, dependency
license and sensitive-data scans, performance and repeated-edge tests, release
builds, cross-host resume, loopback services, and pseudo-TTY smoke tests.
