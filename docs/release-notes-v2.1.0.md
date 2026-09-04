# TimemAi 2.1.0

TimemAi 2.1.0 focuses on a smoother Web workspace, stronger cross-platform
execution, safer model transport, reliable continuation, and a much simpler
installation and update experience.

## Highlights

### Better Session organization and Web usability

- Organize Sessions into named groups, with a permanent **Unsorted** group for
  existing and uncategorized Sessions.
- Sort Session navigation and manage groups from the sidebar.
- Keep the composer docked while long model output scrolls independently.
- Keep sidebar controls reachable on smaller windows and mobile-width layouts.
- Use a cleaner default layout and updated project screenshots.

### Native command tools on every platform

- macOS and Linux use the native Bash command tool.
- Windows uses a native PowerShell command tool rather than Unix shell
  assumptions.
- Command pipelines, process lifecycle, background completion, cancellation,
  and result delivery have dedicated platform and CI coverage.

### External skills and tool discovery

- Discover compatible Claude and Codex skill directories when enabled.
- Match reusable skills by user intent and inspect their `SKILL.md` contracts.
- Enable Web tool discovery by default.
- Add runtime self-capability checks so packaged tools and their declarations
  stay consistent.

### Safer and more diagnosable model transport

- Follow same-origin HTTP redirects by default.
- Do not forward API keys or sensitive custom headers across origins.
- Support a private CA certificate chain per model endpoint.
- Bound serialized requests and response bodies.
- Distinguish DNS, connection, TLS, proxy, request-body, response-body, and
  timeout stages in user-visible failures.
- Record bounded, redacted request IDs, first-byte timing, total timing,
  response size, and redirect count for diagnosis.
- Retry transient response disconnects and temporary local-address exhaustion.

### More reliable resume and background work

- Use one confirmed continuation path for direct Session resume.
- Include Session runtime and Worker identity in the system context used for
  resumed work.
- Refresh Shell job status at the model-request boundary and preserve ordered
  completion events in prompts.
- Improve Windows MEM lock handoff and audit-retention recovery.

### Install and update with one command

The same command performs a first installation or updates an existing install.

macOS or Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/moliam/TimemAi/main/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/moliam/TimemAi/main/install-online.ps1 | iex
```

Then run:

```bash
timem
```

The installer downloads the formal `v2.1.0` Release into a temporary directory,
builds and installs `timem`, and removes the temporary source. Existing MEM
workspaces, Sessions, model settings, credentials, and user configuration are
preserved. Exit Timem before updating on Windows.

Source-checkout installations remain supported and show Git-based update
instructions; one-line installations show only the simple rerun-to-update flow.

## Compatibility notes

- Web remains the default interface; use `timem --shell` for the terminal UI.
- `timem-web` remains a compatibility alias, not a second executable.
- One running Timem Host owns a MEM at a time; use separate MEM directories for
  concurrent hosts.
- Storage upgrades are forward-compatible for this release, but downgrade in
  place is not guaranteed. Back up important MEM data before using an older
  binary.
