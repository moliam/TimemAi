# TimemAi

TimemAi is a local-first AI agent runtime with two user interfaces:

- `timem`: a native terminal UI for shell-heavy work.
- `timem-web`: a loopback-only browser UI built on assistant-ui.

Both hosts use the same Rust `agent_core`: model calls, prompt/context
management, memory, capability execution, audit logs, session history,
provider adapters, and safety boundaries are shared. The UI layer only decides
how to collect input and render structured core events.

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

## Install

```bash
git clone https://github.com/moliam/TimemAi.git
cd TimemAi
./install.sh
```

The installer builds both release hosts and installs:

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

`timem-web` binds only to `127.0.0.1`, chooses a port in `12345..=23456`
unless `--port` is set, generates a per-process access token, and opens the
authenticated local page in the default browser.

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
