# TimemAi

TimemAi is a local-first AI agent delivered as one `timem` executable. It keeps
Sessions, model configuration, memory, tools, and work history in a local MEM
workspace while offering two interfaces:

- **Web (default):** `timem` starts the browser UI.
- **Shell:** `timem --shell` starts the terminal UI.

Both interfaces use the same Core runtime and persisted data. Installer-created
`timem-web` commands are compatibility aliases, not a second product or binary.

![Timem Web UI](docs/assets/timem-web.png)

## Install or update

Use the same command for both the first installation and future updates.

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/moliam/TimemAi/main/install.sh | bash
```

### Windows PowerShell

```powershell
irm https://raw.githubusercontent.com/moliam/TimemAi/main/install-online.ps1 | iex
```

Then start Timem:

```bash
timem
```

Your MEM workspaces, Sessions, model settings, and user configuration are
preserved when you update. On Windows, exit Timem before rerunning the command.

The installer builds the latest formal GitHub Release in a temporary directory,
installs the `timem` executable, and removes the temporary source. Rust and the
platform C/C++ build tools are required; Node.js is not required.

Developers who prefer a source checkout can clone the repository and run
`./install.sh` on macOS/Linux or `install.ps1` on Windows. Checkout installs use
Git-based update instructions; one-line installs simply tell users to rerun the
same command.

See [Install and configuration](docs/install-and-configuration.md) for
prerequisites, version pinning, custom locations, and uninstall instructions.

## Quick start

Start the local Web UI:

```bash
timem
```

It binds to `127.0.0.1`, opens the browser when possible, and does not require a
model credential merely to open the UI. Select the model name in the header to
configure the current Session's API key, protocol, endpoint, model, and token
limits.

Start the terminal interface:

```bash
timem --shell
```

Expose Web access to another machine only when needed:

```bash
timem --public
```

Public mode binds to all interfaces and prints a token-protected URL. Open the
complete URL, including `?token=...`. Put Timem behind HTTPS and suitable network
access controls when it is reachable outside a trusted network.

## Local data and Sessions

The default MEM workspace is `~/.timem/mem`. Select another workspace with an
absolute path:

```bash
timem --space /absolute/path/to/mem
```

The selected directory itself is the MEM root. It contains Sessions, memory,
audit data, configuration, capability overlays, and diagnostics. One running
Timem host owns a MEM at a time; separate MEM directories can run concurrently.
Different Sessions may use different model endpoints and working directories.

Environment variables remain available for automation and Shell-first setup:

```bash
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_BASE_URL=https://your-gateway.example/v1
export TIMEM_MODEL=...
export TIMEM_SPACE=/absolute/path/to/mem
```

Configuration precedence and provider examples are documented in
[Install and configuration](docs/install-and-configuration.md).

## What Timem provides

- Local Session history, memory, favorites, Roles, and workspace configuration.
- Session-specific model endpoints using OpenAI-compatible, OpenAI Responses, or
  Anthropic protocols.
- Built-in capabilities and optional per-Session MCP servers.
- Queued next turns, active-turn supplements, cancellation, approvals, and live
  structured work updates.
- Reconnectable browser delivery with authoritative Host snapshots and events.
- Bounded retention, audit storage, diagnostics, and command/event queues.
- A terminal interface for keyboard-oriented workflows and automation.

## Architecture

Timem follows one dependency direction:

```text
Interface ↔ Bridge ↔ Core
```

```text
applications/timem       unified composition root and `timem` executable
interfaces/shell         terminal presentation
interfaces/web           browser presentation
bridges/in_process       typed same-process communication
bridges/http_websocket   authenticated browser transport and delivery
core/session             Session, Context, Worker, and Turn orchestration
core/agent               model, prompt, capability, tool, and memory execution
core/ui_contract         UI-neutral commands, events, and projections
core/platform            reusable operating-system policy
```

Core owns business semantics and authoritative state. Bridges own communication,
ordering, reconnect, and backpressure. Interfaces own interaction and rendering.
The Application selects a mode and assembles these layers.

Read [Architecture](docs/architecture.md) for the system model and
[Semantic project layout](docs/semantic-project-layout.md) for ownership and
dependency rules.

## Documentation

Start at the [documentation index](docs/README.md).

- [Install and configuration](docs/install-and-configuration.md)
- [Architecture](docs/architecture.md)
- [Capability system](docs/capability-system.md)
- [Core/UI topic protocol](docs/core-ui-topic-protocol.md)
- [Web reliability contract](docs/web_reliability_test_matrix.md)
- [Development and validation](docs/development.md)
- [Test strategy](docs/test-strategy.md)
- [Release management](docs/release-management.md)
- [测试人员手册（中文）](docs/tester-handbook.zh-CN.md)

## Development

Read [`AGENTS.md`](AGENTS.md) and the affected module's `module_boundary.md`
before changing behavior. The authoritative full gate is:

```bash
scripts/ci.sh
```

Architecture changes are additionally checked with:

```bash
python3 scripts/architecture_guard.py --self-test
```

Web source changes must include rebuilt `interfaces/web/dist` assets. See
[Development and validation](docs/development.md) for focused checks and the
repository's delivery rules.

## Update and uninstall

Rerun the one-line command to update to the latest formal Release, or pull and
rerun the installer when using a source checkout. Existing MEM data and user
configuration are preserved. See
[Install and configuration](docs/install-and-configuration.md#update) for exact
update, version-pinning, and uninstall commands.

TimemAi is licensed under the [Apache License 2.0](LICENSE).
