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
- `timem_shell/`: terminal UI, provider HTTP adapters, audit log, CLI.
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
```

Run:

```bash
source /path/to/your/env

timem --space .test_mem --model qwen-plus
```

That is the normal installed path. `timem` does not load any env file
implicitly; sourcing your env file makes the config visible as process env so
the startup banner shows the actual effective values.

To run the newest source code directly from this checkout without reinstalling,
use `cargo run` from the clone root. It compiles the current files and runs the
debug binary:

```bash
cargo run -p timem_shell -- --space .test_mem --model qwen-plus
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
  --gateway-provider custom \
  --api-protocol anthropic \
  --base-url https://your-gateway.example/v1 \
  --model aws-claude-sonnet-4-6 \
  --max-llm-context 100K \
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
export TIMEM_MAX_LLM_CONTEXT=100K
```

```bash
# Custom internal gateway
export TIMEM_GATEWAY_PROVIDER=custom
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=anthropic
export TIMEM_BASE_URL=https://your-gateway.example/v1
export TIMEM_MAX_LLM_CONTEXT=100K
```

```bash
# OpenAI
export TIMEM_GATEWAY_PROVIDER=openai
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-compatible
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
- `anthropic`

If `TIMEM_API_PROTOCOL` is omitted, `TIMEM_GATEWAY_PROVIDER=anthropic` uses
Anthropic protocol, `TIMEM_GATEWAY_PROVIDER=custom` also defaults to Anthropic
protocol, and other providers use OpenAI-compatible chat completions.

`TIMEM_MAX_LLM_CONTEXT` defaults to `100K`. Runtime asks the model to consider
`prompt_shrink` when the observed provider input tokens plus the new prompt
delta estimate reaches about one third of this value.

Override the default URL only when needed:

```bash
TIMEM_BASE_URL=https://your-gateway.example/v1
```

## Runtime Data

By default, runtime data is written under the directory where you start
`timem`:

```text
data/<space>/api_audit.jsonl
data/<space>/memory/
data/<space>/shell_history.txt
```

Use a fixed data root if you do not want data under the current directory:

```bash
TIMEM_DATA_DIR=/path/to/data timem --space .test_mem --model qwen-plus
```

Env files are independent from runtime data. `timem` only reads process env, so
you can keep the private env file anywhere and load it yourself:

```bash
source /path/to/your/env
```

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
cargo run -p timem_shell -- --space .test_mem --model qwen-plus
```

Custom example:

```bash
TIMEM_GATEWAY_PROVIDER=custom \
TIMEM_API_PROTOCOL=anthropic \
TIMEM_BASE_URL=https://your-gateway.example/v1 \
TIMEM_API_KEY=... \
cargo run -p timem_shell -- --space .test_mem --model aws-claude-sonnet-4-6
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
