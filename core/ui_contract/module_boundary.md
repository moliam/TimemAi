# Core UI Contract Boundary

`core/ui_contract` owns UI-neutral data exchanged between Core, Bridges, and Interfaces.
It contains no presentation, transport, persistence, model execution, or application orchestration.

## Current layout

- `src/projections`: authoritative UI-readable state shapes. The initial extraction owns the Turn
  projection wire contract.

`commands` and `events` are added only when real types are extracted; empty architecture
placeholders are forbidden.

## Rules

- This crate may depend only on data/serialization libraries needed by its public contracts.
- It must not depend on Agent, application, platform, Bridge, or Interface crates.
- Contract serialization is stable and protected by tests under `core/ui_contract/tests`.
- Reducers, token allocation, lifecycle transitions, and use-case policy do not belong here.
- Rust package moves must preserve cross-language wire names unless a versioned protocol change is
  documented and tested.
