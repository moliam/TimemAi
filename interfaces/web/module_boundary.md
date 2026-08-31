# Web Interface Boundary

`interfaces/web` is Timem's browser Interface. It uses assistant-ui primitives for the
conversation surface and renders HTTP/WebSocket Bridge projections. It is
one presentation implementation of the same UI-neutral Core Turn semantics used
by Shell, iOS, desktop, and future clients; browser framework choices must not
become Agent lifecycle rules.

Before changing this module, read `docs/turn-state-projection-architecture.md` for the shared Core, Bridge, Interface, and lifecycle boundary.

It may contain:

- React/assistant-ui components, Markdown and syntax highlighting, responsive
  layout, accessibility, themes, animation, and browser-local preferences.
- Session selection and rename controls, composer behavior, file-picker UI,
  session-scoped inline decision queues, activity rendering, completion telemetry, and context
  compaction presentation. Destructive MEM-switch confirmation is isolated in
  `src/mem_switch_confirm_dialog.tsx`: it renders the Host boundary and emits user intent only;
  Session inspection, pending state, command delivery, and failure handling remain in the parent
  composition.
- A right-side Worker Role library shared across Sessions in the active memory
  space. The browser may select Roles per outgoing Session message, create and
  rename groups, and use dnd-kit for keyboard/pointer-accessible ordering and
  cross-group movement. The Host remains authoritative for persisted library
  state.
- MCP server management presentation: transport-specific forms, connection and
  tool-count status, per-Session enable switches, reconnect/edit/delete
  controls, responsive layout, and redacted secret placeholders.
- Bounded client history and revision-aware projection storage for WebSocket
  data, plus progressive DOM mounting and UI-owned scroll anchoring for long
  conversations. Agent lifecycle state is rendered from the authoritative Host
  projection supplied by the Web Pod assembled in `applications/timem`, whose lifecycle fields come directly
  from Core. Browser reducers must not infer Session/Turn working,
  input-admission, cancellation, or terminal state from core topics, worker
  activity, command ACK order, or visible final-answer timing.
- Frame-budgeted, order-preserving inbound event batching; memoized turn
  subtrees; and browser layout/paint containment for completed offscreen turns.
  These presentation optimizations must not drop or reorder semantic events.
- Live one-shot browser command delivery. The UI may assign a correlation
  `command_id`, but sends only while the WebSocket is open and the initial Host
  snapshot is ready. It does not persist an outbox, replay commands after
  reconnect/refresh, or treat `accepted` ACKs as business success. If a command
  never reaches Host, it did not happen and the user may explicitly try again.
  Host projections/events remain the only source of visible business changes.
- Per-tab semantic event cursors and strict sequenced delivery. In cursor mode,
  authoritative state is reduced only from `semantic_event` envelopes; raw
  legacy duplicates are ignored. The cursor advances only after the reducer
  applies the event, and a gap forces replay instead of speculative skipping.
  Delivery cursors and projection caches are bounded and tab-local; command
  payloads, API keys, and MCP secrets must not be persisted for replay.

It must not contain:

- A second Agent lifecycle state machine, a persistent/cross-reconnect command
  outbox, or visible per-command Sending/Waiting/Retrying business state.
- Browser-specific semantics that a Swift, desktop, or terminal UI would need to
  copy. Shared Turn behavior must be added to Core; shared asynchronous delivery
  behavior belongs in the HTTP/WebSocket Bridge; only visual and browser-local
  interaction behavior belongs here.
- Direct lifecycle decisions from `turn_started`, `turn_finished`, `core_topic`,
  or `worker_activity`; those events may populate a timeline only after Pod/Core
  has assigned them to an authoritative Turn projection.
- Model service/model networking, prompt or response-protocol parsing, memory/tool
  execution, command approval policy, or audit persistence.
- Reinterpretation of core topic semantics from unstructured strings when a
  shared structured field exists.
- The upstream assistant-ui monorepo as committed source. The ignored vendor
  checkout is only a pinned development reference.

The browser may understand every public topic field and choose its own visual
representation. It must not merge events from different session or request ids.
