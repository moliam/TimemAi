# Core Session Boundary

`core/session` owns Session/Context/Worker lifecycle, scheduling, and use-cases.
It coordinates Agent execution without moving model, capability, prompt, or tool
semantics out of `core/agent`.

## Current layout

- `src/lib.rs`: per-context worker runtime, handles, events, and multi-worker manager.
- `tests/unit/session_worker_tests.rs`: private white-box behavioral coverage for the worker runtime.

## Dependency direction

- This crate may depend inward on `agent_core`, `timem_ui_contract`, and platform contracts.
- `agent_core` and `timem_ui_contract` must not depend on this crate.
- Bridges and Interfaces may use this crate to coordinate Sessions, but transport and presentation
  policy do not belong here.

## Behavioral compatibility

The extraction from `core/agent` is an ownership move. Worker command ordering, cancellation,
shutdown, status, event, ToolGen, and Agent execution behavior must remain unchanged.
