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

A Bridge is a logical communication boundary, not a process boundary. Shell and native clients may
use an in-process bridge without serialization or networking. Browser clients use HTTP/WebSocket.
A separately running desktop companion uses IPC. No Bridge may invent Agent, Session, Turn,
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
  http_websocket/           # browser host, HTTP/WebSocket and reconnect delivery
  ipc/                      # desktop companion process transport
interfaces/
  shell/
  web/
  macos/
  windows/
  linux/
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

The repository is deliberately migrated in compiling, reviewable steps. These are the only current
transitional roots:

| Current owner | Target owner | Removal condition |
| --- | --- | --- |
| `host_projection/` | `bridges/http_websocket/` | Projection delivery is integrated without creating a second Turn state machine. |
| `timem_web/` | `bridges/http_websocket/` | The `timem-web` binary, HTTP/WebSocket behavior, assets, and lifecycle tests pass at the target path. |

`core/agent`, `core/session`, `core/ui_contract`, `interfaces/shell`, `interfaces/web`, and
`core/platform` are already at their semantic roots, but some internal code still needs finer
boundary cleanup.
`benchmarks/` and `examples/` remain cross-cutting verification/support material until their final ownership is decided; they are not
runtime layers.

Windows and native desktop Interfaces are target architecture, not current support claims. They
must gain an explicit behavior matrix and executable unsupported/supported contract before their
paths are created.

## Incremental sequence and exit gates

Each step is committed separately and must leave the workspace buildable:

1. **Architecture contract**: record this target, migration inventory, no-placeholder rule, and
   executable guard checks.
2. **UI contract extraction**: create `core/ui_contract` from genuinely UI-neutral commands,
   events, and projections. Keep temporary `agent_core` re-exports where they do not reverse the
   target dependency direction.
3. **Agent relocation (complete)**: the `agent_core` package lives at `core/agent`; resource paths,
   tests, scripts, docs, and workspace references preserve behavior and compatibility.
4. **Session extraction (complete)**: `timem_session` owns Session/Context/Worker lifecycle,
   scheduling, and management; Web callers use the new owner and Agent has no reverse dependency.
5. **In-process Bridge**: move Shell/native direct-call and callback/channel adaptation to
   `bridges/in_process`; presentation remains in `interfaces/shell`.
6. **HTTP/WebSocket Bridge**: combine the Web host and asynchronous projection/delivery ownership
   under `bridges/http_websocket`, preserving package/binary and wire behavior.
7. **IPC and native Interfaces**: add the IPC contract and native clients only with real behavior,
   tests, and explicit platform support status.
8. **Final cleanup**: remove transitional roots and compatibility shims; update all docs, guards,
   release paths, and full CI evidence.

For every step, run the architecture guard, module/test contract checks, formatting, relevant crate
and Interface tests, and `git diff --check`. Run the full `scripts/ci.sh` at behavior-sensitive
milestones and before declaring the migration complete.

## Non-regression invariants

- Core remains authoritative for Session, Context, Worker, Turn, input admission, cancellation,
  terminal outcomes, and structured host decisions.
- Shell/native in-process paths do not acquire network, serialization, or reconnect overhead merely
  to satisfy the Bridge abstraction.
- Web keeps bounded command delivery, event ordering, reconnect baselines, authentication, and
  backpressure behavior.
- Public semantic types are not replaced with generic text envelopes.
- Tests move with behavior and remain outside production `src` except minimal external test hooks.
- No step claims macOS, Windows, or Linux support from names alone.
