# TimemAi Development Constitution

This file is the repository-level development contract. Every contributor and
coding agent must read it before changing code. It records durable principles;
detailed designs and temporary implementation plans belong under `docs/`.

## 1. Product and delivery principles

- Solve the user's real requirement, not merely the literal symptom. Confirm the
  intended behavior, ownership boundary, compatibility needs, and failure modes
  before making a broad change.
- Prefer the smallest coherent change that fixes the root cause. Do not mix
  unrelated architecture axes, speculative abstractions, or opportunistic
  rewrites into one delivery.
- Source, tests, architecture guards, documentation, generated assets, and
  release scripts are one product. Update all affected surfaces together.
- Never claim success from code inspection alone. Base conclusions on the
  corresponding executable checks and report any check that was not run.
- Do not push, publish, tag, or alter remote history unless the user explicitly
  asks. Local implementation and release-quality validation come first.

## 2. Semantic architecture

The intended direction is:

```text
Interface ↔ Bridge ↔ Core
```

- **Interface** owns presentation and human interaction: terminal/browser/native
  input, rendering, layout, accessibility, and UI-only convenience.
- **Bridge** owns communication only: in-process calls, HTTP/WebSocket, IPC,
  serialization, routing, sequencing, replay, and reconnect behavior.
- **Core** owns reusable Agent and Session semantics, authoritative state,
  UI-neutral contracts, capability execution, persistence rules, and platform
  policy.
- A Bridge must not invent or reinterpret Agent, Session, Turn, cancellation,
  approval, retry, or lifecycle semantics. An Interface must render structured
  Core meaning rather than reconstructing state from strings or event timing.
- Dependencies point inward. Core never depends on an Interface. Platform code
  never depends on agent, Bridge, or UI crates. Avoid cycles and hidden global
  coupling.
- Preserve semantic types. Do not collapse final answers, progress, action
  intent, evidence, diagnostics, status metadata, and lifecycle outcomes into a
  generic `text` field when their meanings differ.
- Core may return strings when the string itself is data. Core must not contain
  terminal ANSI styling, browser layout, localized UI composition, or
  host-specific presentation policy.

Read `docs/semantic-project-layout.md` for the physical layout and migration
scope. Run `python3 scripts/architecture_guard.py` after architecture or
workspace changes.

## 3. Physical boundaries and migration

The current physical ownership is:

- `core/{agent,session,ui_contract,platform}`
- `bridges/{in_process,http_websocket}`
- `interfaces/{shell,web}`
- `applications/timem` as the unified product composition root
- `resources`, `tests`, `docs`, and `scripts` for shared non-runtime assets and verification.

A Bridge is a logical communication boundary, not necessarily a process boundary. Every
same-process Rust Interface uses direct typed calls and callbacks through `bridges/in_process`;
the browser uses HTTP/WebSocket delivery. Do not force serialization or network I/O onto an
in-process path.

The physical runtime-root migration is complete. The former top-level `timem_web/` root has moved
to `applications/timem/`; projection delivery state lives in `bridges/http_websocket`; semantic
projection types live in `core/ui_contract`; and the package/crate named `agent_core` lives at
`core/agent`. The Cargo package names `timem_web`, `timem_shell`, and `agent_core` remain compatibility
identities, not physical directory names or permission to bypass the dependency graph. The CLI
delivery surface is one real executable named `timem`: it launches Web by default and Shell only
with `--shell`. A `timem-web` symlink or forwarding shim may be installed for command compatibility,
but must not be a second executable or delegated runtime.

Do not restore legacy roots `timem_web`, `timem_shell`, `web_ui`, `agent_core`, or
`core/agent/src/os`. Do not create empty target directories. Add `bridges/native_ffi`,
`bridges/ipc`, `interfaces/desktop`, or another application root only with a real consumer,
implemented behavior, an explicit support matrix, and executable tests. A same-process Rust desktop
Interface should reuse `bridges/in_process`; cross-language same-process access may justify
`native_ffi`; a separate companion process may justify HTTP/WebSocket or IPC.

Module-local rules in `*/module_boundary.md` remain mandatory. If a local rule conflicts with this
file, this repository-level contract wins and both documents must be reconciled in the same change.

## 4. Authoritative state and protocol rules

- Core is authoritative for Agent, Session, Context, Worker, Turn, input
  admission, cancellation, terminal outcomes, and structured Interface decisions.
- UI and delivery layers may cache, project, or transport Core state, but must
  not create a competing state machine from topic order, worker counts,
  acknowledgements, visible output timing, or localized strings.
- Prompt and model response formats are protocols. When changing them, update
  producer, parser, validation, repair behavior, fixtures, examples, and tests
  together. Do not leave accidental dual protocols.
- Runtime code must not infer natural-language intent with hard-coded keyword
  branches. Expose structured evidence and capabilities; let the model reason
  where semantic judgment is required.
- Built-in tools remain paired packages under
  `resources/capabilities/tools/{tool}.yaml` and `{tool}.rs`. The manifest is
  the model/executor interface and validation source; concrete parsing and
  execution belong to the tool implementation, not the top-level turn loop.
- Topic callbacks are synchronous and Core-owned during the call. A host that
  retains data for async rendering, transport, tests, or logs must clone the
  required fields before returning.

## 5. Coding quality requirements

- Keep modules cohesive, names semantic, dependencies explicit, and public APIs
  minimal. Prefer composition and narrow interfaces over cross-layer helpers.
- Preserve safety properties and fail closed for ownership, destructive actions,
  process identity, permissions, and unsupported-platform decisions.
- No silent error swallowing. Return or record actionable, redacted context;
  distinguish invalid input, unavailable capability, transient failure,
  cancellation, and internal defects.
- Avoid unnecessary cloning, full-file reads, unbounded collections, blocking in
  async paths, polling without bounds, quadratic rendering, and repeated prompt
  or projection reconstruction.
- Compatibility is an explicit constraint. Physical moves must preserve package,
  binary, CLI, data schema, protocol, and generated-asset behavior unless a
  separately documented decision changes them.
- Do not add dead compatibility branches, empty architecture placeholders,
  speculative traits, or abstractions with only one unclear responsibility.
- Format and lint changed code. Keep the repository free of warnings, debug
  leftovers, temporary files, generated dependency directories, and unrelated
  formatting churn.
- Never place real user paths, keys, private facts, internal URLs, conversations,
  or secrets in source, fixtures, logs, snapshots, docs, or commits.

## 6. Test requirements

A feature is not protected by one happy-path helper test. Review both quality
axes and all applicable coverage dimensions from `docs/test-strategy.md` and
record feature coverage in `docs/feature-test-management.md`.

### Quality axes

1. **Core interaction correctness**: protocol, state transition, execution,
   persistence, cancellation, retry, audit, and multi-round behavior.
2. **Interface display correctness**: Shell/Web accurately render the same
   structured semantics without cross-session leakage or lifecycle invention.

Behavior crossing both axes requires tests on both sides.

### Coverage dimensions

For every changed behavior, cover or explicitly justify the absence of:

1. Normal path.
2. Boundary path: empty, maximum, long, narrow, threshold, and unusual values.
3. Error/cancellation path: malformed input, permission failure, unavailable
   service, stale identity, retry, and interruption where relevant.
4. Stress/repetition/concurrency path for race-prone, stateful, or hot behavior.

### Test placement and realism

- Test functions and fixture corpora belong under each crate's `tests` directory.
  Production `src` files may contain only a minimal external test-module hook or
  a narrowly scoped test-only access point for private white-box coverage.
- Prefer tests through real public boundaries. Use real temporary files,
  subprocesses, fake model servers, HTTP/WebSocket flows, pseudo-TTYs, and built
  frontend assets where those are part of production behavior.
- Every bug fix needs a regression test that fails for the original defect.
- Architecture guards need negative self-tests that inject violations and prove
  the guard rejects them; checking only the current valid tree is insufficient.
- Tests must be deterministic, isolated, bounded in time/resources, and clean up
  their files and child processes even on failure.

## 7. Performance and zero-regression policy

- Architecture improvement is not permission for functional, performance,
  efficiency, UX, compatibility, or security regression.
- Establish evidence before changing a hot path and compare the same workload
  afterward. Do not infer causality from execution order or unrelated metrics.
- Preserve bounded memory, storage, event queues, rendered rows, output evidence,
  retries, timeouts, and process lifecycles.
- Run `scripts/performance_guard.sh` for changes touching prompt assembly, event
  fan-out, rendering, projection, long histories, caching, process supervision,
  or other measured hot paths.
- Release binaries and embedded Web assets must still build reproducibly. A
  frontend source change must regenerate the tracked `interfaces/web/dist` and
  leave no unexplained bundle diff.

## 8. Documentation requirements

- Update README and the relevant architecture, protocol, test matrix, install,
  or release document in the same change as behavior or layout changes.
- Keep README concise; detailed rationale, diagrams, matrices, and migration
  plans belong under `docs/`.
- Documentation must describe current truth. Remove stale paths and commands
  after moves; do not preserve misleading historical instructions outside
  clearly marked historical audit documents.
- State assumptions and unsupported targets explicitly. Do not imply support
  from a directory name, placeholder module, or compile-only stub.

## 9. Required workflow and quality gates

Before editing:

1. Read this file and affected `module_boundary.md` files.
2. Inspect current behavior, callers, tests, scripts, and relevant history.
3. Define invariants, compatibility constraints, risks, and a test plan.

While editing:

1. Keep changes in the owning semantic layer.
2. Add/update executable tests and guards with the implementation.
3. Run narrow tests first for fast feedback; clean temporary artifacts.

Before declaring completion, run the applicable gates. For architecture or
release-impacting work, the expected baseline is:

```bash
python3 scripts/architecture_guard.py --self-test
scripts/module_boundary_check.sh
scripts/test_contract_check.sh
cargo fmt --all -- --check
scripts/clippy_check.sh
cargo test --workspace --locked -- --test-threads=1
cargo doc --workspace --all-features --no-deps --locked
pnpm --dir interfaces/web test
pnpm --dir interfaces/web build
git diff --exit-code -- interfaces/web/dist
scripts/performance_guard.sh
cargo build --locked -p timem_web --release
git diff --check
```

Run additional platform, pseudo-TTY, browser, runtime lifecycle, security,
installation, and repeated-edge gates whenever the touched behavior requires
them. `scripts/ci.sh` is the authoritative full local/CI gate.

If a required gate cannot run, report exactly which gate, why, and the residual
risk. Never weaken, delete, or bypass a failing test merely to make delivery
appear green.
