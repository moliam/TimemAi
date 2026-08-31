# Timem Application Boundary

`applications/timem` is the product composition root. Its Cargo package and binary are both
named `timem`; no `timem_web` Cargo package or physical top-level `timem_web/` root exists.
The application assembles Core, Bridges, and Interfaces; its current binary combines
local-first Web hosting with Shell entry points. It binds a loopback HTTP/WebSocket
server by default, allows an explicit authenticated `--public` bind, owns
browser authentication and bounded live command delivery, maps browser commands to
Session/Core worker handles, and projects the UI-neutral authoritative
Core Turn contract for a reconnectable browser. Its Pod/projection layer is not
a second agent runtime and is not required by synchronous or in-process Interfaces.

Before changing this module, read `docs/turn-state-projection-architecture.md` for the shared Core, Bridge, Interface, and lifecycle boundary.

It may contain:

The product Web host is split by internal responsibility under `src/server/`:

- `command_lane.rs` owns the process-local FIFO ticket primitive used to serialize accepted
  mutations per scope. It has no Session/Turn semantics and advances tickets through an RAII guard.
- `command_dedup.rs` owns bounded process-local command correlation records, including in-flight
  retention, terminal-record eviction, capacity exhaustion, and explicit reservation removal. It is
  not a durable command ledger and does not infer Session/Turn lifecycle.
- `websocket_delivery.rs` owns authenticated browser command delivery after generic WebSocket
  framing.
- `mem_maintenance.rs` owns bounded memory-space maintenance.
- `server.rs` remains the parent composition module while further behavior-preserving extraction is
  performed incrementally. New code should enter the narrowest existing submodule rather than
  rebuilding these responsibilities in the parent.

- Product HTTP lifecycle, local port selection, explicit public-bind policy, per-process access
  tokens, authenticated product handlers, snapshot construction, and mapping decoded browser
  commands to Session/Core operations. Fixed HTTP/WebSocket paths and method placement, request
  bounds, static fallback routing, generic WebSocket framing, bounded JSON wire I/O, same-origin
  validation, and browser-safe transport headers live in `bridges/http_websocket`. The Application
  injects handlers and state into that route composition; it must not recreate the route table.
- Target-specific host-process lifecycle adapters under `src/os/`: Unix owns
  SIGINT/SIGTERM/SIGHUP streams and parent-shell detection; Windows owns
  Ctrl+C monitoring and PID-reuse-safe launcher-process detection through the
  Core Platform process APIs. Shared Web lifecycle semantics remain in
  `server.rs`.
- Platform-backed secure random generation and diagnostic file leases. Web owns
  access-token and single-instance policy, while `core/platform` owns OS random,
  permissions, sharing, and lock primitives.
- Session worker orchestration and browser-facing snapshots.
- HTTP/WebSocket Bridge projection delivery that converts Core projection changes into
  revisioned, self-sufficient browser snapshots/updates while preserving Core
  Turn identity, input admission, activity, and immutable outcome exactly. The
  Bridge may add transport metadata but may not reinterpret lifecycle.
- Web-specific one-shot command correlation, bounded process-local command
  deduplication, bounded NextTurnIntent FIFO, projection revision, event
  sequence, reconnect baseline, and MEM barrier.
  These are reusable adapter patterns for future asynchronous Interfaces, but they
  are not Core business state and must not be required by a direct Shell/native
  binding.
- Per-session runtime-profile collection and safe projection. The host copies
  global defaults when a Session is created, keeps secrets server-side, and
  gives every context/worker belonging to that Session the same profile. The
  effective allowlisted runtime environment is cached in the core Session
  store and updated immediately after supported runtime configuration changes;
  explicit launch options retain precedence. Browser snapshots and topics must
  use the redacted projection and never include the cached API key. The host
  does not reinterpret model API or response protocol semantics.
- Model service-incomplete startup for browser configurability. The host may hold an
  empty API key while serving the UI and restoring history, but it must validate
  the selected Session before creating a user turn or ToolGen turn. This Web
  draft state must not weaken Shell startup or model service-call validation.
- Static asset serving and browser transport backpressure/reconnect behavior.
  Browser commands enter a bounded, ordered queue and execute on the blocking
  pool; filesystem/session work must never stop the WebSocket loop from
  forwarding core topics. Queue overflow is rejected explicitly rather than
  growing memory without bound.
- Bounded live mutation handling. Browser mutations may carry a correlation
  `command_id`; the Host keeps only a fixed-capacity process-local dedup cache
  and returns correlated `accepted`, `committed`, or `rejected`
  acknowledgements on the live connection. Terminal records may be evicted;
  if every slot is still accepted, new IDs are explicitly rejected rather than
  growing memory. The Host does not create a per-command ledger or
  `web_command_dedup.json`. Same-Session mutations are FIFO across sockets,
  independent Sessions may execute in parallel, and global mutations exclude
  Session mutations through the global barrier.
- Ordered semantic event delivery. After authoritative state is persisted,
  mutations and Core topics enter one in-memory linearization point that assigns
  `event_seq` and broadcasts the envelope without filesystem I/O. WebSocket
  handlers subscribe before taking a snapshot; every `hello` establishes a new
  snapshot baseline. A sequence gap or broadcast lag reloads a full snapshot
  instead of replaying disk history. Request-scoped queries, acknowledgements,
  validation errors, and secret reveals remain direct. Memory-space switching
  changes Session state and the per-MEM Web instance lease under one epoch
  barrier, resets the bounded process-local dedup cache, and prevents old
  accepted work from executing in the new space.
- Per-session browser upload storage and attachment metadata. Uploaded bytes
  remain host-local; the host only contributes their paths as session context.
- Memory-space-scoped Worker Role library ownership. Roles and Role groups are
  shared by every Session in the active memory space, persisted atomically
  outside individual Session directories, and projected into snapshots and
  Session views. Role/group mutations and ordering are global mutations.
  During restore, legacy per-Session `worker_roles.json` arrays are merged by
  case-insensitive name; distinct ID collisions receive new IDs. Turn history
  continues to store immutable Role snapshots so past prompts remain readable.
- Host-only settings and UI command validation.
- MCP configuration projection and Session enablement routing. Server
  definitions are persisted in the active mem, secrets remain server-side and
  are redacted in browser snapshots, and each Session carries its own enabled
  server-id set. UI changes advance a Session-local desired revision. Worker
  creation, Session restore, and new-turn submission must not wait on external
  MCP I/O: they apply only an exact connected tool cache and schedule missing
  discovery on a deduplicated background task. Successful discovery advances a
  new desired revision for the following new-turn boundary; failure updates
  connection status without blocking agent work. The host must not mutate a
  running worker capability set, parse MCP arguments, or execute MCP tool calls
  itself.
- Session-scoped ToolGen commands and ToolRepo projections. The Web host may
  start ToolGen manually for an exact completed turn, attach optional user
  guidance, list/search/detail/rename tools, and request opening a validated
  tool directory in a terminal. It must route these operations by Session and
  forward `core.toolgen` lifecycle data without moving repository validation or
  retrospective model logic out of `agent_core`.

A Web Session is the configuration ownership boundary and contains explicit
`contexts[]` and `workers[]` registries. All workers in the Session inherit its
server-side environment/profile. A Context owns prompt/workspace state such as
cwd and references its workers. A Worker belongs to exactly one Session and one
Context, may reference a parent worker, and owns one core execution loop. The
current Web UI creates one default Context and primary Worker, while the host
creation path can attach child workers to new contexts without moving profile
ownership down to a worker. A different Session may use a different profile.
One Context currently has exactly one worker; subtask concurrency is created as
a new Context plus a new Worker so mutable prompt state cannot fork silently.
Session credentials are server-owned. Web may replace or clear the API key for
an idle Session, persist it in the owner-protected core Session index, and
update every worker before the next turn. Browser snapshots and broadcast
topics expose only `api_key_configured`; they must never serialize the key.
Credential mutation during an active turn is rejected.
An authenticated browser may explicitly reveal a Session API key or sensitive
MCP map values for editing. Such plaintext is a direct reply to the requesting
WebSocket only: it must not enter broadcast events, snapshots, prompt context,
history, activity, or audit. The browser must discard the reply after closing
the editor, changing Session/mem, reconnecting, or saving.

Core topic routing is keyed by the cross-language scope tuple
`session_id/context_id/worker_id` plus the authoritative Core `TurnToken`.
Session-level commands currently target the primary worker. A child worker
finishing must not finish the primary chat turn. Session lifecycle state comes
from the Core Turn projection; the Host must not derive it from worker counts,
topic arrival order, or whichever worker event arrived last.

Only the primary worker has a user-facing chat channel. Child-worker free talk,
actions, and requests are rendered inside the primary Session turn. For a
request that needs a user decision, `worker_id` is a private routing return
address: the host records the approval in the primary chat flow and relays the
structured `TopicReply` to the requesting child worker. It must not create a
second child-worker chat surface or send every reply to the primary worker.
Creating a child worker is an internal implementation choice for a subtask; it
must not emit `session_created`, add a sidebar Session, or otherwise ask the
user to manage runtime scheduling topology.

Task cancellation is Session-wide: the host forwards the user's Stop action to
every current worker so internal subtasks cannot outlive the cancelled primary
task. A later user turn is submitted only to the primary worker; old child
workers are not resumed or broadcast the new input.

Ordinary Send during an Active Turn is always a separate next-turn intent. Only
an explicit supplement command may target the current `TurnToken`. Core may
accept it into pending input, but only a sealed, already-sent `PromptCut` proves
that the current Turn consumed it. On terminal commit, the Pod atomically takes
over every accepted-but-unconsumed task command, including its attachments, as
a next-turn intent under the same command ownership rather than dropping it,
asking the browser to issue a second command, or appending it after the final
answer. Per Session, these intents form a bounded FIFO ordered by Host `enqueue_seq`
and deduplicated by `command_id`; only its head may enter Core. The Host
atomically persists the Session-owned FIFO. A Host/Core process restart is a
hard execution boundary: queued items and the user's auto-send preference remain
visible, but any in-flight dispatch reservation and continuation grant are
cleared. Startup must not automatically redrive a queued item or call Core from
an old interrupted Turn record; later execution requires a new authoritative
continuation grant or an explicit queue command. Each dispatched intent receives
a fresh token and starts at model round one. Decision replies,
ToolGen guidance, and settings mutations retain their own typed commands and
must never be silently converted as late supplements.

It must not contain:

- A second Turn lifecycle state machine. The Host/Pod may manage command delivery,
  MEM barriers, projection revisions, snapshots, and timeline assembly, but only
  Core may create/stop/finish the authoritative Active Turn. Transport caches,
  command queues, event channels, and pending ownership collections must remain
  hard-bounded; exhaustion is an explicit error, never silent growth.
- Web-only lifecycle semantics that another host would have to duplicate. If a
  rule decides whether a Turn exists, accepts input, stops, finishes, or owns an
  outcome, it belongs in Core. This module only adapts that rule to reliable Web
  delivery and browser-facing projection.
- Model API wire formatting or HTTP execution, prompt assembly, memory semantics,
  tool argument parsing, MCP protocol execution, or other tool execution.
- React layout, CSS, browser state reducers, or user-facing visual policy. Those
  belong in `interfaces/web`.
- Natural-language reinterpretation of core topics. UI receives semantic topic
  payloads and decides presentation.


## Dependency direction

- This application may depend inward on `core/*`, `bridges/*`, and `interfaces/*` to assemble a product.
- Core, Bridges, and Interfaces must never depend back on `applications/timem`.
- Product wiring belongs here; reusable synchronous Rust access belongs in
  `bridges/in_process`, and reconnectable HTTP/WebSocket transport belongs in
  `bridges/http_websocket`.
- Do not recreate a `timem_web` Cargo package or top-level `timem_web/` directory. Command
  compatibility, when required, belongs only in installer-created `timem-web` links or shims.
- Do not create placeholder desktop or FFI modules. Add `interfaces/desktop`,
  `applications/timem_desktop`, `bridges/native_ffi`, or `bridges/ipc` only with a real consumer and
  implemented behavior. A same-process Rust desktop Interface should use `bridges/in_process`; a
  cross-language same-process client may justify `native_ffi`; a separate process may justify IPC.
