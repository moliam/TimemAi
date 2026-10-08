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
- Web UI localization in `src/i18n`: browser-local language preference
  (localStorage, cross-tab sync), typed zh/en catalogs with compile-time key
  parity, and the `t()`/`useT()` entry. Catalogs cover Interface chrome text
  only; Host projections, model output, and user data are never translated.
  User-visible inline strings and CSS `content` literals outside the catalog
  are defects (guarded by `tests/i18n_source_guard.test.ts`); see
  `docs/web-i18n-architecture.md`.
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
- System settings presentation. The first-level System category may expose Host-owned, MEM-persisted configuration such as the exact per-tool model-visible result budget, plus stable browser presentation preferences such as showing answers while they are generated. Experimental switches remain grouped in a distinct Beta subsection. The browser sends intent and renders authoritative snapshot/events; it must not independently enforce Core prompt-size policy.
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
- Frame-budgeted, order-preserving inbound event batching with a cancellable
  timer fallback while events are pending, so suspended animation frames do not
  stall Host snapshots or paged replies; memoized turn
  subtrees; and browser layout/paint containment for completed offscreen turns.
  These presentation optimizations must not drop or reorder semantic events.
- Portaled final-answer outline geometry coalesces resize invalidations by frame
  with a cancellable pending-only timer fallback. A collapsed sibling must not
  leave an old absolute outline position extending the scrollable area when
  display frames stop; ordinary scroll navigation remains frame-only.
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


Endpoint sharing is isolated in `src/endpoint_share.tsx`: a transient top-level
Portal dialog above Settings owns category checkboxes, opaque share content,
Lucide warning/progress icons, copy/paste, and local result presentation. Exported
content is a copy-only, focusable, automatically wrapping code output; import uses
an uncontrolled bounded textarea so large pasted payloads do not enter React state
on every keystroke. The modal uses opaque surfaces rather than live backdrop
sampling and is memoized behind stable parent callbacks. Basic is selected by
default; advanced and personal are opt-in. Host commands perform
encoding, validation, collision resolution and persistence. The browser must
not export redacted snapshots as original configurations, interpret ACK as
import success, retain share strings across dialog unmount/MEM switch/disconnect,
or put them in replay storage. Success, rejection, timeout and disconnect share
one correlated completion path; closing/cancelling clears that correlation and
drops late replies. Unknown Host errors use a safe localized fallback rather
than exposing internal codes or paths.


Built-in tool activity rows may derive visual summaries only from structured
Host `core.action.input`: `src/tool_presentation.ts` validates known shapes and
falls back to the generic redacted argument string for malformed, unknown or
third-party inputs. `readfile` uses `SquareText` with path and selector-aware
line/byte/match summaries; historical `max_bytes` is validated only as a legacy
input field and omitted from the primary summary while remaining in expanded
redacted details. `memmgr` search/SQL uses `DatabaseSearch`, while its
other operations use `Database`; `self_tool` uses `Info` with a schema-validated
action summary. A recognized structured summary replaces raw parameters only in
the primary chat row: the expandable disclosure retains the complete redacted
argument detail supplied by the Host projection. These are Interface affordances
only and must not reinterpret action status, success, persistence or capability
semantics. Ordinary and stream presentations share the same identity and summary. Tool
surfaces use the locally bundled IBM Plex Mono face, are 90% of the prose width
on wider viewports (full width on narrow screens), and keep elapsed metadata at
the right edge. Running rows alone show a muted leading execution dot; successful
rows omit both the marker and its layout slot so tool identity shifts left and
returns to the stronger settled color, while failures retain an explicit status.
Rows with details are the disclosure target themselves, with hover/focus feedback,
keyboard activation and selection protection rather than a persistent arrow.
Live elapsed and countdown values use fixed-width, tabular digit cells: only
changed digits perform a short clipped vertical roll, labels and units remain
stationary, and outgoing/incoming glyphs never cross-fade in the same pixels.
Reduced-motion presentation updates without animation. Settled duration facts
remain static. Local-work state uses a reduced-motion-aware swaying wrench;
model waiting keeps its existing star identity. System notices use their own half-pixel-larger type
contract. An in-progress context-compression notice is one concise status line
(“Conversation compressing...” / “对话压缩中...”), without a duplicate category
title and detail; completed notices retain their compression metrics.
