# In-Process Bridge Boundary

`bridges/in_process` owns zero-transport communication between an Interface and Core when both
live in the same process. It provides direct function and callback adaptation without requiring
serialization, networking, reconnect state, or a separate process.

## Current layout

- `src/lib.rs`: synchronous Turn entry points that forward typed inputs, callbacks, and results.
- `tests/turn_bridge_tests.rs`: deterministic proof that Turn projections, topics, and outcomes pass
  through the Bridge without reinterpretation.

## Rules

- This crate may depend inward on Core crates.
- It must not depend on a terminal, Web, native UI, or host implementation.
- It must not parse model responses, execute tools, invent lifecycle state, render presentation, or
  add transport-only identity and replay semantics.
- Interfaces own presentation and user interaction through their `TurnUi` implementations.
- Core remains authoritative for Agent, Turn, cancellation, requests, events, and outcomes.
