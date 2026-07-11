# Core/UI topic protocol

Timem core emits topic events as the cross-language protocol between the
runtime and host UIs. A host UI may be Rust shell, Swift app, web UI, or another
process. Therefore the protocol has two layers:

- Wire contract: `CoreTopicEvent` serialized as structured JSON-like data.
  This is the stable cross-language boundary.
- Host binding: language-specific helpers such as Rust `as_action()` or
  `as_host_decision_request()`. These are convenience bindings over the same
  wire contract, not a replacement for it.

## Event envelope

Every topic event has the same envelope:

```text
session_id: string
topic:
  name: string
  attributes: object
state:
  name: running | waiting_model | waiting_user | waiting_user_with_timeout | paused | stopped | finished | error
  timeout_ms?: integer
payload: object
```

`session_id` is required so a host can multiplex multiple agent sessions.
`topic.name` identifies the semantic payload shape. `state` tells the host what
the session is doing when the topic is emitted. `payload` is typed by
`topic.name`.

Core-originated communication is topic-based. Non-blocking notifications and
blocking host-decision requests use the same envelope. A request topic is
distinguished by:

- `topic.attributes.expects_reply: true`
- `state.name: waiting_user` or `waiting_user_with_timeout`
- a payload containing the request kind, safe default, optional timeout, and
  request body

The host renders the request, collects or defaults the decision, then sends the
decision back through the matching core entry point. Until then, the session is
logically suspended.

Host bindings may expose helpers such as `expects_reply()` and
`is_blocking_request()`, but those helpers must derive from the same wire fields
above. A topic is a blocking request only when it both expects a reply and the
session state is waiting.

Topic callback lifetime is synchronous. Core owns the emitted event batch while
calling registered callbacks. If a host wants to render, queue, or forward a
topic asynchronously after the callback returns, it must copy or clone the
needed `CoreTopicEvent` or field values. Core may release its local event batch
after all callbacks return.

## Topic replies

When a host answers a blocking request topic, it should use a single reply shape
instead of inventing per-request response channels:

```text
session_id: string
topic_name: string
request_id?: string
decision: accept | decline
payload: object
```

`session_id` selects the suspended session. `topic_name` identifies the request
topic being answered. `request_id` is optional today and reserved for hosts that
need multiple outstanding requests in the same session. `decision` carries the
high-level accept/decline answer. `payload` carries topic-specific details when
a future request needs more than a boolean decision.

After receiving the reply, core decides how to resume the session: continue the
same turn, apply a safe default, append a prompt slice to a later delta, or stop
the turn. The UI should not duplicate that runtime policy.

Core validates a reply against the pending request topic before applying it. A
reply with the wrong `session_id`, `topic_name`, or `request_id` must not resume
the session. Replying to a non-blocking notification topic is also invalid.

## Current topics

### `core.lifecycle`

Core lifecycle events for a session. The current lifecycle event is
`initialized`, emitted after a core/session worker has been initialized and is
ready to receive turns.

```text
payload:
  event: initialized
  version: string
  profile:
    name: string
    provider: string
    model: string
  response_protocol: string
  max_llm_input_tokens: number
  max_rounds: number
  capabilities:
    tools: number
    skills: number
  worker?: null | object
    session_id: string
    display_name: string
    ordinal: number
    parent_session_id?: null | string
  workspace?: null | object
    current_dir?: null | string
    data_dir: string
    audit_file: string
    runtime: string
    run_bash_target: string
    env: object
    workspace_dirs: string[]
  context?: null | object
    visible_delta_count: number
    visible_slice_count: number
    estimated_tokens: number
```

This topic is non-blocking. Hosts may render it as startup status, logs, or
web/socket state. Hosts should not infer reusable core lifecycle state solely
from host-local control flow.

Worker identity is the cross-host display identity for a logical session worker.
Default names are `ID0`, `ID1`, ... by ordinal, and hosts may allow users or
parent workers to rename them. Core redacts secret-looking workspace env values
before publishing lifecycle payloads; hosts should still avoid sending API keys
or other secrets as worker metadata.

Hosts that need multiple concurrent sessions should prefer
`CoreSessionWorkerManager` or an equivalent adapter preserving the same protocol:
the default worker is `ID0`, additional unnamed workers are `ID1`, `ID2`, ...
and all workers created by one manager share the global working-worker counter.

### `core.model.response`

Structured model response metadata from core after a model response is parsed
and accepted by the response protocol. Core must not emit this topic for a
malformed response that is being repaired.

```text
payload:
  status: working | finished
  free_talk: string
  final_answer: string
  continue_work: boolean
  global:
    working_worker_count: number
```

`free_talk` is lightweight model context that a host may render as a status
note during an active turn. `global` carries cross-session state; hosts can keep
a global thinking indicator visible while `working_worker_count > 0` and stop it
when the count reaches zero. Direct single-turn shells use `1` while the current
turn continues and `0` when it is finished. Session-worker hosts override this
with the shared atomic worker runtime count.

### `core.action`

Action metadata from core.

```text
payload:
  action: string
  input: object
  kind: object
  active: boolean
  memory_activity: none | read | write
```

Host UIs may render this as a Thought / Action panel, timeline item, log row, or
ignore it. They should not reparse model text to recover this information.

### `core.user.*.request`

Core is asking the host for a user/host decision.

Current names:

- `core.user.approval.request`
- `core.user.round_limit.request`
- `core.user.output_expand.request`
- `core.user.stale_context.request`
- `core.user.work_instruction_load.request`

Common payload:

```text
topic.attributes:
  name: string
  kind: string
  expects_reply: true
state:
  name: waiting_user | waiting_user_with_timeout
  timeout_ms?: integer
payload:
  kind: user_approval | round_limit_continue | output_expansion | stale_context_continue | work_instruction_load
  safe_default: accept | decline
  timeout_ms?: integer
  request: object
```

`safe_default` is core-owned policy. Hosts own the interaction style and may use
menus, buttons, touch UI, timeouts, or default callbacks. If a host does not
implement a decision UI, it should apply the safe default.

## Binding rule

Core should keep tests for both layers:

- Wire-shape tests check the JSON payload field names and values that Swift,
  web, or process IPC consumers rely on.
- Binding tests check Rust typed accessors round-trip the same event into typed
  structures.

Do not remove wire-shape tests merely because a Rust typed accessor exists.
