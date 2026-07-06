# timem_shell module boundary

`timem_shell` is the terminal host for Timem. It owns CLI parsing, environment
collection, terminal input, menus, rendering, and shell-only convenience
commands. It delegates reusable runtime behavior and model/provider transport
to `agent_core`.

Before changing this module, also read the repository-level `AGENTS.md`.

## Belongs here

- Terminal UI rendering, ANSI styling, prompt lines, status panels, and menus.
- Final-answer Markdown rendering behind `final_answer_renderer`, with a
  replaceable renderer interface and a community renderer backend by default.
- Reedline/input handling, paste recovery, Ctrl+C/Esc behavior, and TTY quirks.
- CLI flags, process env collection, shell history, and startup banner display.
- Collecting CLI/env config values, then passing the effective runtime config to
  core. Shell should not duplicate which config fields update core state.
- Shell-only slash commands that improve terminal usage.
- Shell wrappers around core-owned commands, such as exposing core runtime
  configuration through `/config`. Shell owns the menu/input/rendering; core
  owns validation, state changes, and structured command reports.
- Collecting shell-side context sources such as work-instruction text or
  workspace references, then passing them to core for model-context assembly.
- Supplying shell host identity such as `runtime=timem_native_shell` and
  `run_bash_target=user_local_machine` through `TurnInput`. Other hosts must
  be able to supply different values without changing the reusable turn loop.
- Collecting active-turn user supplement input from the terminal, then passing
  it to core. Shell should not duplicate prompt-slice insertion or supplement
  audit logic.
- Rendering approval prompts and collecting approval decisions, then passing
  those decisions to core. Shell should not duplicate pending-action resolution
  or approval audit logic.
- Rendering round-limit prompts and collecting continue/stop choices, then
  passing those choices to core. Shell should not duplicate round-budget
  recharge, round-limit audit, or stop-summary construction.
- Rendering output-expansion prompts and collecting expand/stop choices, then
  passing those choices to core. Shell should not duplicate max-output-token
  updates, output-expansion audit, or output-limit stop-summary construction.
- Rendering stale-context prompts and collecting continue/reset choices, then
  passing those choices to core. Shell should not duplicate dynamic-context
  clearing or stale-context audit events.
- Applying model responses through core and rendering resulting topic events.
  Shell should not compare repair counters, classify repair issues, inject
  repair prompt slices, or construct model-repair audit events.
- Adapters that translate `agent_core` structured reports/topic events into
  terminal text.
- Terminal rendering of core-provided report semantics and values. Shell owns
  labels, descriptions, language, icons, layout, and compact display strings;
  core owns the semantic field kinds and effective values.
- Terminal display formatting for runtime config values such as token limits;
  shell may show `100K`, while core reports the raw effective token number.
- Terminal localization and styling of core-provided command result messages;
  shell should not infer reusable command outcome semantics from raw internal
  state when core already provides a message kind and subject.
- Terminal composition of core-provided runtime status values. Shell owns
  symbols such as KVC markers, token/count display strings, line layout, colors,
  and terminal copy; core owns context math and bounded status-text compaction.
- Terminal rendering of core-provided profiling values. Shell may choose the
  `/prof` layout, labels, language, and compact number/unit formatting; core
  owns KVC/cache-hit percentage, average wait per 1K output, storage counts, and
  raw durations.
- Terminal wording for core-provided retry status views. Shell may choose
  language, tree markers, and truncation, but should not compute retry attempt
  defaults or countdown semantics.
- Terminal wording for core-provided turn stop/outcome structures. Shell may
  match public core enums and fields such as `TurnStopDetail` to choose terminal
  copy/layout; those fields are the shared protocol, not opaque internals.
- Terminal wording for structured failure diagnostics. Shell may render
  protocol repair failures, provider errors, truncation flags, and other
  core-provided observability fields as localized user-facing copy, while core
  keeps those causes machine-readable for audit and other hosts.
- Terminal rendering of stopped-turn messages. The turn loop should return
  `TurnStopSummary`/`TurnStopDetail` as structure; shell decides the localized
  text shown to terminal users.
- Topic/event/request rendering and interaction. Shell subscribes to core
  structured topic events and requests, understands their public fields, and
  maps them to terminal UI affordances such as menus, prompts, status panels, or
  transient messages.
- Lifecycle topic rendering. Shell may render `core.lifecycle` as startup status
  text, but it should not invent reusable core lifecycle state from local
  control flow when core already exposes a structured topic.
- Session worker display. Shell may render worker display names such as `ID0`
  or user-renamed labels from lifecycle topics. It must not keep the canonical
  worker identity as terminal-only state; identity belongs to the core/UI
  protocol so future web/iOS hosts see the same worker.
- Rust shell may use typed topic accessors, but the cross-language contract is
  still the `CoreTopicEvent` wire shape. Shell tests should not be the only
  place where topic payload fields are defined.
- Shell may branch on stable action topic fields such as `kind: "bash"` for
  rendering, but should not depend on Rust enum-default serialized shapes.
- Terminal replies to core request topics. Shell collects the user choice and
  returns it through core's `TopicReply` shape with the original `session_id`,
  `topic_name`, and `request_id`; shell should not resume a waiting session by
  matching only terminal-local state.
- Calling core turn lifecycle audit helpers at terminal turn boundaries. Shell
  should not construct shared turn_start, turn_error, model_repair_request, or
  turn_final audit JSON.
- Rendering `agent_core` host status messages with terminal-specific icons,
  colors, and layout.
- Rendering core-provided message severity and semantic kind into terminal
  wording. Shell may localize the sentence, but the shared severity and kind
  should come from core-owned data.
- Terminal-side cancellation and interaction while a model turn is running.
  Shell may signal cancellation through core APIs, but it must not implement
  provider HTTP/curl, provider endpoint/header/body construction, cache-control
  protocol, or provider response/error interpretation.
- Terminal-side approval UI for model-requested tools such as `run_bash`.
  Shell may render the request and collect the user's choice, but it must not
  execute the command, shape stdout/stderr evidence, decide tool semantics, or
  write tool audit records.
- Terminal-side decision UI for long foreground `run_bash` commands with
  positive model-provided `timeout_ms`. Shell renders the
  keep-waiting/stop-waiting menu and returns the user's decision through the
  core host-decision channel; core owns the process lifecycle, action result,
  and any follow-up `user_supplement`.
- Local host concerns such as where this shell process stores history or audit
  files.
- Choosing host policy for this terminal process, while using `agent_core`
  capability profiling so the active capability set matches the actual
  environment. Shell should not imply it is the only host that can have Bash;
  any host with local command execution may expose that capability through core.

## Does not belong here

- Model response protocol parsing rules.
- Provider/model transport, including HTTP/curl execution, SDK calls, provider
  payload construction, provider response interpretation, and provider
  cache-control details.
- Parsing raw model responses to infer Thought / Action UI events; core must
  emit structured topic events for those events.
- Capability parameter parsing and tool semantics.
- Built-in tool callback implementation files. Those belong with their YAML
  capability manifests under `resources/capabilities/tools/`, and are invoked by
  `agent_core` dispatch rather than by terminal UI code.
- Model-requested tool execution, including `run_bash` process execution,
  command output/evidence shaping, command status handling, and tool audit.
- Registered tool job lifecycle management, including background job ids,
  status files, output files, polling/cancel semantics, process termination,
  and timeout policy. Shell may display the resulting core topics or action
  evidence, but must not manage those jobs itself. Final-answer/context-compact
  cleanup is also a core responsibility.
- Foreground long-command process waiting and cancellation. Shell may prompt the
  user for a decision but must not kill or track the model-requested process
  itself.
- Memory conflict logic, context shrink/compact algorithms, provider cache
  planning, or retry policy.
- Runtime configuration validation or reusable configuration side effects, such
  as changing context-window state or bash-approval policy in core.
- Prompt/resource language that should be shared by other Timem hosts.
- Prompt/supporting-context assembly rules. Shell should provide context source
  values, not decide how runtime identity and additional context are combined.
- Session prompt component assembly. Shell may submit user input, user
  supplements, and host-provided context values through core APIs, but it must
  not build dynamic prompt deltas itself or format `## USER` / `## SYSTEM` /
  assistant blocks. The per-session pending component buffer and
  `build_next_prompt()` belong to `agent_core`.
- Long-lived runtime state that another UI would need to reproduce.
- User-facing stopped-turn copy inside the reusable turn loop. A stopped turn
  should be represented structurally so non-terminal hosts can render it in
  their own language and UI style.

## Interface rule

Shell code may choose how to render and interact, but it should ask core for
structured data and call core functions for reusable behavior. If a feature
would also be needed by a future web UI or another host, put the data model and
business logic in `agent_core`, then render it here.

Threading rule: the terminal host may keep using the synchronous
`run_session_turn` path for its single active session. If shell or a future host
needs concurrent sessions, it should use a core-owned per-session worker rather
than building a separate terminal-specific runtime loop. Shell remains
responsible for terminal input/rendering; the worker/core owns session state,
model/action loop, topic emission, and request correlation.

The intended call chain is `timem_shell UI -> agent_core -> provider -> LLM`.
Shell should not insert a transport adapter between UI and provider. If provider
transport needs to change, change the core/provider boundary first.

Shell may expose core functions as shell-specific commands such as `/prof`,
`/config`, or `/workspace`, but those commands should return or render
core-owned data structures. Core-initiated runtime events should be consumed as
topic callbacks, not inferred by parsing prompt text or model output in the
shell.

Shell-initiated actions call core functions directly. Core-initiated work in an
already-running session arrives as topic events, including progress, waiting
states, retries, and requests that need a user decision. Shell may route those
topics to terminal callbacks, but it should not turn them back into ad hoc
string parsing.

Active-turn progress and action rendering should consume `CoreTopicEvent`
payloads emitted by core. The shell may map those topic payloads to terminal
panels, status bars, colors, and line wrapping, but it should not recreate the
underlying action/progress semantics from protocol text.

When core returns or asks through a structured request, shell owns the terminal
interaction around that request: menu layout, Ctrl+C/Esc handling, and optional
timeouts. Optional requests should have a safe timeout default when blocking the
terminal would hurt usability. For example, the AGENTS.md/CLAUDE.md load prompt
times out after 30 seconds and continues without loading.

For active turns, shell receives core requests through its `TurnUi`
implementation and replies through core's `request_host_decision_topic` flow.
Shell renders and collects the choice, while core publishes the topic, assigns
request correlation, validates the `TopicReply`, and applies the safe default on
mismatch. For startup/outer-shell flows, shell may call a core request builder,
render the returned request, then call the matching core operation. In both
cases shell should avoid inventing business rules that belong in core request
metadata, such as safe defaults or whether a request is optional.

When core emits topic events, shell treats non-request topics as non-blocking display updates.
They feed the status bar or Thought / Action panel, but they are not the channel
for decisions that require a user choice.

Topic callbacks are synchronous. If shell wants to preserve an event for delayed
rendering, background processing, logs, or tests after the callback returns, it
must clone the `CoreTopicEvent` or copy the specific fields it needs.
Shell topic callbacks must not synchronously reenter the same `AgentCore`
session. They should render/copy/enqueue, or return the correlated `TopicReply`
for request topics, then let the active core call continue.
