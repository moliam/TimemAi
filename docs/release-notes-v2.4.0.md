# TimemAi 2.4.0

TimemAi 2.4 focuses on long-running work, a more responsive Web and terminal
experience, and reliable cross-platform command execution. These highlights
cover changes since the last public release, 2.3.2, including the previously
unpublished 2.3.3 improvements.

## Major features

### Long-running work and context management

- Manually compress context from the Web context menu, including while a turn
  is busy, with visible pending and completion states.
- Configure and persist the automatic context-compression threshold. Compaction
  retains explicitly selected exchanges and a replacement summary, preserves
  current tool definitions, and warns about repeatedly ineffective reductions.
- Preserve complete dynamic prompt context across restore and handoff. Accepted
  supplements that were not consumed by a completed turn move into a new turn
  rather than being stranded or attributed to the wrong response.
- Surface runtime resource and disk-pressure observations with gated sampling
  and bounded diagnostic output.

### Web and terminal workflow

- Connect `timem attach` to multiple local runtime instances and select Sessions
  in the streaming terminal interface; recover interactively from a missing
  working directory.
- Restore recent Sessions first and publish them progressively, so users can
  open available Sessions while older history continues loading.
- Share selected model endpoint settings through a dedicated import/export
  dialog, with advanced and personal settings explicitly opt-in.
- Improve live tool activity, elapsed time, model-request counts, context
  compression status, and history identity handling.

### Model and tool execution

- Persist native tool/function-call capability negotiation by endpoint/model
  identity and use provider-aware protocol admission. Transport or authentication
  failures are not treated as proof that native tools are unsupported.
- Strengthen managed command lifecycle handling and background exit reporting.
  Large stdin payloads and process waiting now share one execution deadline;
  output is drained concurrently, and input failures cannot silently become
  successful actions.

## Performance improvements

- Serve content-hashed Web assets with long-lived immutable caching and
  build-time gzip compression, while keeping the HTML shell revalidated for
  upgrades.
- Load heavier Markdown rendering plugins lazily rather than placing all of
  them on the initial rendering path.
- Bound runtime observation, history/debug, and terminal-rendering hot paths;
  keep large endpoint-share input out of per-keystroke React state updates.
- Batch browser events in order with a pending-only timer fallback when animation
  frames are suspended, preventing snapshots and replies from stalling.
- Reuse healthy native HTTP connections; retire failed or interrupted transports
  so unread work is not retained in an idle cached runtime.

These are implementation-level improvements backed by regression and performance
checks, not a claim of a universal percentage speedup or benchmark score.

## Reliability and platform fixes

- Bound post-exit stdout/stderr collection even when a descendant holds pipes
  open; retain bounded timeout and input-failure diagnostics.
- Correct macOS process-group liveness and cancellation escalation, and use
  caller-owned filesystem sampling buffers with real-volume filtering.
- Verify nonblocking stdin backpressure, partial writes, exact payload delivery,
  and EOF on native Windows, Linux, and macOS runners.
- Fix suspended-frame outline geometry and collision-prone restored history IDs.
- Remove the standalone Terminal-Bench adapters, campaign scripts, and historical
  evaluation configuration from the product repository. Product tests and
  performance gates remain in place.

## Support boundaries

- macOS cleanup covers the direct child and descendants that remain in its
  managed process group. Processes deliberately escaping through `setsid`,
  changing groups, or delegating to external services are not guaranteed to be
  cleaned up. Bounded pipe capture is not proof that escaped processes ended.
- Linux exact descendant ownership requires an available delegated cgroup-v2
  subtree; otherwise command execution uses the documented process-group fallback.
- Command input/wait deadlines do not promise a hard wall-clock bound over every
  OS spawn, serialization, and cleanup operation.

## Upgrade

Use the documented installer to update, then restart Timem when convenient so
it serves the new embedded Web assets. Existing MEM workspaces and Sessions are
retained. Run `timem` for the authenticated local Web UI and configure the selected
Session in the browser; `timem --shell` remains the optional terminal mode.

Custom integrations using the context tool must follow the current
`context_compress` schema (`summary`, optional `keep`, optional `offload`) rather
than older discard-based arguments. The removed evaluation harness is still
available in earlier Git history.

## Validation

Release publication requires the complete local `scripts/ci.sh` gate and native
CI on Ubuntu, Windows, macOS latest, macOS 15, and macOS 15 Intel. The gate includes
workspace and Web tests, performance guards, repeated edge regressions, release
builds, and applicable real-browser, TTY, and runtime I/O checks. Windows uses its
platform-specific CI steps rather than claiming Unix-only checks ran there.
