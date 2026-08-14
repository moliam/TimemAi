# TimemAi 1.1.0

TimemAi 1.1.0 makes the quality-hardened Timem Web experience the recommended
way to use TimemAi: install it, run one command, and configure each Session in
the browser.

## Highlights

- **Simple start:** run `timem-web`. The authenticated local page opens without
  requiring an environment file or model credential up front.
- **Configure in Web:** click the current model name to set the selected
  Session's API key, model, API protocol, and Base URL. Each Session can keep
  its own model endpoint and settings.
- **Quality-hardened delivery:** browser mutations use durable command IDs and
  explicit acknowledgements; authoritative events use ordered, replayable
  cursors. Disconnects, duplicate clicks, reordered acknowledgements, and
  multiple browser tabs no longer lose or double-apply work.
- **Concurrent Sessions:** multiple Sessions can work and recover in parallel
  while task, supplement, cancellation, final-answer, queue, and event state
  remain isolated.
- **Performance and security:** long-running event storage is bounded with safe
  snapshot recovery, browser recovery records are size-bounded, credentials
  stay out of durable browser queues, CSP is stricter, and normal production
  compilation is warning-free.

## Verification

The release is gated by the repository's complete production CI suite on Linux
and macOS: Rust formatting and Clippy, workspace and Web tests, deterministic
concurrency/restart/failure-injection regressions, frontend type checks and
production build reproducibility, dependency-license and sensitive-data scans,
performance and repeated-edge tests, release builds, cross-host resume, and
pseudo-TTY smoke tests. Desktop and 390px Timem Web layouts were also reviewed
in a real browser for overflow, settings accessibility, focus restoration, and
console errors.
