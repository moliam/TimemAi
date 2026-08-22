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
- `resources/reminder_tips.json`: runtime-loaded default reminder schedules, normally under `~/.local/share/timem/resources`

`TIMEM_SHELL_INSTALL_DIR` changes the binary directory. Resources follow the same prefix at `../share/timem/resources` unless `TIMEM_RESOURCES_DIR` is set explicitly. User-level `reminder_tips.json` overrides are separate and are never overwritten by installation.

Binary updates are installed with an atomic file replacement. This allows
`./install.sh` to update an installation even while an older `timem-web`
process is still running, without invalidating the executable inode used by
that process on macOS. Restart the old process to use the newly installed
version.

Release users do not need Node.js or a separate assistant-ui checkout. Node/pnpm
are only needed for frontend development.

## Recommended Start: Timem Web

Start the installed Web host with one command:

```bash
timem-web
```

The authenticated local UI opens without requiring credentials at process
startup. Click the current model name in the upper-left header and configure
the API key, model, API protocol, Base URL, and token limits for the selected
Session. Configuration is Session-owned: changing one Session does not change
another Session's endpoint or model.

Use environment variables below when supplying defaults for new Sessions,
running the terminal UI, or automating startup. They are optional for opening
and configuring Timem Web.

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

## Model Service Examples

Aliyun DashScope compatible mode:

```bash
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_BASE_URL=https://dashscope.aliyuncs.com/compatible-mode/v1
export TIMEM_MODEL=qwen-plus
export TIMEM_RESPONSE_PROTOCOL=xml
export TIMEM_MAX_LLM_INPUT=100K
export TIMEM_MAX_LLM_OUTPUT=20K
```

OpenAI:

```bash
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-responses
export TIMEM_BASE_URL=https://api.openai.com/v1
export TIMEM_MODEL=...
```

Anthropic:

```bash
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=anthropic
export TIMEM_BASE_URL=https://api.anthropic.com
export TIMEM_MODEL=...
```

Compatible or self-hosted service:

```bash
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_BASE_URL=https://your-gateway.example/v1
export TIMEM_API_KEY=...
export TIMEM_MODEL=...
```

`TIMEM_API_PROTOCOL` chooses the model API wire format:

- `openai-compatible`
- `openai-responses`
- `anthropic`

`TIMEM_TOOL_CALL_MODE` chooses `auto`, `native`, or `inline` (default `auto`).
Auto mode probes the configured gateway/model and falls back to inline when
native tool calls are unsupported. `TIMEM_PARALLEL_TOOL_CALLS` accepts `auto`,
`true`, or `false`; Timem sends the resolved parallel flag explicitly to the API.

`TIMEM_RESPONSE_PROTOCOL` chooses the inline response format parsed by the local
runtime. Supported values are `xml` and `json`; default is `xml`. Native mode
uses provider tool-call structures, does not inject this inline protocol, and
automatically uses JSON prompt serialization. The configured inline protocol is
restored if the runtime later switches back to inline mode.

## Runtime Options

Common values:

```bash
export TIMEM_SPACE=.test_mem
export TIMEM_DATA_DIR=/path/to/data
export TIMEM_BASH_APPROVAL=approve
export TIMEM_WORK_INSTRUCTIONS=silent
```

`TIMEM_WORK_INSTRUCTIONS` controls `AGENTS.md` / `CLAUDE.md` loading:

- `silent`: auto-load and notify
- `ask`: ask the host UI
- `off`: do not load

`TIMEM_BASH_APPROVAL` controls model-requested command approval:

- `ask`: prompt before risky/local command execution
- `approve`: approve by policy for the current host; this is the default when unset

## Runtime Data

New environments use a hidden data root by default:

```text
.timem_data/<space>/
  audit/api_audit.json
  audit/action_audit.json
  memory/
  sessions/
  shell_history.txt
```

If an unconfigured existing environment already has a recognizable Timem
layout under `data/` (Timem workspace, Session index, or audit files) and does
not yet have `.timem_data/`, Timem continues using it so upgrades do not hide
or split existing Sessions. An unrelated directory merely named `data` is not
treated as Timem storage. `TIMEM_DATA_DIR` always takes precedence.

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
- While the model is working, typing another question and pressing Enter queues a
  separate next turn; it does not replace the current turn’s final answer.

Web:

- Sessions can use different model/API/runtime settings.
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

Uninstall removes the binaries and installed reminder resource. It does not remove user configuration, including a user-level `reminder_tips.json` override.
If Rust was installed only for Timem, remove it separately:

```bash
rustup self uninstall
```
