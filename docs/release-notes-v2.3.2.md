# TimemAi 2.3.2

TimemAi 2.3.2 makes context compaction visible while it happens, stops
stranding user input after a `task_finished` turn, and cleans up the live
stream presentation.

## Live compacting notice

- Core now emits a `core.context.compact` topic with `phase=requested` the
  moment the forced-shrink threshold is crossed, so the WebUI can show a
  "Context compacting..." state with an indeterminate meter while the model
  still has to run the compaction; the existing completion event now carries
  `phase=completed`.
- The completed notice also shows the percent-off (e.g. `62k → 18k (71% off)`)
  with the existing token breakdown.

## No more stranded supplements

- A turn that ends via `task_finished` (like natural exhaustion) now resubmits
  unconsumed user supplements as a new turn instead of leaving them recorded
  but never dispatched. Stopped/error turns remain fail-closed: the user
  drives recovery.

## Stream presentation

- The live stream keeps every dynamic activity visible (thoughts, tools, memo
  notices, compaction, supplements) until the authoritative turn end archives
  them into the Thought/Action frame.
- Collapsed tool-run groups show a bare `xN` count without success/failure
  verdicts; verdicts and exact `✓ | ✗` counts appear only when expanded.
- Memo notice rows lost their icon chip background and distinguish
  agent-deleted vs runtime-force-deleted notices with clearer wording.

## Cross-platform CI fixes (since 2.3.1)

- `tungstenite` is a normal (cross-platform) dependency of `timem attach`,
  fixing the Windows build.
- First-touch note tests canonicalize their expected paths (macOS `/var`
  symlink, Windows verbatim paths).
- Timing-sensitive supplement dispatch-timeout test is unix-gated; the
  Ctrl+C shutdown-detach timing bound is relaxed for CI scheduler jitter.

## Upgrade notes

- No breaking changes. Restart the `timem` process after upgrading.
