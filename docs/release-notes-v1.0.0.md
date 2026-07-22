# TimemAi 1.0.0

Timem 1.0 introduces the browser as a first-class host for the same local
agent runtime that powers the terminal application.

## Highlights

- Authenticated `timem-web` built on assistant-ui.
- Multiple isolated Sessions with per-session provider/model profiles.
- Persistent history, paged restore, mem switching, and Web/Shell resume.
- Live Thought/Action work streams, inline runtime decisions, supplements,
  cancellation, reconnect handling, and context-compact visualization.
- Markdown/GFM answers, syntax-highlighted code, attachments, cwd display,
  responsive layout, appearance controls, and final usage telemetry.
- Compact tool activity rendering keeps execution details visible without
  letting tool panels overwhelm the conversation.

## Architecture

The browser is a host renderer and transport. `agent_core` remains responsible
for provider calls, prompt/context construction, memory, capabilities, tool
execution, session workers, safety policy, and structured topics. Shell and Web
hosts render those shared structures according to their own environment.

## Verification

The release is covered by the Rust workspace tests, Web reducer/rendering tests,
production frontend build, protocol/capability checks, session isolation and
resume tests, cancellation pressure tests, performance guards, and release CI.
Manual checks are still recommended on the target browser and terminal
environment before using a live provider.
