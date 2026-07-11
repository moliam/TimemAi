# Timem Shell Agent

```text
  𝓣𝓲𝓶𝓮𝓶 Ai

  ████████╗██╗███╗   ███╗███████╗███╗   ███╗
  ╚══██╔══╝██║████╗ ████║██╔════╝████╗ ████║
     ██║   ██║██╔████╔██║█████╗  ██╔████╔██║
     ██║   ██║██║╚██╔╝██║██╔══╝  ██║╚██╔╝██║
     ██║   ██║██║ ╚═╝ ██║███████╗██║ ╚═╝ ██║
     ╚═╝   ╚═╝╚═╝     ╚═╝╚══════╝╚═╝     ╚═╝

  Bash   ·   Memory   ·   Time-aware
```

TimemAi is a lightweight local agent for everyday work. It combines an LLM
conversation loop, guarded Bash command execution, and local multidimensional
structured memory so it can inspect your working environment, help with files
and commands, and remember useful context across sessions.

This repository packages the standalone Rust shell agent extracted from the
Timem iOS workspace. It is intended for users who want to run and update the
agent directly from a terminal without building the iOS app.

## Layout

- `agent_core/`: protocol loop, provider wire-format adapters, memory/search
  tools, guarded local actions.
- `timem_shell/`: terminal UI, input editor, provider HTTP transport, audit
  log, CLI.
- `resources/`: system prompt, response protocol prompt suites, and capability
  manifests used by the runtime.
- `docs/architecture.md`: module boundaries, turn lifecycle, runtime contracts.
- `docs/feature-test-management.md`: feature ownership and test coverage ledger.
- `.github/workflows/ci.yml`: GitHub Actions workflow for push / pull request
  CI.

## Quick Start

Install once:

```bash
cd ~/timemai
./install.sh
```

Use `env_template` as the complete config reference. Copy it to a private env
file, then uncomment and edit only the values you need. The template uses
`export`, so a plain `source env` makes the values visible to `timem` and
`cargo run` child processes:

```bash
cp env_template env
$EDITOR env
```

For Aliyun/Qwen, the minimum config is:

```bash
export TIMEM_GATEWAY_PROVIDER=aliyun
export TIMEM_API_KEY=your_api_key_here
export TIMEM_MODEL=qwen-plus
export TIMEM_SPACE=.test_mem
```

Run:

```bash
source /path/to/your/env

timem
```

That is the normal installed path. `timem` does not load any env file
implicitly; sourcing your env file makes the config visible as process env so
the startup banner shows the actual effective values.

To run the newest source code directly from this checkout without reinstalling,
use `cargo run` from the clone root. It compiles the current files and runs the
debug binary:

```bash
cargo run -p timem_shell
```

Every env-backed setting can also be passed on the command line. See:

```bash
timem --help
```

**Command line options override process env values.**

Example without sourcing an env file:

```bash
timem \
  --data-dir data \
  --space .test_mem \
  --gateway-provider aliyun \
  --api-protocol openai-compatible \
  --model qwen-plus \
  --max-llm-input 100K \
  --bash-approval ask
```

## Update

If you installed from a git clone, update in the same clone directory:

```bash
cd ~/timemai
git pull --ff-only
./install.sh
```

This rebuilds the latest release binary and overwrites the installed
`timem-native-rs` and the `timem` command. Your private env file is
user-managed and is not touched; source it explicitly before running:

```bash
source /path/to/your/env
```

If you cloned into another directory, run the same commands there. Keep your
private env file wherever it is easiest for you to manage.

## Quality Gates

Local production gate:

```bash
scripts/ci.sh
```

This runs script syntax checks, install logic tests, feature/test contract
checks, sensitive scan, Rust formatting, full workspace tests, repeated edge
regression, performance guard, release build, real TTY smoke, and whitespace
checks.

GitHub Actions runs the same `scripts/ci.sh` gate on pushes and pull requests
for Linux and macOS. Feature coverage is tracked in
`docs/feature-test-management.md`; each release-ready feature is expected to
have normal, boundary, error, and stress/repetition coverage or an explicit
residual-risk note.

## Provider Config

Common examples:

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
important but lengthy state into scratch memory before shrinking covered
delta/slice ids. For prompt context the model can ask runtime to offload
specific delta/slice ids instead of rewriting that context itself.

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
    "action": "my_tool",
    "args": {
      "query": "hello"
    }
  }
```

Runtime invokes `/bin/sh scripts/my_tool.sh` and writes one JSON object to
stdin: `{"action":"my_tool","args":{...}}`. Output is captured
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
{
  "action": "run_bash",
  "args": {
    "cmd": "cargo test",
    "background": true
  }
}
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

## Install Details

One-command install:

```bash
cd ~/timemai
./install.sh
```

Timem shell currently supports macOS and Linux. Windows is not supported yet.
`install.sh` detects the OS before building:

- macOS: checks Xcode Command Line Tools and `curl`.
- Linux: checks `cc`, `make`, `curl`, `pkg-config`, and `ca-certificates`;
  when possible it installs them with `apt-get`, `dnf`, `yum`, `pacman`, or
  `zypper`.

If Rust/cargo is not installed, `install.sh` installs the Rust toolchain with
rustup first. Cargo 1.78+ is required because the repository uses
`Cargo.lock` v4; if an older Cargo is found and `rustup` exists, the installer
updates the stable toolchain automatically. To disable automatic Rust
install/update:

```bash
TIMEM_SHELL_SKIP_RUST_INSTALL=1 ./install.sh
```

After Rust is ready, the installer runs `cargo fetch --locked` and
`cargo build --locked -p timem_shell --release`. Cargo downloads Rust crates
from `Cargo.lock` automatically, including terminal rendering dependencies such
as `termimad`; users do not manually install those crate libraries. If this
step fails on a fresh machine, check network access to crates.io and rerun
`./install.sh`.

If a manual `cargo run` fails with `lock file version '4' was found`, update
Rust first:

```bash
rustup update stable
rustup default stable
```

The installer builds `timem_shell`, installs:

- `timem-native-rs`: release binary
- `timem`: thin wrapper that runs the binary; it does not load env files

## Development Run

Use `cargo run` when developing the shell itself or when you want to verify the
latest local source before running `./install.sh`. Like `timem`, `cargo run`
does not automatically load env files, so source your file or pass the
needed env vars in the shell:

```bash
cd ~/timemai

TIMEM_GATEWAY_PROVIDER=aliyun \
TIMEM_API_KEY=... \
TIMEM_MODEL=qwen-plus \
TIMEM_SPACE=.test_mem \
cargo run -p timem_shell
```

Custom gateway example:

```bash
TIMEM_GATEWAY_PROVIDER=custom \
TIMEM_API_PROTOCOL=anthropic \
TIMEM_BASE_URL=https://your-gateway.example/v1 \
TIMEM_API_KEY=... \
TIMEM_MODEL=aws-claude-sonnet-4-6 \
TIMEM_SPACE=.test_mem \
cargo run -p timem_shell
```

## Uninstall

Uninstall binaries:

```bash
./uninstall.sh
```

Private env files are user-managed and are not removed.

The uninstall script does not remove Rust. If Rust was installed only for Timem
shell, remove it separately with:

```bash
rustup self uninstall
```

## Test

```bash
cd ~/timemai
cargo fmt --check
cargo test -p agent_core
cargo test -p timem_shell
```

One real-provider test is intentionally ignored unless a real local key file exists.

## Contributors

TimemAi is developed by limo with assistance from Claude and Codex.
This line is added by myself(Timem).
