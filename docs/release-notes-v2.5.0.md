# TimemAi 2.5.0

TimemAi 2.5 focuses on session timers, a lighter and faster Web composer,
memory-recall correctness, and portable release gates. These highlights cover
changes since the last public release, 2.4.0.

## Major features

### Session timers

- The new `timer` built-in tool lets the agent schedule named reminders; the
  host owns authoritative scheduling, and fired timers hand the recorded prompt
  to the next run as its work assignment.
- The Web timer panel creates, toggles, and deletes timers (with confirmation),
  clamps and expands long prompts, edits them inline, and renders fired turns
  with a distinct tint and badge. Terminal users get timer action rows and a
  core.timer attach summary; lifecycle rows are rendered in the user language.

### Faster, calmer Web composer

- Keystrokes stay local behind a throttled write-through so heavy history no
  longer rerenders the whole thread per key; typing stays responsive under
  20-turn x 8KB histories and submitting inside the throttle window still sends
  the complete live text.
- Unsent composer drafts persist browser-locally per Session, and typed
  keystroke counters localize behind the same throttle.
- Session lists and the chat header are denser; system settings merge
  tool-result length and context-compression threshold into one aligned card
  with dropdowns.

### Memory recall and stream visibility

- `raw_chat` history search reports real timestamps, orders newest first,
  honors after/before windows, and keeps a scoped session visible even when a
  sibling session is busier.
- `task_finished` summaries stream as an optimistic preview so the answer
  appears while the turn is closing; repair and error notices are explicit and
  visible, and the action-result heading is reserved for real tool results.
- Tool rows show pipe-delimited result size and elapsed metadata.

### Endpoint and context handling

- A warning appears when the active endpoint cannot read image attachments,
  derived from documented input modalities in the capability catalog, covering
  queued attachments, both themes, and narrow viewports.
- Opt-in restore of the last shutdown's dynamic prompt context on startup,
  degrading to history-only restore when the snapshot is unusable.
- Applying an endpoint catalog template preserves user-entered values.

## Reliability fixes

- A command child that exits without reading its stdin payload is now reported
  by its own exit status and output; fast tools no longer surface a spurious
  `command_input_failed` under scheduler jitter, while genuinely closed or
  broken stdin on a live child still fails as before.
- Failed snapshot restores degrade instead of breaking session restore.
- Browser acceptance gates are portable across remote runner hardware: the
  Chrome startup budget is 30 seconds and the typing median ceiling is 120ms
  with unchanged structural regression guards (heavy-frame ratio and bounded
  total time).

## Support boundaries

- Timer state is host-authoritative; clients render lifecycle from host
  projections rather than inferring state from event timing.
- Browser-local drafts are convenience storage, not a command outbox; command
  delivery remains one-shot over the live WebSocket.
- The typing budget is a regression guard, not a device performance guarantee.

## Upgrade

Use the documented installer to update, then restart Timem so it serves the new
embedded Web assets. Existing MEM workspaces and Sessions are retained. Run
`timem` for the authenticated local Web UI and configure the selected Session
in the browser; `timem --shell` remains the optional terminal mode.

## Validation

Release publication requires the complete local `scripts/ci.sh` gate and native
CI on Ubuntu, Windows, macOS latest, macOS 15, and macOS 15 Intel. The gate
includes workspace and Web tests, performance guards, repeated edge
regressions, release builds, and applicable real-browser, TTY, and runtime I/O
checks. Windows uses its platform-specific CI steps rather than claiming
Unix-only checks ran there.
