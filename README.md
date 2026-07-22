# TimemAi

TimemAi is a local-first AI agent runtime with two user interfaces:

- `timem`: a native terminal UI for shell-heavy work.
- `timem-web`: a token-protected browser UI built on assistant-ui; it binds to
  loopback by default and can opt into public binding.

Both hosts use the same Rust `agent_core`: model calls, prompt/context
management, memory, capability execution, audit logs, session history,
provider adapters, and safety boundaries are shared. The UI layer only decides
how to collect input and render structured core events.

## 1.0: Web As A First-Class Host

Version 1.0 makes the browser host a first-class Timem experience while the
terminal remains fully supported. `timem-web` provides an authenticated local
browser workspace built on assistant-ui with:

- multiple isolated Sessions with independent model/provider profiles
- persistent chat history, cross-host resume, paged history loading, and mem
  space switching
- live Thought/Action work streams, compact tool rows, inline runtime decisions,
  active-turn supplements, cancellation, and reconnect handling
- Markdown/GFM answers, syntax-highlighted code, attachments, context-compact
  visualization, cwd display, and token/time telemetry
- responsive desktop/mobile layout with theme, font, text-size, accessibility,
  and keyboard interaction support

The browser UI is a host renderer, not a second agent runtime. Provider calls,
memory, prompt construction, tool execution, safety checks, session workers,
and structured topics remain in `agent_core`; `timem_web` transports those
structures over authenticated HTTP/WebSocket connections.

## What It Does

- Runs local agent turns with provider adapters for OpenAI-compatible,
  OpenAI Responses, and Anthropic-style APIs.
- Executes model-requested tools such as guarded Bash, memory management,
  capability loading, and runtime self-inspection.
- Keeps local memory, raw chat history, scratch/context notes, API/action audit,
  and resumable sessions under a selected data space.
- Supports multi-session Web use with per-session model/provider settings,
  uploads, inline approvals, live activity, final answer telemetry, and history
  paging.
- Supports terminal workflows with paste recovery, thinking-time supplements,
  `/config`, `/prof`, `/workspace`, and real TTY cancellation behavior.

Everything is local except the model provider request you configure.

## Repository Layout

```text
agent_core/    reusable runtime, provider adapters, tools, memory, sessions
timem_shell/   native terminal host and shell UI
timem_web/     local HTTP/WebSocket host for browser sessions
web_ui/        assistant-ui React frontend embedded into timem-web
resources/     system prompt, response protocols, capability manifests
docs/          architecture, topic protocol, capability, testing, release docs
scripts/       CI, release, prompt snapshot, smoke and guard scripts
```

Start with:

- [Architecture](docs/architecture.md)
- [Install and configuration](docs/install-and-configuration.md)
- [Core/UI topic protocol](docs/core-ui-topic-protocol.md)
- [Capability system](docs/capability-system.md)
- [Test strategy](docs/test-strategy.md)
- [Feature/test ledger](docs/feature-test-management.md)
- [Release smoke checklist](docs/manual-release-smoke.md)

## Install

```bash
git clone https://github.com/moliam/TimemAi.git
cd TimemAi
./install.sh
```

The installer builds both release hosts and installs:

Cargo downloads Rust crates and resolves the project dependencies automatically
during the build; no separate Rust dependency installation step is required.

- `timem`
- `timem-native-rs`
- `timem-web`

Release source packages do not require a separate assistant-ui checkout or
Node.js at runtime; the tracked production Web bundle is embedded into the Rust
binary. Development of `web_ui/` does require pnpm/Node.

## Configure

Use `env_template` as the complete reference:

```bash
cp env_template env
$EDITOR env
source /path/to/your/env
```

Minimum Aliyun-compatible example:

```bash
export TIMEM_GATEWAY_PROVIDER=aliyun
export TIMEM_API_KEY=your_api_key_here
export TIMEM_MODEL=qwen-plus
export TIMEM_SPACE=.test_mem
```

Useful defaults:

```bash
export TIMEM_RESPONSE_PROTOCOL=xml
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_MAX_LLM_INPUT=100K
export TIMEM_MAX_LLM_OUTPUT=10K
export TIMEM_WORK_INSTRUCTIONS=silent
export TIMEM_BASH_APPROVAL=ask
```

Command-line options override process env values:

```bash
timem --help
timem-web --help
```

## Run

Terminal UI:

```bash
source /path/to/your/env
timem
```

Web UI:

```bash
source /path/to/your/env
timem-web
```

`timem-web` binds to `127.0.0.1` by default, chooses a port in
`12345..=23456` unless `--port` is set, generates a per-process access token,
and opens the authenticated local page in the default browser. Use
`timem-web --public` only when you intentionally want it reachable through the
machine's network address; browser entry, API, upload, and WebSocket access
require the printed token or an authenticated browser session cookie. Public
mode prints a directly usable URL using the detected host address. For a
multi-interface server or reverse proxy, set `TIMEM_PUBLIC_HOST` or pass
`--public-host <host>`. Public mode does not try to open a browser on the
server; use `--no-open` explicitly for clarity in scripts.

Remote server example:

```bash
source /path/to/your/env
timem-web --public --public-host 10.125.112.83 --port 20699 --no-open
```

Open the complete tokenized URL printed by the server on your local machine.

Run directly from source during development:

```bash
cargo run -p timem_shell
cargo run -p timem_web
```

## Runtime Data

By default Timem writes runtime data under the current directory:

```text
data/<space>/
  audit/
  memory/
  sessions/
  shell_history.txt
```

Use a fixed data root when needed:

```bash
export TIMEM_DATA_DIR=/path/to/data
export TIMEM_SPACE=my_project
```

Shell and Web share the same core session store and raw chat history format.
Web restores the newest history page first and loads older pages on demand.
Shell can resume the same stored session data.

## Development

Frontend changes:

```bash
pnpm --dir web_ui/timem-web install --frozen-lockfile
pnpm --dir web_ui/timem-web test --run
pnpm --dir web_ui/timem-web build
cargo test -p timem_web
```

Full production gate:

```bash
scripts/ci.sh
```

The gate covers script syntax, module boundaries, install logic, prompt
snapshots, sensitive scans, Rust format/clippy/tests, Web tests/build,
performance guards, repeated edge regressions, release builds, cross-host
resume smoke, and real TTY smoke/stress.

## Provider and Protocol Details

Common provider setups:

```bash
# Aliyun DashScope compatible mode
export TIMEM_GATEWAY_PROVIDER=aliyun
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_RESPONSE_PROTOCOL=xml
export TIMEM_MAX_LLM_INPUT=100K
export TIMEM_MAX_LLM_OUTPUT=10K
```

```bash
# OpenAI
export TIMEM_GATEWAY_PROVIDER=openai
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-responses
```

```bash
# Anthropic
export TIMEM_GATEWAY_PROVIDER=anthropic
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=anthropic
```

`TIMEM_GATEWAY_PROVIDER` chooses the traffic platform and default URL.
`TIMEM_API_PROTOCOL` chooses the provider HTTP wire format. Supported values:

- `openai-compatible`
- `openai-responses`
- `anthropic`

`openai-compatible` means the Chat Completions-compatible shape
(`/chat/completions`). `openai-responses` means OpenAI's Responses API shape
(`/responses`), where output text and usage fields differ from Chat
Completions.

If `TIMEM_API_PROTOCOL` is omitted, `TIMEM_GATEWAY_PROVIDER=openai` uses
OpenAI Responses, `TIMEM_GATEWAY_PROVIDER=anthropic` uses Anthropic protocol,
and other providers use OpenAI-compatible chat completions. For a custom
gateway, set both `TIMEM_API_PROTOCOL` and `TIMEM_BASE_URL` explicitly.

OpenAI-compatible gateways may expose optional reasoning and SSE extensions.
Timem accepts `TIMEM_ENABLE_THINKING=true`, `TIMEM_REASONING_EFFORT=<value>`,
and `TIMEM_STREAM=true`. Streaming responses are assembled into the normal
provider result; private `reasoning_content` is not exposed as assistant output.
Use these settings only when the selected gateway documents them.

`TIMEM_RESPONSE_PROTOCOL` chooses how the model must format its response for
the local runtime parser. Supported values are `markdown`, `json`, and `xml`;
the default is `xml`.

`TIMEM_WORK_INSTRUCTIONS` controls whether Timem loads `AGENTS.md` and
`CLAUDE.md` from the current working directory into agent context. Supported
values are `silent` (default, auto-load and notify), `ask`, and `off`.

`TIMEM_MAX_LLM_INPUT` defaults to `100K`; `TIMEM_MAX_LLM_OUTPUT` defaults to
`10K`. When observed provider input tokens plus the new prompt delta estimate
reaches 90% of `TIMEM_MAX_LLM_INPUT`, runtime requires the model to compact
dynamic prompt deltas before continuing: summarize useful dynamic context to
about 10%-20% of its current token footprint, discard stale details, and place
important but lengthy state into scratch memory by using the response
protocol's `context_compact` block. For prompt context the model can ask
runtime to discard old prompt delta ids or offload prompt delta ids into
scratch instead of rewriting that context itself.

If a provider reports that output was cut off by the output-token limit, Timem
asks whether to temporarily increase `TIMEM_MAX_LLM_OUTPUT` by `10K` and retry
the same turn. The increase only affects the current running shell process.

Inside the interactive shell, use `/config` to change runtime settings such as
model, gateway provider, API protocol, base URL, max input/output tokens, and
bash approval mode. The menu uses arrow keys and Enter, then returns to chat.

If the shell has been idle for at least 3 hours and the existing dynamic task
context is over about 10K tokens, Timem asks whether to continue the previous
task context. Choose `YES` to keep it, or `NO` to clear only the old dynamic
prompt context and start the new question cleanly. Durable memory, chat history,
scratch memory, and runtime data are not deleted by this choice.

Override the default URL only when needed:

```bash
TIMEM_BASE_URL=https://your-gateway.example/v1
```

## Runtime Capabilities

Timem ships with built-in tool manifests, but you can overlay prompt/IDL
capabilities at startup without recompiling:

```bash
export TIMEM_CAPABILITIES_DIR=/path/to/capabilities
timem
```

Directory layout:

```text
capabilities/
  tools/*.yaml
  skills/<skill_id>/skill.yaml
  skills/<skill_id>/instructions.md
```

Runtime tool manifests may add or override canonical tool names only when they
bind to an existing builtin executor such as `run_bash`, `memmgr`, `self_tool`,
or `capmgr`, or to a command script inside the overlay directory.

Command-bound tools use this manifest shape:

```yaml
kind: tool
id: my_tool
binding_type: command
binding_name: scripts/my_tool.sh
summary: My runtime tool.
description: |
  Use when this local runtime tool is appropriate.
input_properties:
  query: string
required:
  - query
example_json: |
  {
    "my_tool": {
      "query": "hello"
    }
  }
```

Runtime invokes `/bin/sh scripts/my_tool.sh` and writes one JSON object to
stdin: `{"my_tool":{...}}`. Output is captured
as an action result with bounded size and timeout.

## Runtime Data

By default, runtime data is written under the directory where you start
`timem`:

```text
data/<space>/audit/api_audit.json
data/<space>/audit/action_audit.json
data/<space>/memory/
data/<space>/memory/shell_jobs/
data/<space>/shell_history.txt
```

`api_audit.json` is a JSON document with an `events` array. `action_audit.json`
is grouped JSON for model-requested actions.

Use a fixed data root if you do not want data under the current directory:

```bash
TIMEM_DATA_DIR=/path/to/data timem
```

Env files are independent from runtime data. `timem` only reads process env, so
you can keep the private env file anywhere and load it yourself:

```bash
source /path/to/your/env
```

## Local Shell Jobs

`run_bash` runs short commands in normal mode. For long builds, tests,
package installs, or video commands, the model can request:

```json
[
  {
    "run_bash": {
      "cmd": "cargo test",
      "background": true
    }
  }
]
```

Runtime returns a process id such as `pid=12345, now keeps running in
background`. If a normal command reaches its `timeout_ms`, runtime returns
`pid=12345, timeout, but is still running` and keeps tracking that process.
When a tracked job exits, runtime adds a one-time `RUNNING_JOB_UPDATE` to the
next prompt delta. After large context compaction, runtime can also add a
`RUNNING JOB LIST` snapshot. The model can inspect or stop those jobs with
ordinary `run_bash` commands such as `ps -p <pid>` or `kill <pid>`.

`Ctrl+C` is always a cancellation key, not an exit key: while editing input it
cancels the current line, inside menus it cancels the current selection, and
while Timem is thinking it cancels the current turn. Use `Ctrl+D` or `/exit` to
leave the shell intentionally.

While Timem is thinking, you can also type an extra instruction and press Enter.
That line is added to the current turn as a `user_supplement` prompt slice, so
the next model round sees it as the newest user correction/instruction instead
of waiting for a new chat turn.

## Update and Uninstall

Update from a clone:

```bash
git pull --ff-only
./install.sh
```

Uninstall Timem binaries:

```bash
./uninstall.sh
```

Private env files and runtime data are user-managed and are not removed.

## Project Notes

- Default response protocol is XML; Markdown and JSON protocol suites remain
  available for parser parity and controlled experiments.
- Built-in capabilities are described by YAML manifests under
  `resources/capabilities/tools/` and paired Rust callbacks.
- Web UI source lives in `web_ui/timem-web`; the optional ignored
  `web_ui/vendor/assistant-ui` checkout is only a development reference.
- Please star [moliam/TimemAi](https://github.com/moliam/TimemAi).

## Contributors

TimemAi is developed by limo with assistance from Claude and Codex.
