# agent_core module boundary

`agent_core` is the reusable, UI-neutral Timem runtime. It owns agent state,
prompt/context management, model service payload/transport, capability
registration, tool execution coordination, memory access, retry policy, and the
authoritative semantic contracts consumed by every Shell, Web, iOS, desktop, or
future host. A new UI shell should adapt these contracts, not fork the agent
loop or reconstruct Turn lifecycle from events.

Before changing this module, also read the repository-level `AGENTS.md`.
Also read `docs/turn-state-projection-architecture.md` for the shared Core, Bridge, Interface, and lifecycle boundary.

## Belongs here

- Protocol-neutral runtime data structures and algorithms.
- `command_output` owns bounded retention and post-exit drain policy for finite command bindings and tool self-tests. Platform owns interruptible pipe reads; capture deadlines do not establish descendant ownership. Finite command bindings drain output before nonblocking stdin delivery and apply one execution deadline to input backpressure and process waiting. Input setup, write failure, or exit before complete delivery cannot be reported as success; no detached writer thread is created. Command-binding timeouts and input failures retain bounded captured stdout/stderr when available, or report capture failure separately, without replacing the timeout outcome or extending execution deadlines.
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
  reconnectable, or remote UI may place a reconnectable Bridge in front of
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
- Native HTTP uses a cached current-thread runtime and connection pool. Transport
  errors, cancellation, and incomplete/invalid streams retire that runtime and
  cancel its async connection tasks before returning, so rejected bodies cannot
  leave unread sockets retained while the runtime is idle. Successful complete
  responses retain keep-alive reuse; a subsequent request rebuilds a retired
  transport. Shutdown does not wait for already-running blocking DNS work.
- Model retry policy, retry decision data, and model-call outcome accounting.
  Model service I/O belongs behind the core/model service boundary. Hosts may surface
  waiting/cancellation UX, but should not redefine retryability or retry
  metadata.
- Native function-call capability negotiation. Core owns the strict endpoint/model identity,
  process cache, persisted-result validation, probe request and result classification, periodic
  re-probe of durable negative results, and `core.model.capability_negotiation` topics. Only an
  explicit provider rejection of native/function tools may become a durable unsupported result;
  authentication, rate limiting, cancellation, transport/5xx/generic errors, and successful
  responses without the requested probe calls remain inconclusive and keep the Native path.
  Formal model-request failures are not capability evidence and must not trigger an Inline
  fallback. Hosts may supply catalog knowledge and persist/clear exact Core records, but must not
  reinterpret these semantics.
- Capability and tool registries, including validation data loaded from
  resources.
- MCP client transport, server discovery, namespaced dynamic tool registration,
  and `tools/call` execution. The response-protocol parser remains generic: it
  parses the registered action name and JSON arguments, while the MCP server is
  authoritative for its full JSON Schema validation. MCP failures are bounded
  action evidence, never response-protocol repair; failed transports are evicted
  so a late response or dead connection cannot contaminate later calls. Current
  MCP definitions come from the capability registry on every request: Native
  mode sends them through provider API tools, while Inline mode renders one
  request-level current-capabilities section outside prompt deltas. Historical
  `mcp_capability_catalog` slices are suppressed during restore so stale schemas
  cannot compete with current registry state. Applying a host-requested MCP
  update compares complete tool definitions and submits one concise `SYSTEM`
  capability-change component only when the model-visible capability set
  actually changed; that notification never copies full schemas or instruction
  bodies.
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
  request. Each post-round dispatch boundary evaluates progress, time, and round
  schedules once, batching all due components into one prompt rebuild. Re-entering
  that boundary for host updates must not inject another period before dispatch;
  the next actual model round rearms evaluation. The first request has no reminder.
  Selecting `NONE` consumes that interval without prompt injection.
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
  Model-visible compression guidance must tell the model what to inspect,
  preserve, summarize, select, and verify. It may state observable consequences
  needed to make a safe choice, but must not explain Core's deletion algorithm,
  telemetry, prompt-rewrite sequence, or reinjection implementation; those
  details belong in this boundary document, code comments, and tests.
  Context compression is retain-by-exception: only deltas explicitly named in
  `keep` remain verbatim; every other live delta is removed after useful state
  is extracted into the authoritative summary. Optional `offload` ids are saved
  to scratch before removal and may not overlap `keep`; missing keep/offload ids
  fail closed. Successful compaction action results stay minimal: the next model prompt gets
  completion status and only actionable follow-up data such as an offload
  `scratch_id`, never the full discarded/offloaded delta-id lists, shrink
  counters, or post-shrink live-ref diagnostics. Detailed ids and token
  accounting remain available to structured host topics; failed compaction may
  include missing ids and current live refs so the model can repair its call.
  Current MCP schemas are request-level capability state rather than prompt
  history: they have no delta id, are not selectable by `keep`/`offload`, and
  remain available after compression without reinjection. The full rendered
  prompt estimator includes the Inline MCP section, while the dynamic-history
  estimator excludes it because compression cannot remove it. Across successive
  successful compactions, every
  assistant-authored replacement summary remains authoritative, while only the
  latest runtime CWD/memo success confirmation stays model-visible.
- Dynamic prompt-context snapshots and reset semantics. Core owns the complete
  consistency unit: rendered deltas, Native exchanges, prompt-token baseline,
  active memo, and pending runtime-authority memo notices. Import replaces the
  entire prior unit, including when the imported snapshot is empty; clear drops
  every model-visible and one-shot pending component so the next turn is fresh.
  Core also owns whether this compound snapshot is empty. Hosts may atomically
  persist, consume, and restore it, but must not infer emptiness from one field.
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
- The single model-visible tool-result gate. Core validates the supported per-tool result budgets, defaults each AgentCore to 16 KiB, and bounds the complete structured action-result envelope while preserving valid JSON and head/tail retention semantics; Hosts may configure the value but may not recreate the gate.
- Disk-pressure observation gates, comparable-sample baselines, thresholds and model notices. Filesystem discovery, device identity, deduplication and usage sampling are owned by `core/platform::filesystem_usage_snapshot`; Agent supplies only working paths and consumes platform-neutral snapshots.
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
- Foreground `run_bash` action timing and lifecycle topics. Core publishes the
  effective wait budgets used by execution, including defaults when the model
  omits `timeout_ms`, `loop_timeout_ms`, or `once_timeout_ms`. The action
  lifecycle uses `event: "start"` for proposal/approval visibility,
  `event: "execution_start"` when foreground execution handling begins, and
  the terminal finish event for settlement. Hosts must start countdown UI from
  `execution_start`, not from proposal or approval time.
- Long foreground command lifecycle for positive model-provided `timeout_ms`:
  core owns process waiting, long-running decision requests, timeout transition
  into the session running-pid set, action result shaping, and user-supplement
  insertion after host/user cancellation.
- Structured reports, requests, stop reasons, status snapshots, and topic events
  for any host UI to render.
- Agent-owned collaboration ports for Session orchestration. `agent_core` exposes narrow APIs for
  detached-resource cancellation, running-job refresh with runtime event delivery, and temporary
  ToolGen capability activation. It does not expose its job stores or capability registry for
  Session code to mutate directly.
- Session worker lifecycle, worker threads, multi-worker management, scheduling, shutdown, and
  worker status belong to `timem_session`, which coordinates one `AgentCore` per Context.
- Worker identity and workspace projections remain UI-neutral contracts re-exported by Agent for
  compatibility; their lifecycle policy and mutable ownership do not belong in this crate.
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
  implementation uses a native Rust HTTP client with rustls, but transport choice remains a
  core/model service responsibility, not a shell UI responsibility. The intended chain is
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
  Projection Adapter such as `bridges/http_websocket` plus its Application wiring,
  not to the reusable Core semantic projection.
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

Threading rule: `AgentCore` is the state owner for one logical Context. The synchronous API is
still valid for simple hosts. Hosts that need concurrent Sessions should use `timem_session`,
which runs one `AgentCore` per Context instead of sharing mutable Agent state. Agent-originated
state still uses the same topic/event interface, and host decisions return through `TopicReply`.
Do not add ad hoc shared global Agent state to make multi-session UI easier.

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

Agent test functions and fixture corpora live under `core/agent/tests`. Session orchestration
tests live under `core/session/tests`. Production
modules may keep only a minimal `#[cfg(test)]` external-module declaration or
an explicitly test-only hook needed for private white-box access.

Session metadata also preserves an optional `model_endpoint_id` for stable
host-managed endpoint selection. Old records deserialize without this field;
secrets remain in owner-protected configuration, never in the binding ID.

## Reasoning policy boundary

`reasoning.rs` owns protocol-independent preference, per-request demand and effective
policy. Version 0 retains legacy ordinary/required behavior; version 1 uses daily H0
and optional critical-call H1 within the model-ordered allowed subset. Unknown
capabilities never invent an ordering. Explicit adaptive disable is retained.
Existing persisted `OpenAiCompatibleOptions` reasoning fields are a compatibility
input, not independent policy authority in each adapter. `model_api` resolves them
once through `model_requirements`; `model_payload` maps the effective policy to
Chat Completions, Responses or the legacy Anthropic path.
Callers may also supply `EffectiveReasoning` to `build_model_request_with_policy`.
Protocol adapters must not reinterpret scheduling flags or downgrade intensity.
Anthropic uses adaptive thinking and output effort, not invented token budgets;
unsupported protocol-level effort is rejected before HTTP. Model-specific adaptive
support is validated upstream (no model-name heuristics or silent legacy fallback). A model-facing reasoning trailer is injected only when the resolved request policy is a real increase above the normal H0 baseline; a critical scheduling flag or enabled thinking alone is insufficient. The trailer asks the model to use the stronger pass for direction, methodology, and corrective reflection while preserving the active response/compaction protocol.

### 模型白名单接入
`model_catalog` 负责内置/启动目录 JSON 描述、UI 无关投影、默认值、native-safe 约束及预算准入。`model_requirements` 以显式供应商+模型匹配能力并解析本次需求，模板 ID 仅为建议来源（v0 保留绑定兼容）。`model_payload` 选择描述中已注册的适配器并做最终字段一致性校验。配置不能执行脚本或任意 JSON path；新增同协议模型只需描述文件，新 wire 行为必须实现并测试处理器。详见 `docs/model-endpoint-architecture.md`。

### Responses SSE
Responses 复用有界 SSE 分帧与 HTTP 取消/超时传输。会话预览只接收 response.output_text.delta，忽略 reasoning 与函数参数增量；权威结果必须来自 response.completed 或 response.incomplete 的完整 response，复用非流式的文本、函数调用、用量解析。缺失/重复/结构错误的结束事件或 error/response.failed 均拒绝作为成功结果。函数调用不从未完成参数增量执行。

### Zhipu catalog adapter

Built-in model profiles select a typed reasoning adapter in Core. Zhipu Chat
profiles map thinking fields and enforce final-wire consistency before I/O.
Assistant reasoning continuation is bounded opaque metadata carried on the first
native call of an exchange, not tool arguments or public assistant text. It is
replayed only for the same capability descriptor and model (not the editable template source); absent legacy metadata is
valid. Cross-turn preserved thinking is not promised across context compression.
See `docs/zhipu-model-catalog.md` for scope, evidence and tests.
