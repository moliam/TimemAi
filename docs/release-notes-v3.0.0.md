# TimemAi 3.0.0

TimemAi 3.0.0 extends the prebuilt release matrix to five platforms by adding
Linux aarch64, sets the Linux baseline to glibc 2.31 (Ubuntu 20.04 LTS or
newer), turns upgrades into a one-click action inside the Web UI, and folds a
full round of protocol, robustness, and UI fixes into the unified `timem`
product.

## Five-platform release matrix

- aarch64-unknown-linux-gnu joins as the fifth prebuilt target next to
  x86_64-pc-windows-msvc, aarch64-apple-darwin, x86_64-apple-darwin, and
  x86_64-unknown-linux-gnu.
- Linux builds (x86_64 and aarch64) are produced with cargo-zigbuild against
  a glibc 2.31 baseline: they run on Ubuntu 20.04 LTS and any distribution
  shipping glibc 2.31 or newer, without requiring a container or a newer
  system.
- The packaged README-ASSETS documents the Linux requirements (glibc 2.31+,
  bash) per target; macOS and Windows packages remain free of the Linux
  section.

## In-app self upgrade

- Settings -> System has a Software upgrade card: check for updates, download
  with a progress bar, automatic integrity verification, and a restart button
  that finishes the switch to the new version.
- The updater downloads the platform's prebuilt `tar.gz` from the official
  GitHub release, verifies the published sha256 sidecar, validates the new
  binary with `--help` before swapping it in, and keeps a timestamped backup
  next to the executable. Any failure leaves the running version untouched.
- The restart is a handover, not a stop: the new process takes the same port
  and access token, and the browser session keeps working after the reload.
- MEM workspaces, Sessions, credentials, and user configuration are never
  modified by an upgrade.

## Protocol and runtime

- Admission-time runtime notes (process scopes, config notices, state
  reminders, memory prechecks) are committed before the user entry that
  closes its delta, so a first message after a restart is no longer
  sandwiched between runtime notices.
- Native mode renders the built-in tool catalog as tool names only; full
  parameter schemas arrive through the API tool parameters, and the inline
  (no-tools) mode keeps the complete catalog.
- Context reduction trailers state the trigger first and match the runtime's
  actual reinjection checklist.
- The system prompt carries an instance containment note when the runtime
  runs inside a governed cgroup/job facility so spawned work stays inside
  the managed process tree.
- Rate-limit switching consumes the latest endpoint route, comment-only SSE
  streams are no longer treated as response bodies, context application
  deduplicates work with bounded queues, and background shell jobs are
  capped at 32 concurrent.

## Web UI

- Thinking gets an inline window with a display toggle, and debug dumps
  record thought content for the most recent 50 responses.
- Statistics page and Session Insights polling dropped to a lower CPU
  profile; Insights memory shows the real working set versus reclaimable
  cache.
- Mailbox single-line rows no longer stretch the layout horizontally; the
  timer queue-timeout field is usable again; the Stop interruption banner
  survives preview throttling; the endpoint collapse control shows the
  active endpoint name on hover; light-theme popover contrast and header
  centering are fixed.

## Testing infrastructure

- Every hand-written WebSocket frame encoder in browser fixtures implements
  the 64-bit extended-length branch, eliminating 64 KiB snapshot crashes.
- Browser acceptance scripts discover Chrome through CHROME_BIN and a
  platform candidate list that includes the macOS application path.
- Model transport test fixtures are independent of ambient proxy
  environment variables; the dedicated proxy smoke keeps production proxy
  discovery.
- catalog-endpoint-e2e passes end to end after repairing its stale
  assertion chain (template-switch URL semantics, effort option count,
  property-based value assertions, and budget-scoped selectors).

## Upgrade from earlier versions

Open Settings -> System and click "Check for updates", or keep using the
root `upgrade.sh` / `upgrade-online.ps1` scripts. Extracting a newer archive
over an existing installation preserves MEM workspaces, Sessions,
credentials, and user configuration.
