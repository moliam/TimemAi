# TimemAi

TimemAi is a local-first AI agent delivered as one `timem` executable with two interfaces:

- **Web (default, recommended):** run `timem` for the browser UI, including sessions, configuration, chat history, tools, and live work status.
- **Shell (optional):** run `timem --shell` for terminal-heavy work.

Both modes use the same local runtime, memory, session history, tools, and model service configuration. Installers may also provide `timem-web` as a compatibility alias to `timem`; it is not a second executable.

## Development Architecture

The source tree follows `Interface ↔ Bridge ↔ Core`. The unified product is
assembled under `applications/timem/`; terminal and browser presentation live
under `interfaces/`; direct Rust calls and HTTP/WebSocket delivery live under
`bridges/`; reusable semantics live under `core/`.

```text
applications/timem        product composition and the `timem` binary
interfaces/shell          terminal presentation and interaction
interfaces/web            browser presentation and interaction
bridges/in_process        typed zero-transport API for all same-process Rust Interfaces
bridges/http_websocket    reconnectable browser delivery
core/session              Session/Context/Worker orchestration
core/agent                model, prompt, capability, memory, and Turn execution
core/ui_contract          UI-neutral commands, events, and projections
core/platform             reusable OS policy
```

The Cargo package name `timem_web` remains for command compatibility; there is
no top-level `timem_web/` source root. Do not create placeholder desktop, FFI,
or IPC directories. A future same-process Rust Interface reuses
`bridges/in_process`; other Bridges are added only with a real consumer and
executable behavior.

Before contributing, read [`AGENTS.md`](AGENTS.md),
[`docs/architecture.md`](docs/architecture.md), and
[`docs/semantic-project-layout.md`](docs/semantic-project-layout.md). Architecture
changes are enforced by `python3 scripts/architecture_guard.py`.

## Install

macOS/Linux:

```bash
git clone https://github.com/moliam/TimemAi.git
cd TimemAi
./install.sh
```

Windows PowerShell (delivery adapted; native revalidation is still required):

```powershell
git clone https://github.com/moliam/TimemAi.git
cd TimemAi
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

The Windows installer uses `%LOCALAPPDATA%\TimemAi\bin` by default and can add
that directory to the current user's PATH without administrator privileges. It
requires stable Rust for `x86_64-pc-windows-msvc` and Microsoft Visual C++ x64
Build Tools.

The installer builds and installs one `timem` executable. On macOS/Linux it also creates a `timem-web` symlink, and on Windows a forwarding `timem-web.cmd`, for compatibility. Cargo downloads Rust crates automatically during the build. The released Web bundle is already included;
Node.js is only needed when developing the Web frontend.

## Quick Start — Timem Web (Recommended)

After installation, the shortest local start is:

```bash
timem
```

To open Timem Web from another machine, enable public listening:

```bash
timem --public
```

`--public` binds to all network interfaces and prints a token-protected URL;
it does not remove authentication. Open the complete URL, including
`?token=...`. Use HTTPS and appropriate network access controls when exposing
Timem Web beyond a trusted network.

With the local command, Timem Web binds only to `127.0.0.1` and opens the
page automatically without an access token. No environment file or model
credential is required just to start the UI. In
the page, click the current model name at the top left, then configure the API
key, model, API protocol, and Base URL for that Session. Send a message when
the Session is configured.

Each Session keeps its own model service configuration, so different Sessions
can use different models or endpoints without changing the others.

### Reminder tips configuration

The default schedules live in `resources/reminder_tips.json`. `install.sh` installs that file under the installation prefix, normally `~/.local/share/timem/resources/reminder_tips.json`, and both Web and Shell modes load it at startup.

To customize tips globally for the current user, create `reminder_tips.json` in one of these locations. A user file takes precedence over the installed resource and is never overwritten or removed by install/uninstall:

- macOS: `~/Library/Application Support/TimemAi/reminder_tips.json`
- Linux: `${XDG_CONFIG_HOME:-~/.config}/timem/reminder_tips.json`
- Windows: `%APPDATA%\TimemAi\reminder_tips.json`
- Config override: set `TIMEM_CONFIG_DIR` to the directory containing the user file.
- Resource override: set `TIMEM_RESOURCES_DIR` to an alternate resources directory.

A schedule may trigger by active minutes or completed model rounds; when `NONE` is randomly selected, that period is consumed without adding anything to the prompt. Project-local `.timem_data` directories never hold this program configuration. Restart Timem after editing either the user override or installed resource.

```json
{
  "schedules": [
    {
      "every_minutes": 10,
      "tips": ["TIPS: Review the goal.", "NONE"]
    },
    {
      "every_rounds": 8,
      "tips": ["TIPS: Check the deduction chain.", "NONE"]
    }
  ]
}
```

Each schedule must set exactly one of `every_minutes` or `every_rounds` to a positive integer and provide a non-empty `tips` list. Invalid user or resource configuration is reported as a warning and safely falls through to the next valid source or the embedded safety fallback; it never prevents Timem from starting.

## Optional Environment Configuration

The Web UI is the recommended place to configure each Session. Environment
variables remain useful for terminal-first use, automation, or initial defaults
for newly created Sessions.

Create a private environment file:

```bash
cp env_template env
$EDITOR env
source ./env
```

Minimum Aliyun-compatible configuration:

```bash
export TIMEM_API_KEY=your_api_key_here
export TIMEM_MODEL=qwen-plus
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_BASE_URL=https://dashscope.aliyuncs.com/compatible-mode/v1
export TIMEM_SPACE=/absolute/path/to/mem
```

The environment file can be stored elsewhere:

```bash
source /path/to/your/env
```

MEM data defaults to `~/.timem/mem`; Timem creates that directory on first
startup. To use another MEM, pass an absolute directory path through
`--space /absolute/path/to/mem` or `TIMEM_SPACE`. Relative `--space` paths are
rejected. A MEM can hold many Sessions: each Session keeps its own metadata and
history files and uses a Session-scoped lock, so unrelated Sessions can read,
write, and run concurrently without rewriting one global Session index.

Use `timem --help` for Web startup options or `timem --shell --help` for Shell options. After the first successful configuration, Timem caches the effective
runtime environment with the local Session, so later starts can resume without
re-entering it. Runtime configuration changes update that cache. Command-line
options always override cached values.

## Run Shell (Optional)

```bash
timem --shell
```

The `source ./env` step is needed for the initial configuration or when you
intentionally select a different MEM workspace. The selected Session
restores its cached model service settings on later starts.

Example terminal session:

```text
┏━ Thought / Action ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ · Checking the project status                             ┃
┃   └─ [✔] git status --short                               ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
Timem  Done. The working tree is clean.
You ❯❯ _
```

Common controls:

- `/help`: show interactive commands.
- `/config`: change runtime settings.
- `/workspace`: manage the working directory.
- `/prof`: inspect runtime profiling.
- `Ctrl+C` or `Esc`: cancel the current input, menu, or thinking turn.
- `Ctrl+D` or `/exit`: exit the shell.

While the model is working, type another question and press Enter to queue it as
a separate next turn. The current final answer stays visible before the queued turn starts.

## Timem Web Details

The Web UI provides session switching, Markdown rendering, live work updates,
attachments, runtime status, context usage, persistent Session groups, a
MEM-wide Role library, and per-session MCP tools in the browser. Create and
group reusable Roles, then select one or more for the next message to apply
specialized working methods to that task. Session groups can be created, renamed,
reordered, collapsed, or deleted. Drag a Session by its handle into another group
or **Unsorted**; deleting a group leaves its Sessions intact under **Unsorted**. Open the plug control in the header to add a local stdio, remote
Streamable HTTP, or legacy SSE MCP server and choose which Sessions may use it.
New Sessions start with no MCP servers selected, so MCP access is enabled
explicitly per Session. Timem Web can start without a model API key so configuration remains available
in the browser. Click the current model name to edit the selected Session. A
Session must have a valid API key before Send can start model work; the New
Session dialog also accepts Session-specific model service settings and caches
them for later starts.

![Timem Web UI](docs/assets/timem-web.png)

Local mode binds to `127.0.0.1` and opens the page automatically without an
access token when a local graphical session is available:

```bash
timem
```

On SSH or headless Linux, the server prints the URL without trying to open a
browser. Open that URL on a machine with a browser.

Public mode binds to all interfaces and prints a token-protected URL:

```bash
timem --public
```

Open the complete URL printed in the terminal, including `?token=...`, from
your local browser. With the default MEM directory (`~/.timem/mem`), automatic
selection tries port `13764` first and falls back to another port in the
supported range if it is occupied. A custom MEM keeps the rotating automatic
selection strategy. An explicit `--port` always takes priority. To choose a
fixed port or advertised host:

```bash
timem --public --port 20699 --public-host 10.125.112.83
```

Public mode does not open a browser on the server. HTTP access may show a
browser "Not secure" warning because it uses plain HTTP; the access token is
still required. For production exposure, place Timem behind HTTPS and an
appropriate network access control layer.

A MEM directory is the complete Timem workspace. It contains Sessions, memory,
audit data, workspace configuration, capability overlays, and Web lifecycle
diagnostics. The default is `~/.timem/mem`; select another absolute directory
with `--space` or `TIMEM_SPACE`. One Timem Web or Shell host may own a MEM at a
time, while different MEM directories can run concurrently. Timem Web keeps
bounded, low-overhead lifecycle diagnostics under
`<MEM>/diagnostics/timem-web/`. Separate completed runs use independent records,
so one run cannot erase another run's crash evidence. The recorder captures process milestones and the
actual graceful-shutdown trigger without storing prompts, replies, API keys, or
HTTP header values. After an unexpected exit, see
[Install and configuration](docs/install-and-configuration.md#timem-web-lifecycle-diagnostics)
for the files to inspect.

## More Documentation

- [Architecture](docs/architecture.md)
- [Install and configuration](docs/install-and-configuration.md)
- [Core/UI topic protocol](docs/core-ui-topic-protocol.md)
- [Web delivery reliability contract](docs/web_reliability_test_matrix.md)
- [Web performance tracing](docs/web-performance-tracing.md)
- [Capability system](docs/capability-system.md)
- [Test strategy](docs/test-strategy.md)
- [测试人员手册（中文）](docs/tester-handbook.zh-CN.md)
- [Feature and test management](docs/feature-test-management.md)
- [Release management](docs/release-management.md)
- [Release smoke checklist](docs/manual-release-smoke.md)
- [TimemAi 1.3.0 release notes](docs/release-notes-v1.3.0.md)
- [TimemAi 1.2.0 release notes](docs/release-notes-v1.2.0.md)
- [TimemAi 1.1.3 release notes](docs/release-notes-v1.1.3.md)
- [TimemAi 1.1.2 release notes](docs/release-notes-v1.1.2.md)
- [TimemAi 1.1.1 release notes](docs/release-notes-v1.1.1.md)
- [TimemAi 1.1.0 release notes](docs/release-notes-v1.1.0.md)

## Update and Uninstall

The installers support both first installation and in-place upgrade. They install the current source checkout; they do not fetch newer source automatically.

macOS/Linux upgrade from any older checkout-based installation:

```bash
cd /path/to/TimemAi
git pull --ff-only
./install.sh
```

This atomically replaces the old `timem` command, removes the former independent `timem-native-rs`/`timem-shell` artifacts, and replaces an old independent `timem-web` binary with a compatibility symlink to the unified `timem`. MEM data, Sessions, credentials, and user configuration are preserved. Restart running Timem processes after upgrading.

On Windows, first exit running Timem processes so Windows file locks do not block replacement, then run:

```powershell
cd C:\path\to\TimemAi
git pull --ff-only
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

The Windows upgrade removes legacy independent `.exe` files and installs `timem-web.cmd` as a forwarding compatibility shim.

macOS/Linux:

```bash
./uninstall.sh
```

Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\uninstall.ps1
```

Runtime data and private environment files are user-managed and are not
removed by uninstall.

Please star [moliam/TimemAi](https://github.com/moliam/TimemAi).
