# Semantic Project Layout

## Runtime model

Timem uses one architectural direction:

```text
Interface ↔ Bridge ↔ Core
```

- **Interface** owns presentation and human interaction.
- **Bridge** owns communication: direct calls/callbacks/channels, HTTP/WebSocket, or IPC.
- **Core** owns reusable agent semantics, Session/Context/Worker orchestration, UI-neutral
  contracts, and platform policy.

A Bridge is a logical communication boundary, not a process boundary. Shell and same-process Rust Interfaces may
use the in-process Bridge without serialization or networking. Browser clients use HTTP/WebSocket.
A separately running desktop companion may use HTTP/WebSocket or a real IPC Bridge when implemented. No Bridge may invent Agent, Session, Turn,
cancellation, approval, retry, or lifecycle semantics.

## Target physical layout

```text
core/
  agent/                    # model loop, capabilities, prompt/protocol and agent execution
  session/                  # Session/Context/Worker lifecycle, scheduling, and use-cases
  ui_contract/
    commands/               # UI-neutral requests entering Session use-cases
    events/                 # semantic events emitted by Core
    projections/            # authoritative UI-readable state
  platform/
    api/                     # target-neutral platform contract
    shared/                  # implementation shared by multiple targets
    macos/
    windows/
    linux/
bridges/
  in_process/               # direct functions, callbacks and channels
  http_websocket/           # browser HTTP/WebSocket and reconnect delivery
  native_ffi/               # create for a real cross-language same-process client
  ipc/                      # create for a real desktop companion process transport
interfaces/
  shell/
  web/
  desktop/                 # create only with a real implementation
applications/
  timem/                   # unified CLI/Web/Shell composition root
  timem_desktop/           # create only with a real desktop product
resources/
tests/
docs/
scripts/
```

The tree describes ownership, not a requirement that every leaf be a separate crate. A directory is
created only with a real implementation, contract, test, or explicitly truthful unsupported-target
behavior. Empty placeholders and directory-only claims of platform support are forbidden.

## Dependency direction

The intended compile-time direction is:

```text
core/platform ───────────────┐
core/ui_contract ────────────┼──> core/agent ──> core/session
                             │                         ▲
                             └─────────────────────────┘

core/{session,ui_contract} <── bridges/* <── interfaces/*
```

More precisely:

1. `core/platform` depends on no Agent, Session orchestration, Bridge, or Interface crate.
2. `core/ui_contract` contains data contracts and pure contract helpers. It depends on neither
   Session orchestration nor any Bridge/Interface.
3. `core/agent` owns model/capability execution and may consume platform and UI-contract types. It
   must not depend on Session orchestration, Bridges, or Interfaces.
4. `core/session` owns Session/Context/Worker lifecycle, scheduling, and use-cases. It may
   depend on agent, UI-contract, and platform crates.
5. Bridges depend inward on Core. Bridges may add transport identity, ordering, serialization,
   replay, reconnect, and backpressure metadata, but not domain lifecycle rules.
6. Interfaces depend on the appropriate Bridge and shared UI contracts. Core never depends on an
   Interface, and one Interface never depends on another.
7. Package, binary, CLI, persisted-data, and wire-protocol compatibility stays stable during
   physical moves unless a separately reviewed change documents and tests the exception.

The arrows above are the target dependency graph. During extraction, temporary re-exports may
preserve callers, but they must not introduce a cycle or become permanent duplicate ownership.

## Migration inventory

Migration complete for the physical runtime roots covered by this change:

- The transitional top-level `timem_web/` directory has been removed.
- The unified executable composition root now lives at `applications/timem/`.
- The unified Application Cargo package and binary are both `timem`; `timem_web` is neither a
  package nor a physical architecture root. `timem-web` may exist only as an installer alias.
- `bridges/in_process` depends on `core/session` and `core/ui_contract`, not directly on Agent.
- `interfaces/shell` depends on `bridges/in_process` and `core/ui_contract`, not directly on Agent or
  Session.
- `bridges/http_websocket` owns fixed HTTP/WebSocket paths and method composition, request bounds,
  static fallback routing, reusable reconnect/delivery state, generic WebSocket framing, bounded
  JSON wire I/O, same-origin validation, and browser-safe transport headers.
  `applications/timem` injects product handlers and retains authentication, snapshots, state, and
  Session/Core command mapping without recreating the route table or a transitional runtime root.

The in-process Bridge is for **all same-process Rust Interfaces**, not specifically Shell. Shell is
the first production consumer. A future Rust-native desktop Interface, embedded GUI, or test Host
reuses the same Bridge. A cross-language same-process desktop client may add `bridges/native_ffi`,
which adapts ABI ownership and callbacks to `bridges/in_process` rather than duplicating Session
semantics. A separate-process desktop client may use HTTP/WebSocket or add IPC when real behavior
and tests exist.

## Extension and no-placeholder rule

The no-placeholder rule remains mandatory. Do not create `interfaces/desktop`,
`applications/timem_desktop`, `bridges/native_ffi`, or `bridges/ipc` until a real implementation,
contract, and executable tests are delivered together. Bridge names follow communication mechanics;
Interface names follow interaction form; Application directories are composition roots for concrete
products.

## Final dependency graph

All arrows mean “depends on”:

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

The browser project communicates with the HTTP/WebSocket Bridge through its wire protocol; it is not
a Cargo dependency. The Application embeds `interfaces/web/dist` and performs top-level lifecycle,
mode selection, and dependency assembly.

## Non-regression invariants

- Core remains authoritative for Session, Context, Worker, Turn, input admission, cancellation,
  terminal outcomes, and structured Interface decisions.
- Shell/native in-process paths do not acquire network, serialization, or reconnect overhead merely
  to satisfy the Bridge abstraction.
- Web keeps bounded command delivery, event ordering, reconnect baselines, authentication, and
  backpressure behavior.
- Public semantic types are not replaced with generic text envelopes.
- Tests move with behavior and remain outside production `src` except minimal external test hooks.
- No step claims macOS, Windows, or Linux support from names alone.
