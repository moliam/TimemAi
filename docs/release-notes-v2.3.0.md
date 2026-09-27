# TimemAi 2.3.0

TimemAi 2.3.0 completes the turn-finishing protocol with the memo reminder,
turns `timem attach` into a full interactive terminal client, and delivers a
large batch of model-endpoint management and Web polish.

## Highlights

### Turn finishing and the memo reminder

- A turn now ends only through the explicit `task_finished` tool (renamed from
  `turn_finished`); plain text no longer finishes a turn, and the sub_answer
  interim tool was removed in favor of one authoritative delivery.
- The `task_finished` summary is persisted as the turn's final answer and is
  rendered in the Web answer area and attach client instead of being buried in
  tool history.
- New `memo` tool: one runtime-held reminder for long-running work. Core
  publishes the authoritative state on a `core.memo` topic, the WebUI shows a
  memo indicator with hover content below the working navigation button
  (theme-neutral chip, right-opening tooltip), and a finish guard intercepts
  premature task_finished calls while a memo is still active.
- The memo is persisted beside the prompt-context snapshot as one consistency
  unit: after a restart it intentionally goes inactive and the resume notice
  tells the model to recreate it if necessary (snapshot-sourced, with a
  history-scan fallback).

### timem attach as a full interactive client

- Attach lists Host sessions and streams them live in the terminal with the
  Timem shell look (colored banner, prompt, session selector).
- Entered lines submit new turns when idle and inject as forced supplements
  while working; numeric replies answer decision prompts, `!c` cancels the
  running turn, `!s` stops the session.
- Large hello snapshots (>16MiB) no longer stall attach silently; recoverable
  socket errors are surfaced.

### Web usability and endpoint management

- Context meter: clearing the working context now resets the percentage
  immediately, with a compact clear action at the end of the ctx line.
- Stream UI shows a dim elapsed-time label beside the working trailer dot with
  hour/day tiers for long tasks.
- Chat message stream is ordered by creation time; turn_updated is published
  after sub-answers so attach always renders them.
- Model endpoints: self-describing endpoint store, import from codex and
  claude CLI configs (reasoning/vendor fields, real profile combinations,
  unreferenced provider scan), batch deletion with prompt confirmations, 300K
  and custom max-context options, and an Off option for reasoning effort.
- Debug directory browsing in a new tab with auto-close on runtime loss, and
  HTML debug files render natively.

### Core and model

- First-touch readfile path reminders and edited-file tracking with
  module-boundary hints; context compaction resets the tracking.
- Live supplements that time out during local actions force the next model
  dispatch; endpoint reasoning effort applies only to critical requests.
- context_compact checklist aligns with the runtime prompt.
- Shell resume notices carry a restart timestamp.

## Upgrade notes

- Protocol clients should send `task_finished` (not `turn_finished`) to end a
  turn; inline plain-text final answers still pass the memo guard.
- The WebUI dist is rebuilt and committed; restart the Host to load it.
