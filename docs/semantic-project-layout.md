# Semantic Project Layout

This document is the normative physical ownership and dependency contract. The
system overview is in [Architecture](architecture.md).

## Runtime model

```text
Interface ↔ Bridge ↔ Core
```

Interface owns presentation, Bridge owns communication, Core owns reusable and
authoritative semantics, and Application owns concrete product assembly. A
Bridge is not necessarily a process boundary.

## Target physical layout

Only implemented directories are present. Commented extension points below are
names, not placeholders to create.

```text
core/
  agent/                    # model, prompt, capabilities, tools, memory, Turn engine
  session/                  # Session/Context/Worker lifecycle and use cases
  ui_contract/              # UI-neutral commands, events, topics, projections
  platform/                 # target-neutral and target-specific OS policy
bridges/
  in_process/               # typed calls, callbacks, and channels
  http_websocket/           # HTTP/WebSocket routing and reconnect delivery
  ipc/                      # future: only with a real separate-process consumer
interfaces/
  shell/                    # terminal interaction and rendering
  web/                      # browser interaction and rendering
applications/
  timem/                    # unified composition root and executable
resources/                  # capability manifests, tool implementations, resources
tests/                      # cross-module and product-level tests
docs/                       # architecture, operation, protocol, and delivery docs
scripts/                    # quality gates and delivery automation
```

Do not create `bridges/ipc`, `bridges/native_ffi`, `interfaces/desktop`, or a
second Application until implementation, consumer, contract, and executable
tests arrive together. Empty placeholders and directory-only platform claims
are forbidden.

## Dependency direction

```text
core/platform ───────────────┐
core/ui_contract ────────────┼──> core/agent ──> core/session
                             │                         ▲
                             └─────────────────────────┘

core/{session,ui_contract} <── bridges/* <── interfaces/*
applications/* assembles the complete graph
```

Rules:

1. `core/platform` depends on no Agent, Session, Bridge, or Interface crate.
2. `core/ui_contract` contains data contracts and pure helpers; it depends on no
   Session orchestration, Bridge, or Interface.
3. `core/agent` may consume platform and UI-contract types, but never Session,
   Bridge, Interface, or Application layers.
4. `core/session` may depend on Agent, UI contract, and platform policy.
5. Bridges depend inward on Core and add communication mechanics only.
6. Interfaces depend on their Bridge and shared UI contracts. Core never depends
   on an Interface, and Interfaces never depend on one another.
7. Applications compose products and are not reusable dependencies of lower layers.
8. Physical moves preserve package, binary, CLI, persisted-data, and wire
   compatibility unless an explicitly reviewed change documents and tests an exception.

## Current graph

```text
applications/timem
  -> interfaces/shell
  -> bridges/in_process
  -> bridges/http_websocket
  -> core/session
  -> core/agent
  -> core/platform

interfaces/shell -> bridges/in_process
interfaces/shell -> core/ui_contract
bridges/in_process -> core/session
bridges/in_process -> core/ui_contract
bridges/http_websocket -> core/ui_contract
core/session -> core/agent
core/session -> core/ui_contract
core/agent -> core/platform
core/agent -> core/ui_contract
```

`interfaces/web` is a TypeScript project. It consumes the HTTP/WebSocket wire
contract rather than becoming a Cargo dependency. `applications/timem` embeds
its tracked `dist` bundle.

## Current architecture contract

The normative 2.0 state is:

- `applications/timem` owns the executable and Cargo package named `timem`;
  `timem-web` is installer command compatibility only.
- Shell reaches Session use cases through `bridges/in_process`.
- HTTP/WebSocket routing and reusable delivery mechanics live in
  `bridges/http_websocket`; the Application supplies product composition and
  concrete handlers.
- The in-process Bridge serves **all same-process Rust Interfaces**, not only
  Shell.
- FFI or IPC Bridges exist only when required by a real, tested consumer and
  adapt to Core semantics rather than duplicating them.

## Non-regression invariants

- Core is authoritative for Session, Context, Worker, Turn, admission,
  cancellation, terminal outcomes, and structured Interface decisions.
- Same-process Interfaces do not acquire networking or serialization overhead.
- Browser delivery keeps bounded ordering, deduplication, reconnect baselines,
  authentication, and backpressure behavior.
- ACK state is not substituted for authoritative business projections.
- Public semantic types are not replaced by generic text envelopes.
- Tests move with behavior; production `src` contains only minimal test hooks.
- New directory names never claim unsupported platform or product behavior.
