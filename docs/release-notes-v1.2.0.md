# TimemAi 1.2.0

TimemAi 1.2.0 expands the runtime from a single inline action protocol into an
adaptive agent platform: models can use provider-native tool calls when the
configured API supports them, and users can build a reusable Role library to
shape how work is carried out across Sessions.

## Highlights

### Native tool call support

- **Provider-native execution:** Timem now sends tool definitions through the
  native model API and accepts structured tool calls from OpenAI-compatible
  Chat, OpenAI Responses, and Anthropic endpoints.
- **Adaptive compatibility:** `TIMEM_TOOL_CALL_MODE=auto` is the default. Timem
  probes the configured model/gateway, uses native calls when supported, and
  falls back to the established inline XML/JSON protocol when they are not.
- **Parallel and ordered workflows:** native calls preserve tool-call/result
  correlation and model-input order, while parallel-call support can be
  negotiated automatically or controlled with `TIMEM_PARALLEL_TOOL_CALLS`.
- **One capability contract:** builtin, MCP, and runtime-overlay tools share the
  same manifest-backed schemas and result gate across native and inline modes,
  reducing protocol drift without weakening validation or output bounds.

### Roles for different kinds of work

- **Reusable working methodologies:** define Roles such as reviewer,
  investigator, implementer, release manager, or any workflow your team needs.
  A Role contributes its saved methodology to the selected message's task.
- **MEM-wide Role library:** Roles are shared across Sessions in the active MEM,
  durably stored, and available without recreating instructions in every chat.
- **Organize and combine:** create groups, reorder Roles with drag and drop, and
  select one or more Roles for a message when a task benefits from multiple
  perspectives or responsibilities.
- **Traceable application:** selected Roles are shown in the composer and on the
  resulting user entry, so the working context remains visible in history.

## More in 1.2.0

- Timem Web adds streaming controls for OpenAI-compatible endpoints, connection
  and inactivity timeouts, shared endpoint management, and safer secret reveal
  and copy controls.
- Temporary runtime data can be retained for 1, 5, or 10 days, or indefinitely,
  while user and assistant messages remain outside this cleanup policy.
- Queued questions, long-running shell jobs, Session lifecycle recovery,
  reconnect/replay behavior, and cross-platform process supervision are more
  resilient on macOS and Linux.
- Web management now includes Session groups, improved message navigation,
  ToolRepo and MCP controls, attachment handling, runtime diagnostics, and a
  committed production bundle embedded in `timem-web`.
- The production gate covers strict Rust checks, frontend tests and reproducible
  builds, release/performance/edge guards, Web lifecycle and public-mode smokes,
  Linux platform checks, and real pseudo-TTY interaction tests.

## Upgrade

```bash
git pull --ff-only
./install.sh
timem-web
```

For native tool calling, keep the default `TIMEM_TOOL_CALL_MODE=auto`, or set it
explicitly to `native` or `inline`. Open the Roles panel in Timem Web to create
and organize working methodologies, then select the Roles to use before sending
a message.
