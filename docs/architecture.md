# TimemAi Architecture

This document is the current architectural overview for the 2.0 codebase. It
explains system boundaries and authoritative ownership. Detailed wire contracts, test matrices, and operational limits live in
focused documents linked below.

## 1. Product model

TimemAi ships one executable and one local runtime:

```text
                         ┌─ Shell Interface ─ In-process Bridge ─┐
Human ─ timem Application│                                      ├─ Core
                         └─ Web Interface ─ HTTP/WebSocket Bridge┘
```

- `timem` starts Web mode by default.
- `timem --shell` starts the terminal Interface.
- Both modes use the same Session, Agent, capability, memory, and persistence
  semantics.
- `timem-web` may be installed as a compatibility alias; it is not an independent
  executable or Cargo package.

The Application embeds the built browser assets from `interfaces/web/dist` and
assembles concrete implementations. It does not own reusable domain semantics.

## 2. Architectural direction

The repository follows:

```text
Interface ↔ Bridge ↔ Core
```

### Interface

Interfaces own human interaction: layout, rendering, input composition,
accessibility, keyboard behavior, and UI-local transient state. They do not own
model calls, tool policy, Session scheduling, or Turn lifecycle.

### Bridge

Bridges own communication mechanics: typed calls, callbacks, channels,
authentication, routing, serialization, ordering, bounded queues, reconnect,
replay baselines, and backpressure. A Bridge is a logical boundary, not
necessarily a process boundary. It must not invent Agent or Session semantics.

### Core

Core owns authoritative behavior: Session/Context/Worker/Turn lifecycle, input
admission, cancellation, terminal outcomes, prompt/model execution, capabilities,
tools, memory, persistence rules, and UI-neutral state contracts.

### Application

The Application chooses the product mode, constructs dependencies, embeds Web
assets, and owns top-level process lifecycle. Lower layers must not depend on it.

## 3. Core ownership

### `core/agent`

Owns the model-facing Turn engine: prompt construction, API protocols, response
parsing and repair, capability negotiation, built-in and MCP tools, memory, audit
integration, and tool execution. It does not own multi-Session orchestration or
presentation.

### Host capability profile

Core exposes capabilities according to the active host profile. The `local command execution` capability is enabled only when that profile permits it and is independent of UI type: Web and Shell do not gain or lose `run_bash` merely because of their
presentation form. The Host supplies approval policy and executable capability;
the Interface renders structured requests and results.

### `core/session`

Owns Session, Context, Worker, and Turn orchestration; command admission;
scheduling; cancellation; Session persistence; and use cases consumed by
Bridges. It is the authority for whether work is active, accepted, completed, or
cancelled.

### `core/ui_contract`

Owns UI-neutral commands, semantic events, projections, topic types, and pure
contract helpers. Contracts remain typed; progress, final answers, decisions,
diagnostics, and lifecycle state are not collapsed into generic text envelopes.

### `core/platform`

Owns reusable operating-system policy and target-specific implementations. It
must not depend on Agent, Session, Bridge, or Interface layers.

## 4. Bridge and Interface paths

### Same-process path

Shell uses `bridges/in_process`, which exposes typed Core use cases without
networking or serialization. This Bridge is for all same-process Rust
Interfaces, not only Shell.

### Browser path

The browser uses authenticated HTTP and WebSocket routes supplied by
`bridges/http_websocket` and composed by `applications/timem`. The Browser sends
commands; the Host maps them to Core use cases and returns authoritative
snapshots, topics, and events.

Command acknowledgement is transport control, not business truth. `accepted`,
`committed`, or `rejected` describes command handling; visible Session and Turn
state comes from Host projections and semantic events.

The browser does not keep a persistent command outbox or replay business
commands after refresh. Once the Host accepts work, browser disconnection,
locking, or closure does not transfer ownership away from the Host process.
Reconnect restores the view from a snapshot baseline and bounded subsequent
events.

See [Web reliability test matrix](web_reliability_test_matrix.md) and
[Core/UI topic protocol](core-ui-topic-protocol.md).

## 5. Runtime state model

A **MEM** is the complete local workspace root. It contains Sessions, memory,
audit data, Roles, workspace configuration, capability overlays, and
diagnostics. The default is `~/.timem/mem`; `--space` and `TIMEM_SPACE` select a
different absolute root.

A running Host exclusively owns one MEM. Session data uses Session-scoped lock
domains, while groups, Roles, jobs, and audit stores use separate domains. This
allows unrelated Sessions to persist and execute without one global mutable
Session index.

Each Session owns its model/runtime profile, working directory, Contexts,
Workers, history, and current projections. Core remains authoritative across
both Web and Shell presentations.

Operational layout, retention, permissions, and data-format compatibility are
documented in [Install and configuration](install-and-configuration.md#runtime-data).

## 6. Ordering, bounds, and failure behavior

Timem's queues, histories, event windows, retries, logs, and durable stores must
be bounded or have an explicit retention policy. Capacity exhaustion is reported
rather than silently creating unbounded state.

The Web transport maintains bounded command lanes, duplicate suppression,
semantic event sequence numbers, and snapshot baselines. Event timestamp order
must not be interpreted as lifecycle causality; semantic IDs and typed state are
the correlation mechanism.

Permission, ownership, destructive actions, process identity, and unsupported
platform behavior fail closed. Errors distinguish invalid input, unavailable
capability, transient failure, cancellation, and internal defects while
redacting secrets.

## 7. Dependency direction

```text
applications/timem
  -> interfaces/shell
  -> bridges/in_process
  -> bridges/http_websocket
  -> core/session
  -> core/agent

interfaces/shell -> bridges/in_process -> core/session
interfaces/*     -> core/ui_contract
bridges/*        -> core/{session,ui_contract}
core/session     -> core/{agent,ui_contract,platform}
core/agent       -> core/{ui_contract,platform}
```

The TypeScript browser project consumes a wire contract; it is not a Cargo
dependency. Core never depends on an Interface, Interfaces do not depend on one
another, and reusable semantics do not move outward for caller convenience.

The exact physical layout and extension rules are normative in
[Semantic project layout](semantic-project-layout.md).

## 8. Extension rules

Add a directory only with a real consumer, explicit contract, implemented
behavior, and executable tests. Do not create placeholder desktop, FFI, IPC, or
platform trees.

- A same-process Rust Interface reuses `bridges/in_process`.
- A cross-language same-process client may add a native FFI adapter only when it
  exists and adapts to the in-process semantics.
- A separate process should reuse HTTP/WebSocket unless a real IPC requirement
  justifies a tested IPC Bridge.
- A new interaction form belongs under `interfaces/`; a concrete product
  composition belongs under `applications/`.

Extensions must not duplicate Session, Turn, cancellation, approval, retry, or
model semantics.

## 9. Development contract

Repository-level rules are in [`AGENTS.md`](../AGENTS.md). Every architectural
module also has a local `module_boundary.md`. Automated checks enforce forbidden
roots, dependency direction, module boundaries, contracts, generated Web assets,
and tests.

Use [Development and validation](development.md) as the contributor entry point.
The complete production gate is `scripts/ci.sh`.
