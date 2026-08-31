# In-Process Bridge Boundary

`bridges/in_process` is the reusable zero-transport Bridge for **all Rust Interfaces that live in
the same process** as Timem Core. Shell is its first consumer, not its owner. A future Rust-native
desktop, embedded GUI, test host, or other same-process Rust Interface should reuse this Bridge
instead of reaching directly into Session or Agent.

It adapts typed calls and callbacks without serialization, networking, reconnect state, or a
separate process. Its public surface is built on `core/session` and `core/ui_contract`; Session is
the lifecycle entry point and may expose the Agent API types needed by callers.

## Current layout

- `src/lib.rs`: synchronous Turn entry points that forward typed inputs, callbacks, and results
  through Session, plus deliberate re-exports of the public Agent and UI contract types needed by
  Rust Interfaces.
- `tests/turn_bridge_tests.rs`: deterministic proof that Turn projections, topics, and outcomes pass
  through the Bridge without reinterpretation.

## Dependency direction

- Required inward dependencies: `core/session` and `core/ui_contract`.
- Forbidden direct dependency: `core/agent`; Session owns that dependency and API mediation.
- Forbidden outward dependencies: Shell, Web, desktop, application roots, or any host implementation.
- An Interface using this Bridge must not also bypass it with a direct Session or Agent dependency.

## Semantic rules

- The Bridge must not parse model responses, execute tools, invent lifecycle state, render
  presentation, or add transport-only identity/replay semantics.
- Interfaces own presentation and user interaction through their `TurnUi` implementations.
- Core remains authoritative for Agent, Session, Turn, cancellation, requests, events, and outcomes.
- `bridges/native_ffi` is not a synonym or placeholder for this crate. Create it only when a real
  cross-language same-process consumer needs an ABI/serialization boundary.
- A separately running desktop companion may require HTTP/WebSocket or a real IPC Bridge; do not put
  process transport into this crate.
