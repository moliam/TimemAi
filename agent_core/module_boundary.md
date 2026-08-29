# agent_core module boundary

`agent_core` is the reusable, UI-neutral Timem runtime. It owns agent state,
prompt/context management, model service payload/transport, capability
registration, tool execution coordination, memory access, retry policy, and the
authoritative semantic contracts consumed by every Shell, Web, iOS, desktop, or
future host. A new UI shell should adapt these contracts, not fork the agent
loop or reconstruct Turn lifecycle from events.

Before changing this module, also read the repository-level `AGENTS.md`.
Also read `docs/turn-state-projection-architecture.md` for the shared Core, Host Adapter, Host Projection Adapter, and UI-shell lifecycle boundary.

## Belongs here

- Protocol-neutral runtime data structures and algorithms.
- The authoritative per-Session Turn Gate and minimal Turn reducer: durable
  `TurnId`, monotonic `TurnEpoch`, exact `TurnToken` validation, Active-Turn
  ownership, stop intent, immutable terminal outcome, and rejection of stale or
  late worker/model/tool events. Worker activity and topic state are subordinate
  facts and must never create or revive a Turn.
- The authoritative, UI-neutral Core Turn projection contract. Core exposes
  current Active Turn identity, input admission, activity, stop intent, immutable
  outcome, worker scope, and structured requests/replies without terminal,
  browser, Swift, React, transport, or layout assumptions. All hosts must obtain
  lifecycle truth from this contract. A host may cache or repackage the
  projection, but cannot derive a competing lifecycle from topic order, worker
  counts, or local command state.
- A stable host-adapter surface for both deployment shapes: a simple synchronous
  host may call Core and render the projection directly; an asynchronous,
  reconnectable, or remote UI may place a Host Projection Adapter in front of
  Core to add snapshots, revisions, reliable command delivery, and transport
  sequencing. Those delivery mechanisms remain outside Core and must not alter
  Core Turn semantics.
- Model request/response adapters and cache planning. Provider-facing tool
  schemas are rendered through a dedicated protocol-dialect module rather than
  mutated in the capability registry or assembled ad hoc in hosts. The executor
  always validates against the original registered schema; compatibility
  renderers may only transform the model-facing copy.
- Model API wire-request planning and transport, including endpoint, headers,
  payload shape, cache-control fields, structured-output fields, HTTP execution,
  response parsing, and audit redaction metadata. Hosts should not rebuild
  model API protocol details, execute model HTTP, or reinterpret model HTTP
  response semantics.
- Model retry policy, retry decision data, and model-call outcome accounting.
  Model service I/O belongs behind the core/model service boundary. Hosts may surface
  waiting/cancellation UX, but should not redefine retryability or retry
  metadata.
- Capability and tool registries, including validation data loaded from
  resources.
- MCP client transport, server discovery, namespaced dynamic tool registration,
  and `tools/call` execution. The response-protocol parser remains generic: it
  parses the registered action name and JSON arguments, while the MCP server is
  authoritative for its full JSON Schema validation. MCP failures are bounded
  action evidence, never response-protocol repair; failed transports are evicted
  so a late response or dead connection cannot contaminate later calls. Applying a host-requested MCP update
  compares complete tool definitions and submits one `SYSTEM` capability-change
  component only when the model-visible capability set actually changed.
- Built-in action dispatch by registered action/binding name. Core may route a
  manifest-backed builtin action to its callback through
  `resources/capabilities/tools/registry.rs`, but concrete option parsing and
  tool execution live in the paired tool implementation under
  `resources/capabilities/tools/{tool}.rs`; the paired YAML remains the source
  for prompt injection and generic input validation.
- Host capability profiling. Resource manifests describe known capabilities, but
  the active registry must be filtered by the current host/environment, such as
  whether local command execution is available.
  Capability detection is based on executable runtime affordances, not host UI
  type: a terminal host, server host, or desktop app may expose local command
  execution, while a mobile app or sandboxed host may not.
- Prompt context construction and runtime-injected context sections.
- Session prompt component assembly. Core owns the per-session pending prompt
  component buffer, `submit_prompt_component(...)`, and `build_next_prompt()`.
  Runtime/model/user/action outputs enter the next model prompt as structured
  `PromptComponent` records with role, kind, source, logical timestamp, and
  sequence. `build_next_prompt()` is the single formatting exit that drains the
  pending buffer into dynamic prompt deltas. Other modules should not hand-roll
  visible prompt text or bypass this queue when adding context for the next
  model call.
- Prompt component ordering. The pending prompt buffer is a timeline, not a
  role map. It must preserve repeated visible roles such as
  `SYSTEM -> USER -> SYSTEM -> Ai4`. Components derived from one previous LLM
  response parsing/execution batch use the same earliest logical timestamp so
  they appear before later user/runtime submissions. Logical timestamps are only
  for prompt assembly order and must not be rendered into the model prompt.
- Active-turn context updates such as explicit user supplements entered while a
  model turn is in progress, including their prompt-slice insertion and audit
  events. Core owns the Turn's one-way input-admission gate and seals a
  `PromptCut` before each model request to record exactly which input sequence
  that request consumes. Command acceptance alone does not prove that a
  supplement influenced the current model response. When Core accepts a
  terminal/final response, it keeps only input covered by an already-sent
  PromptCut in the current Turn and returns every accepted-but-unconsumed task
  input to the Host for atomic conversion into a new-turn intent. The new Turn
  starts from its first model round with a fresh `TurnToken`.
- Active-turn reminders. Core evaluates the host-loaded, user-global reminder
  schedules independently for active-time and completed-round intervals, then
  submits each due random selection as a SYSTEM component before the next model
  request. Selecting `NONE` consumes that interval without prompt injection.
  Host-decision wait time is excluded, long blocking calls are not interrupted,
  and missed time intervals collapse rather than building a stale backlog.
- Host-decision results once chosen by the UI, such as applying user approval
  decisions to pending actions and recording their runtime audit events.
- Round-limit decisions after the host chooses continue/stop, including audit
  events, round-budget recharge, and structured stop summaries.
- Output-expansion decisions after the host chooses expand/stop, including
  max-output-token updates, audit events, and output-limit stop summaries.
- Stale-context decisions after the host chooses continue/reset, including audit
  events and dynamic-context clearing.
- Model-response repair handling after a model response is applied, including
  repair issue classification, repair counter updates, repair prompt-slice
  injection, generic repair audit events, and realtime
  `audit/api_output_repair.json` diagnostics containing malformed output plus
  the RUNTIME repair message.
- Turn supporting-context assembly, including runtime identity and host-provided
  additional context. Hosts provide source values; core owns how they are
  combined for the model.
- Host/runtime identity fields such as `runtime` and `run_bash_target` as
  structured turn input. Core consumes these values when assembling model
  context; reusable runtime loops should not hard-code a terminal host identity.
- Memory, scratch, raw chat, context shrink/compact, and conflict handling.
  A successful compact re-injects one bounded RUNTIME snapshot of currently
  applied MCP actions when any are active; pending host configuration and MCP
  secrets are never included.
- Cross-host Session persistence schemas. Core owns `StoredSession`,
  `ChatHistoryRecord`, history paging, and resume-notice format so Shell, Web,
  iOS, and future hosts share one JSONL history contract. The first resume
  layer stores session metadata and raw chat/event records, not live
  Worker/Context execution state.
  The metadata includes the effective allowlisted TIMEM runtime environment so
  hosts can resume model service configuration. The local Session index may contain
  an API key and must be owner-only; secrets must never be copied into history,
  prompts, topics, snapshots, or audit records.
  Session persistence includes the MCP server ids enabled for that Session;
  server definitions and credentials remain in the active mem and are not
  embedded in chat history or topic payloads.
- Session ToolRepo persistence and ToolGen retrospective execution. Core owns
  Session-scoped draft/published paths, manifest/tree validation, bounded
  self-tests, atomic publication/update, repository search/detail/rename data,
  source-turn-bound manual requests, same-Context sequential execution, normal
  turn lifecycle, temporary capability activation, and structured
  `core.toolgen` lifecycle topics.
  ToolGen failure must not replace a successful source-turn result. Hosts own
  the manual trigger and repository presentation, not candidate validation.
- Local tool execution abstractions that return structured action evidence.
- Registered command-tool foreground/background execution semantics. Core owns
  background job ids, persisted status/output files, polling, cancellation,
  timeout handling, process termination, and action evidence for command-bound
  tool jobs. For `run_bash`, core owns the session running-pid set for
  background jobs and timed-out normal commands. `run_bash` prompt evidence
  shows the running transition once, core injects one-time job-exit updates on
  status transition, and core injects a full running-job snapshot only after
  large context shrink/compact. Hosts may render progress/status, but they must
  not own the lifecycle for model-requested jobs.
- Model-requested local tool execution, including `run_bash`, command approval
  application, process execution, command output/evidence shaping, and tool
  audit. Hosts may provide user decisions and cancellation signals, but the
  executor remains a core responsibility. Parallel actions share the owning
  turn's cancellation state; core must keep polling it while joins are pending
  and must terminate the full command process group on explicit host Stop.
- Action failure isolation. External command exits, including signal-based
  termination, are action results and must not terminate the core process.
  Builtin callback panics are contained at the tool registry boundary, reported
  as internal action failures, and audited as `internal_error`. A tool that can
  cause a native in-process fault must use process isolation rather than relying
  on panic recovery.
- Long foreground command lifecycle for positive model-provided `timeout_ms`:
  core owns process waiting, long-running decision requests, timeout transition
  into the session running-pid set, action result shaping, and user-supplement
  insertion after host/user cancellation.
- Structured reports, requests, stop reasons, status snapshots, and topic events
  for any host UI to render.
- Optional per-context worker lifecycle. Core may provide a worker that owns one
  `AgentCore`, one Session identity, one Context identity, one Worker identity,
  and one runtime loop on a dedicated thread. Multiple workers may belong to
  the same Session while operating on separate contexts. This is a host adapter convenience for multi-session/web-style
  execution; it must preserve the same topic/request semantics as the
  synchronous `run_session_turn` path.
- Multi-session worker management. Core owns the standard manager that allocates
  worker identities from `ID0`, keeps worker handles/status snapshots by
  `worker_id`,
  shares global working-worker state across workers, polls worker events, and
  shuts workers down. Hosts may choose to use the manager or manage workers
  explicitly, but they should not create incompatible identity/lifecycle rules.
- Session worker shutdown semantics. Core owns cancellation and cleanup for its
  worker threads: shutdown cancels the active turn, rejects new work, skips
  queued turn/rename commands that have not started, emits a stop event, and
  joins the thread when the worker owner shuts down or is dropped.
- Session worker identity and workspace metadata. Worker identity includes
  `session_id`, `context_id`, `worker_id`, display name, ordinal, and optional
  `parent_worker_id`. Default
  display names are `ID0`, `ID1`, ... by ordinal, but host/user/parent-agent
  code may rename a worker through core's worker handle. Workspace metadata may
  include current directory, data/audit paths, runtime, bash target, sanitized
  environment, and workspace reference directories. Do not expose full prompt
  text as lifecycle metadata; expose only context summaries.
- Context ownership is exclusive in the current runtime: one `(session_id,
  context_id)` may have only one worker because that worker owns the mutable
  `AgentCore` prompt state. A subtask worker must receive a new Context. Do not
  allow two independent `AgentCore` instances to masquerade as one shared
  Context without first introducing an explicit context coordinator.
- A unified topic event surface for core-initiated runtime output. User/host
  initiated operations enter core through functions; core-initiated progress,
  status, requests, and decisions are represented as topic events with
  `session_id`, topic metadata, session state, and structured payload.
- Core lifecycle topics, including initialization. A host should not infer that
  core started successfully from shell-local control flow alone; core exposes a
  structured lifecycle event that hosts can render as startup status, logs, or
  web/socket events.
- Turn outcome and stopped-turn structure, including stats, usage, repair
  issue, stop reason, and stop detail. These public structures are the shared
  protocol between core and host UIs; hosts are expected to understand their
  fields and render them appropriately.
- Stopped-turn semantics as structured data. Core owns the reason/detail fields;
  hosts own localized/user-facing wording for those fields.
- Failure diagnostics and repair issues as structured observability data. Core
  should preserve machine-readable causes such as protocol issue names,
  model service errors, truncation flags, request ids, and stop summaries for audit
  and host rendering, but it should not turn those causes into localized
  terminal/app copy. Strings are valid core outputs when the string itself is
  data, such as model/user-visible answer text, paths, ids, model service messages,
  or diagnostic reason codes.
- Topic/event/request structures and fields. Core owns their stable semantic
  contract; host UIs subscribe to them, understand their public fields, and
  decide how to render or interact with them.
- Core/UI fields are semantic, not opaque. Hosts are expected to understand
  public fields such as action kind, final answer, progress report, diagnostic
  reason, status metadata, and request ids. Do not collapse those into a single
  untyped text field when the semantic distinction matters.
- Model service/model transport belongs behind core's model service boundary. The current
  implementation may use HTTP/curl internally, but that is a core/model service
  responsibility, not a shell UI responsibility. The intended chain is
  `host UI -> agent_core -> model service -> LLM`.
- Cross-language topic wire contracts. `CoreTopicEvent` payload field names are
  part of the shared core/UI boundary for Rust, Swift, web, and process IPC
  hosts. Rust typed accessors are host bindings over that wire contract, not a
  replacement for it. `CoreTopicEvent::wire_payload()` is the canonical envelope
  shape: `{ session_id, topic: { name, attributes }, state, payload }`.
- Topic callback lifetime. Core owns the emitted event batch while invoking
  registered callbacks. A callback that wants to render later, enqueue work, or
  cross a thread/process boundary must copy or clone the needed
  `CoreTopicEvent` or field values before it returns. After callbacks return,
  core may release its local event batch normally.
- Topic callbacks are notification/decision delivery points, not reentrant core
  entry points. A host callback must not synchronously call back into the same
  `AgentCore` session while core is emitting events; enqueue host-side work or
  return a `TopicReply` through the request path instead. This keeps future web,
  iOS, and multi-session hosts from creating callback reentrancy, lock
  inversion, or deadlock.
- Topic action payloads must use stable discriminated objects such as
  `{ kind: "bash", command: "..." }`, not Rust enum-default shapes. Hosts may
  branch on the public `kind` field when rendering action-specific UI.
- Topic reply correlation. Request topics that expect a reply must carry a
  `request_id`; host replies use `TopicReply { session_id, topic_name,
  request_id, decision, payload }`. Core owns validation of this tuple before a
  waiting session is resumed or before a safe default is applied.
- Turn lifecycle audit schemas and write helpers for turn start, model/system
  errors, repair requests, and final outcomes. Hosts decide when lifecycle
  points occur, but should not construct the shared audit JSON themselves.
- Report field semantics, stable row/section kinds, raw values, and effective
  state that are shared across hosts. Hosts may choose labels, descriptions,
  language, icons, and layout from those semantic kinds.
- Runtime configuration reports expose token limits as raw effective numbers;
  hosts own friendly unit formatting such as `100K` or localized descriptions.
- Command result message kinds and subjects for core-owned commands such as
  workspace management and runtime configuration updates. Hosts may localize
  and style them, but the semantic outcome should come from core data.
- Shared runtime status algorithms, including context percentage/bar fill,
  meaningful latest-usage selection, and bounded status-text compaction. Hosts
  own token/count display strings, icons, colors, and layout.
- Shared profiling metric algorithms, including KVC/cache-hit percentage,
  average wait per 1K output tokens, storage counts, and raw durations. Hosts
  own compact number/unit formatting, section names, icons, and terminal/web
  layout, but should not redefine the underlying metric calculations.
- Retry status semantics, including attempt defaults and countdown remaining
  time calculations. Hosts may choose wording and layout for retry messages.
- Runtime configuration application, including validation, model service/token
  field updates, and any resulting core state changes such as context-window or
  bash-approval policy updates.
- Host startup/runtime configuration synchronization. Hosts may collect env/CLI
  values, but core owns which config fields affect core state.
- Host-facing status/message data such as severity level and message text,
  without UI-specific icons, colors, or layout.
- Command/load result message data may include shared severity and semantic
  kind; hosts own localized copy, icons, colors, and layout.
- Protocol parsing results that drive progress/action UI updates. Hosts should
  receive structured topic events rather than reparsing model response text.

## Does not belong here

- Terminal rendering, ANSI escape sequences, Reedline/crossterm input handling,
  menus, cursor control, or shell-specific layout.
- Shell-only slash commands whose behavior is purely UI convenience.
- User-facing terminal copy that depends on a specific UI surface.
- Direct assumptions about how a host renders progress, errors, prompts, or
  confirmations.
- Browser/WebSocket-specific projection revisions, event cursors, reconnect
  outboxes, HTTP authentication, or UI command queues. These belong to a Host
  Projection Adapter such as `timem_web`, not to the reusable Core semantic
  projection.
- A separate lifecycle API for each UI toolkit. Rust, Swift, Web, desktop, and
  process-IPC bindings must expose the same Turn identity and transition
  semantics even when their language-level types differ.

## Interface rule

Core should expose functions, structs, enums, an authoritative Turn
projection, and topic event streams. Host adapters render or transport those
structures in their own style. When Core needs host input, it returns a
structured request rather than printing, reading from stdin, or assuming a UI
framework. Adding a new UI should require a binding/adapter and presentation
work, not a new Agent lifecycle implementation.

Threading rule: `AgentCore` is the state owner for one logical session/context.
The synchronous API is still valid for simple hosts. Hosts that need concurrent
sessions should run one `AgentCore` per session, usually via
`CoreSessionWorker`, instead of sharing a mutable core across sessions. Worker
threads are an adapter around the same function/topic interface: user input is a
function call into the worker, core-originated state is emitted as topic/events,
and host decisions return through `TopicReply`. Do not add ad hoc shared global
core state to make multi-session UI easier.

Function calls are the host/user initiated control surface: start a turn, update
configuration, add user input, query reports, or apply a host decision. Topic
events are the core-initiated runtime surface: progress, actions, requests,
waiting states, retries, and future background/session events. Topic events must
carry a session id so one host can multiplex multiple agent sessions without
global state.

In other words: if the user or host explicitly asks core to do something, expose
it as a function. If core is already running and needs to tell or ask the host
something on its own initiative, emit a topic event. The host subscribes to topic
events and maps them to callbacks, menus, panels, web events, logs, or ignored
background notifications.

Core-originated communication uses topic semantics. Non-blocking notifications
and blocking host-decision requests are both topic events; the difference is the
session state and whether a reply is expected. A request topic sets
`expects_reply=true` and moves the session to `waiting_user` or
`waiting_user_with_timeout` until the host returns a decision or the safe
default is applied. It also carries `request_id` so a host can reply safely even
when multiple sessions or repeated requests are active. Examples include
work-instruction loading, bash approval, round-limit continuation, output
expansion, and stale-context decisions. Core owns the request data, session
state, timeout, request id, reply validation, and default-safe meaning; host UIs
own rendering, keyboard/mouse interaction, and the final choice passed back to
core.

`TurnUi::request_host_decision_topic` is the core-owned adapter for active-turn
blocking requests: it publishes the request topic, lets the host reply through
`TopicReply`, validates that reply against the session/topic/request, and falls
back to the request's safe default if the reply does not match. Hosts should not
duplicate this correlation logic.

There are currently two implementation shapes for host-decision request topics:

- Active-turn requests go through the `TurnUi` callback interface. Core
  publishes a request topic, the host blocks or routes it to its UI, and the
  chosen result is returned as `TopicReply` so the same turn can continue.
- Startup or host-driven flows may call a core function that returns a structured
  request value, such as `WorkInstructionLoadRequest`. The host renders it and
  then calls the matching core function with the resulting choice or context.

Both shapes should be treated as the same architectural concept: core publishes
a topic describing what decision is needed; the host owns how the decision is
presented and timed.
If a host does not override a `TurnUi` request callback, core's default trait
implementation is still the behavior contract for that missing UI capability.

Naming rule: use `Input` for host-provided data passed into a core function
(`TurnInput`), and reserve `Request` for a core-originated decision that asks
the host/UI for a response (`HostDecisionRequest`, `WorkInstructionLoadRequest`,
`RoundLimitDecisionRequest`). Do not name ordinary host-to-core function inputs
as `*Request`, because that hides the direction of control.

Non-blocking notification topics carry status/progress while core is working,
such as action intent, job progress, retry status, or memory activity. A host may
render, throttle, or ignore those topics, but it should not treat them as
required user decisions unless the topic explicitly expects a reply and the
session state is waiting.

## Test Layout

Test functions and fixture corpora live under `agent_core/tests`. Production
modules may keep only a minimal `#[cfg(test)]` external-module declaration or
an explicitly test-only hook needed for private white-box access.
