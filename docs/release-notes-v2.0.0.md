# TimemAi 2.0.0

TimemAi 2.0.0 establishes a unified local AI-agent product architecture,
expands native cross-platform delivery, and strengthens Host-owned execution for
work that must continue independently of the browser.

## Highlights

### One Timem application

- `timem` is the single executable and product entry point.
- Web is the default interface; `timem --shell` starts the terminal interface.
- Web and Shell share the same Core runtime, Sessions, memory, tools, model
  configuration, and local MEM workspace.
- Installer-provided `timem-web` commands remain compatibility aliases rather
  than a separate runtime.

### Clear system boundaries

The 2.0 codebase follows `Interface ↔ Bridge ↔ Core`:

- Interfaces own browser and terminal interaction.
- Bridges own typed in-process calls and HTTP/WebSocket communication.
- Core owns Agent execution, Session orchestration, authoritative state,
  capabilities, memory, persistence, and platform policy.
- The Timem Application assembles the product without duplicating Core
  semantics.

This structure keeps browser delivery concerns separate from Session and Turn
behavior and provides one reusable path for same-process Rust interfaces.

### Host-owned work and reconnect behavior

- Accepted queued tasks are durably ordered by the live Host rather than a
  browser-local completion loop.
- Work continues while the browser is locked, disconnected, refreshed, or
  closed, as long as the Timem Host remains running.
- Browser command acknowledgements remain transport control; authoritative
  snapshots, projections, and semantic events determine visible business state.
- Reconnect and sequence-gap recovery use Host snapshots and bounded delivery
  state without a persistent browser command outbox.

### Native platform delivery

- Timem includes platform-aware storage, command execution, process lifecycle,
  browser launch, terminal handling, and Web-host behavior for macOS, Linux, and
  Windows.
- Windows installation and uninstall use PowerShell scripts and a user-level
  installation path without requiring an administrator shell.
- Platform-specific behavior is covered by native CI and host-scoped tests.

### Web and runtime improvements

- Web route composition and reusable WebSocket transport are owned by the
  HTTP/WebSocket Bridge.
- Session and MEM switching preserve authoritative Host behavior and provide
  explicit restart-workspace decisions where required.
- Interface response preferences can be supplied to Core without moving prompt
  or response semantics into the UI.
- Debug runtime tracing identifies Turn enqueue, Core event consumption, model
  request delivery, and prompt-persistence latency while keeping timing evidence
  distinct from causal claims.

## Install or upgrade

macOS or Linux:

```bash
git pull --ff-only
./install.sh
timem
```

Windows PowerShell:

```powershell
git pull --ff-only
powershell -ExecutionPolicy Bypass -File .\install.ps1
timem
```

Existing MEM workspaces, Sessions, credentials, and user configuration are
preserved by the installers. Restart running Timem processes after upgrading so
they use the new executable.

## Compatibility notes

- The selected `--space` or `TIMEM_SPACE` directory is the MEM root itself.
- Only one running Timem Host may own a MEM at a time; separate MEM directories
  may run concurrently.
- `timem-web` remains available through installer compatibility shims, but new
  usage and documentation should invoke `timem`.
- Downgrade-in-place is not guaranteed for storage formats introduced or updated
  by newer releases. Back up the MEM before running an older binary.
