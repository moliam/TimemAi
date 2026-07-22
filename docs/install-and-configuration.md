# Install and Configuration

This page keeps operational details out of the top-level README while preserving
the full setup reference.

## Install

```bash
git clone https://github.com/moliam/TimemAi.git
cd TimemAi
./install.sh
```

Timem supports macOS and Linux. Windows is not supported yet.

`install.sh` checks platform prerequisites:

- macOS: Xcode Command Line Tools and `curl`.
- Linux: `cc`, `make`, `curl`, `pkg-config`, and `ca-certificates`; when
  possible it installs missing packages through the system package manager.

If Rust/cargo is missing, the installer installs the Rust toolchain with
rustup. Cargo 1.78+ is required. To disable automatic Rust install/update:

```bash
TIMEM_SHELL_SKIP_RUST_INSTALL=1 ./install.sh
```

The installer runs:

```bash
cargo fetch --locked
cargo build --locked -p timem_shell -p timem_web --release
```

It installs:

- `timem-native-rs`: terminal release binary
- `timem`: thin wrapper for the terminal UI
- `timem-web`: local browser UI with embedded production assets

Release users do not need Node.js or a separate assistant-ui checkout. Node/pnpm
are only needed for frontend development.

## Env Files

Timem reads process environment variables. It does not load env files
implicitly.

```bash
cp env_template env
$EDITOR env
source /path/to/your/env
```

Command-line options override process env values:

```bash
timem --help
timem-web --help
```

## Provider Examples

Aliyun DashScope compatible mode:

```bash
export TIMEM_GATEWAY_PROVIDER=aliyun
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_RESPONSE_PROTOCOL=xml
export TIMEM_MAX_LLM_INPUT=100K
export TIMEM_MAX_LLM_OUTPUT=10K
```

OpenAI:

```bash
export TIMEM_GATEWAY_PROVIDER=openai
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-responses
```

Anthropic:

```bash
export TIMEM_GATEWAY_PROVIDER=anthropic
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=anthropic
```

Custom gateway:

```bash
export TIMEM_GATEWAY_PROVIDER=custom
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_BASE_URL=https://your-gateway.example/v1
export TIMEM_API_KEY=...
export TIMEM_MODEL=...
```

`TIMEM_GATEWAY_PROVIDER` chooses provider defaults. `TIMEM_API_PROTOCOL`
chooses provider wire format:

- `openai-compatible`
- `openai-responses`
- `anthropic`

`TIMEM_RESPONSE_PROTOCOL` chooses the model response format parsed by the local
runtime. Supported values are `xml`, `markdown`, and `json`; default is `xml`.

## Runtime Options

Common values:

```bash
export TIMEM_SPACE=.test_mem
export TIMEM_DATA_DIR=/path/to/data
export TIMEM_BASH_APPROVAL=ask
export TIMEM_WORK_INSTRUCTIONS=silent
```

`TIMEM_WORK_INSTRUCTIONS` controls `AGENTS.md` / `CLAUDE.md` loading:

- `silent`: auto-load and notify
- `ask`: ask the host UI
- `off`: do not load

`TIMEM_BASH_APPROVAL` controls model-requested command approval:

- `ask`: prompt before risky/local command execution
- `approve`: approve by policy for the current host

## Runtime Data

Default layout:

```text
data/<space>/
  audit/api_audit.json
  audit/action_audit.json
  memory/
  sessions/
  shell_history.txt
```

Use a fixed data root if you do not want data under the current directory:

```bash
export TIMEM_DATA_DIR=/path/to/data
export TIMEM_SPACE=my_project
```

Env files are independent from runtime data. Private env files are
user-managed and are not touched by install or uninstall scripts.

## Interactive Notes

Shell:

- `/help` lists runtime commands.
- `/config` changes runtime settings in the current process.
- `/prof` shows runtime profiling.
- `/workspace` manages workspace reference directories.
- `Ctrl+C` / `Esc` cancel the current input/menu/turn; use `/exit` or `Ctrl+D`
  to exit.
- While the model is working, typing and pressing Enter submits a supplement to
  the active turn.

Web:

- Sessions can use different provider/model/runtime settings.
- Attachments are stored under the active data space and passed to the active
  turn.
- Stop cancels all workers in the active Session; the next send starts from the
  primary worker.
- History is restored in pages so long conversations do not block the UI.

## Update

```bash
git pull --ff-only
./install.sh
```

## Uninstall

```bash
./uninstall.sh
```

If Rust was installed only for Timem, remove it separately:

```bash
rustup self uninstall
```
