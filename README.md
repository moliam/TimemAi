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

- `agent_core/`: protocol loop, memory/search tools, guarded local actions.
- `timem_shell/`: terminal UI, input editor, provider HTTP adapters, audit log,
  CLI.
- `resources/static_v1.json`: static prompt used by the shell runtime.
- `docs/architecture.md`: module boundaries, turn lifecycle, runtime contracts.

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

## Provider Config

Common examples:

```bash
# Aliyun DashScope compatible mode
export TIMEM_GATEWAY_PROVIDER=aliyun
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-compatible
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
`TIMEM_API_PROTOCOL` chooses the request/response format. Supported values:

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

`TIMEM_MAX_LLM_INPUT` defaults to `100K`; `TIMEM_MAX_LLM_OUTPUT` defaults to
`10K`. Runtime asks the model to consider `prompt_shrink` when the observed
provider input tokens plus the new prompt delta estimate reaches about one
third of the input window. After that first review, the next review threshold
advances by one fifth of the window each time. If the prompt reaches 95% of the
configured input window, runtime marks shrink as required.

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
scratch notes, and runtime data are not deleted by this choice.

Override the default URL only when needed:

```bash
TIMEM_BASE_URL=https://your-gateway.example/v1
```

## Runtime Data

By default, runtime data is written under the directory where you start
`timem`:

```text
data/<space>/audit/api_audit.jsonl
data/<space>/audit/action_audit.json
data/<space>/memory/
data/<space>/memory/shell_jobs/
data/<space>/shell_history.txt
```

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

`run_bash` runs short commands in the foreground. For long builds, tests,
package installs, or video commands, the model can request:

```json
{
  "action": "run_bash",
  "intent": "Run a long local task.",
  "input": {
    "command": "cargo test",
    "background": true
  }
}
```

Runtime returns a `job_id`, output file, and status file. The model should poll
with `shell_job_status` instead of retrying the same command after a foreground
timeout.

While editing input, `Ctrl+C` cancels the current line. While Timem is thinking,
`Ctrl+C` cancels the current turn without exiting the shell.

## Install Details

One-command install:

```bash
cd ~/timemai
./install.sh
```

Timem shell currently supports macOS and Linux. Windows is not supported yet.
`install.sh` detects the OS before building:

- macOS: checks Xcode Command Line Tools and `curl`.
- Linux: checks `cc`, `make`, `curl`, and `ca-certificates`; when possible it
  installs them with `apt-get`, `dnf`, `yum`, `pacman`, or `zypper`.

If Rust/cargo is not installed, `install.sh` installs the Rust toolchain with
rustup first, then builds the release binary. Cargo 1.78+ is required because
the repository uses `Cargo.lock` v4; if an older Cargo is found and `rustup`
exists, the installer updates the stable toolchain automatically. To disable
automatic Rust install/update:

```bash
TIMEM_SHELL_SKIP_RUST_INSTALL=1 ./install.sh
```

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
