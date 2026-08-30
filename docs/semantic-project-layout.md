# Semantic Project Layout

## Runtime model

Timem's architectural direction is:

```text
Interface ↔ Bridge ↔ Core
```

- **Interface** owns presentation and human interaction.
- **Bridge** owns communication only: in-process calls, HTTP/WebSocket, or IPC.
- **Core** owns reusable agent/application semantics, UI contracts, and platform policy.

A Bridge may serialize, route, reconnect, sequence, or transport semantic data. It must not
invent Agent, Session, Turn, cancellation, or approval behavior. An in-process host may use a
zero-cost direct-call bridge; the architecture does not require network machinery where none is
needed.

## First-stage physical layout

```text
interfaces/
  shell/                 # Rust terminal Interface; package remains timem_shell
  web/                   # React/assistant-ui browser Interface
core/
  platform/              # OS policy crate: timem_platform
    src/api.rs
    src/shared.rs        # Unix primitives shared by macOS and Linux
    src/macos.rs
    src/linux.rs
agent_core/              # Existing Core agent/application/UI-contract implementation
host_projection/         # Existing asynchronous projection/transport support
timem_web/               # Existing Web host and HTTP/WebSocket Bridge
```

This stage deliberately moves only axes with clear ownership. `agent_core`, `host_projection`,
and `timem_web` are not mechanically renamed or split in the same change; doing so would mix
semantic decomposition with transport and state-authority changes.

Windows is intentionally absent from this stage. Adding it requires an explicit platform design,
CI target, and behavior matrix rather than an empty placeholder directory.

## Dependency rules

1. Interfaces depend inward on Core contracts; Core never depends on an Interface.
2. Bridges depend on Core contracts and expose them to Interfaces without redefining semantics.
3. `agent_core` consumes `timem_platform`; platform code never depends on agent or UI crates.
4. OS selection is centralized in `core/platform`; business modules do not duplicate process-group
   primitives or macOS/Linux policy.
5. Package and binary names remain stable during physical moves unless a separate compatibility
   decision explicitly changes them.
6. Tests move with the owned behavior and stay outside production `src` trees.

## Enforced guard

`scripts/architecture_guard.py` checks the physical layout, dependency direction, compatibility
names, target-gated platform modules, absence of legacy/Windows directories, and escaped Unix
process primitives. Its `--self-test` mode creates valid temporary fixtures and injects each major
violation to prove the guard rejects real regressions.
