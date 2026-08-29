# Changelog

## [Unreleased]

### Added

- Add Web performance tracing under the existing `--debug` switch in the shared PID diagnostic directory with command-correlated browser/server stages, fixed 20 MiB in-place wrapping, browser hot-path microbenchmarks, and a reproducible diagnosis guide.

### Changed

- Render Web Chat sub-answers newest-first inside an independent compact disclosure matching Thought/Action, with a restrained green-tinted answer surface. When the final answer arrives, Chat and Thought/Action automatically collapse once and remain under explicit user control afterward.

- Replace the Web Thought/Action process panel shadow with a fine theme-aware border while retaining progressive top/bottom scroll-edge fades.

- Reworked Web Session switching around a two-entry LRU of presentation-only
  timeline panes. Warm A/B switching now reuses parsed Markdown and mounted Turn
  DOM without duplicating composer or queue side effects; inactive panes freeze
  their last visible snapshot, completed Turns retain stable layout containment,
  and navigation geometry work is coalesced to one pass per animation frame.

- Replaced the durable Web semantic-event replay journal with an in-memory,
  linearized delivery stream. Connected event publication no longer performs
  journal reads, rewrites, or fsyncs; reconnects, sequence gaps, and lag recover
  from a full authoritative snapshot. Browser command durability, deduplication,
  Core delivery state, restart recovery, per-MEM process ownership, and
  multi-tab convergence remain intact.

### Fixed

- Keep long-chat scrolling stable without giving up warm Session switching.
  Variable-height Turn cards no longer use intrinsic-height `content-visibility`;
  ordinary chat scroll frames derive user-message and Markdown-outline state
  from cached offsets instead of scanning DOM geometry; outline collision
  updates use visibility/intersection invalidation; hidden or distant cached
  answers do not retain active scroll listeners. Content, size, outline, and
  Session-activation changes still trigger bounded geometry refreshes so scroll
  position restoration, message navigation, current-section tracking, and
  outline avoidance remain accurate.

- Refine the Web long-answer Markdown outline: its collapsed control is now a compact bookmark-only tab placed clear of the left sidebar, its disclosure arrow and drawer styling are more precise, and the user-message bubble navigation dynamically preserves distance from the reading column while degrading through no, partial, and full outline overlap only as horizontal space becomes constrained.
- Keep large Web chat-search result sets in a bounded scroll area and prevent result rows from shrinking into unreadable stripes.
- Recover provider `length`/`max_tokens` responses whose native tool arguments end mid-JSON: retain the received response and argument fragment in the repair context, then request one smaller complete step and continue iteratively instead of surfacing `invalid_tool_call ... EOF` or expanding the output limit first.
- Keep Web Session Worker disclosure behind the host `--debug` option so normal launches do not expose an unfinished capability; debug launches retain the attached Worker hierarchy panel. Session names start at the left edge, and hover/focus controls remain on the right.
- Reduce Web Session-list row height and vertical padding for a denser sidebar
  while preserving status indicators, disclosure controls, and touch behavior.
- Speed up Web Memory settings temporary-data discovery by filtering directory
  entries before reading file metadata, retaining only the largest 100
  candidates during traversal, and reusing the result when reopening the same
  MEM instead of automatically rescanning it each time.
- Fixed Web Markdown math normalization so multiline `\[...\]` and custom
  display formulas remain isolated block-math nodes instead of swallowing the
  following Markdown, and number-leading inline formulas such as `\(2P\)`
  are no longer mistaken for currency. Display formulas also receive adaptive
  vertical breathing room so multiline fractions, boxes, and aligned equations
  render at their natural height without clipping.
- Make local `timem-web` launches use only the loopback port without an access token; `--public` continues to require a rotating token for browser, API, upload, and WebSocket access.

- Render Web polling actions as a dedicated `Poll` activity with a clock icon, live `mm:ss` elapsed time, and the polled command on a second line.

- Clarify polling exit-code semantics while preserving the standard Bash result
  structure: `status` describes the polling lifecycle and `exit_code` is the
  last `loop_cmd` exit code; a waited task's own exit code must be read from its
  status file, wait result, or remote API.

- Let non-empty `TIMEM_*` process environment values loaded with `source env`
  override restored Shell Session configuration, while keeping command-line
  options highest priority and retaining cached values when the process
  environment only contains an empty value.

## [1.2.0] - 2026-08-26

### Highlights

- Added adaptive provider-native tool calling for OpenAI-compatible Chat,
  OpenAI Responses, and Anthropic APIs. The default `auto` mode probes support,
  preserves ordered tool-call/result history, supports negotiated parallel
  calls, and falls back to the existing inline XML/JSON protocol when needed.
- Added a durable MEM-wide Role library for reusable working methodologies.
  Roles can be created, edited, grouped, reordered, and combined per message so
  users can assign specialized ways of working across Sessions and tasks.

### Changed

- Refined the Web composer and Session navigation: Session names are smaller,
  cwd is shown only below the composer text area, and a subtle divider separates
  the text area from the cwd/actions row.
- Polished the sidebar brand alignment, moved Session worker disclosure into
  the left gutter on row hover, and made expanded worker names use a fixed-width
  hierarchy rail so deep parent/child nesting no longer consumes the name area.
  Worker labels remain normal weight, and endpoint model summaries use the
  shared model glyph.
- Added a centralized `agent_core::os` abstraction with initial macOS and Linux
  implementations for host/version detection, shell paths, default config
  directories, browser and terminal launch commands, and process-group
  lifecycle operations. The model-facing `run_bash` description now dynamically
  includes the detected host OS and `/bin/bash` versions.

### Fixed

- Stream large model request bodies through curl stdin instead of embedding them
  in curl configuration lines, removing the approximately 100 KiB failure mode
  for long multi-turn contexts while preserving cancellation and inactivity
  timeout behavior.

### Removed

- Removed unused capability `output_schema` declarations and parser state.
  Tool manifests now describe fresh model submissions with `input_schema` and
  describe executor-owned evidence with `prompt_result`.

## [1.1.3] - 2026-08-15

### Added

- Use the existing Timem logo as the browser tab icon in both development and
  the production Web bundle embedded in `timem-web`.

## [1.1.2] - 2026-08-14

### Fixed

- Install release binaries by atomic replacement so rerunning `./install.sh`
  while `timem-web` is active does not leave the installed macOS executable
  terminating immediately with `SIGKILL`.
- Keep `timem-web` startable when recoverable replay, command-dedup, MCP, or
  Session-index state is malformed. The original bytes are retained in private
  diagnostic backups before a safe cache reset or valid-record repair.
- Reuse an authenticated URL across repeated page opens for the lifetime of one
  Host, and continuously verify clean shutdown plus immediate restart in CI.
- Run performance thresholds against optimized release binaries and
  synchronize process-cancellation tests on actual child startup so public
  macOS and Linux CI results do not depend on shared-runner scheduling speed.

All notable changes to TimemAi are tracked here. This project follows a
pragmatic Keep a Changelog style: newest changes first, with release sections
for tagged versions and an `Unreleased` section for work not yet tagged.

## [1.1.1] - 2026-08-14

### Fixed

- Serialized Session index read-modify-write operations across threads and
  processes, and replaced the index atomically after a durable write. Concurrent
  Web Sessions can no longer observe a truncated JSONL index or overwrite one
  another's Session record.

## [1.1.0] - 2026-08-14

### Added

- Added durable, correlated browser command delivery with stable command IDs,
  explicit accepted/committed/rejected acknowledgements, reconnect replay, and
  per-Session ordering across browser tabs and sockets.
- Added a sequenced semantic event journal with cursor replay, bounded
  compaction, snapshot fallback for expired cursors, crash-tail repair, and
  isolated concurrent restore batches for multiple working Sessions.
- Added durable queued-message editing, deletion, reordering, immediate send,
  automatic next-message dispatch, and cross-tab synchronization without
  persisting API keys or MCP secrets.

### Changed

- Made Timem Web the recommended interface: install, run `timem-web`, then
  click the current model name to configure the selected Session's API key,
  model, protocol, and Base URL in the browser.
- Tightened the Web content-security policy, bounded recovered browser storage
  and long-running event journals, and kept the production Rust build free of
  warnings.

### Fixed

- Closed races around final answers, late supplements, cancellation, queue
  mutation, Session deletion, mem switching, command acknowledgement loss,
  reconnect gaps, and multi-Session startup recovery.
- Prevented raw and sequenced authoritative events from being reduced twice,
  and preserved accepted user work across WebSocket disconnects and Host
  restarts.

## [1.0.4] - 2026-08-13

### Fixed

- Unified the Rust workspace, embedded Web host, and frontend package version
  metadata so every Timem Web release reports one authoritative version.
- Added a release gate that rejects mismatched Cargo, Cargo.lock, and frontend
  package versions before tests or packaging begin.

## [1.0.3] - 2026-08-13

- New environments now store runtime data under hidden `.timem_data/` by
  default, while unconfigured installations with an existing legacy `data/`
  directory continue using it for upgrade compatibility.

- Stabilized long-session scrolling by removing competing viewport auto-scroll controllers and disabling native anchoring on the managed conversation viewport.

- Improved final-answer Markdown readability with theme-specific semantic colors for inline code and syntax highlighting; key formatted-text color pairs are now guarded by WCAG AA contrast tests.
### Added

- Added Session-scoped MCP tool management for local stdio, Streamable HTTP,
  and legacy SSE servers. Enabled tools enter the same prompt, validation,
  execution, topic, and audit pipeline as built-in capabilities.
- Added Web configuration for replacing or clearing an idle Session's API key.
  Opening settings loads an existing value through an authenticated,
  request-scoped reply and displays it masked by default; the eye control
  toggles visibility without putting credentials in snapshots, broadcasts,
  prompts, or audit.

### Changed

- Replaced model-facing XML action JSON/CDATA payloads with XML-native
  `<actions>`, explicit `<parallel>` groups, tool-id elements, and
  schema-typed argument attributes/children. Tool prompts now expose concise
  nested types without JSON-specific wording; runtime conversion covers
  nullable/union values, tuples, dynamic object fields, large integers, XML
  entities, and literal CDATA. Invalid batches execute nothing, unsafe XML
  constructs are rejected, and action trees have explicit depth/size bounds.
  The runtime retains legacy parsing for existing Session context while new
  prompts teach only the native form.
- Reduced decorative emoji in ordinary model headings, status updates, test
  results, and confirmations while retaining semantically useful emoji.
- Kept Web startup, Session restore, and unrelated agent work independent from
  unavailable MCP servers by moving discovery to bounded background work.
- Kept long conversations and event bursts responsive with an ordered bounded
  browser-command queue, frame-budgeted event delivery, and bounded visible
  turn rendering.
- Cached effective runtime settings with each Session while preserving explicit
  command-line overrides.
- Simplified model-service settings around model, API protocol, and base URL;
  raised the default output budget from `10K` to `20K`; and kept unrelated
  unsaved field drafts intact when one setting is applied.
- Refined the Web settings, cwd, and MCP controls with standard settings and
  secret-visibility icons, quieter workspace context, and clearer action spacing.
- Moved context usage beside the active model as a short progress meter with
  compact `percent/limit` text, and removed the separate diagnostic Activity
  panel while retaining semantic task events and visible host errors.
- Removed the separate service identity/configuration dimension. Web, Shell,
  Session persistence, topics, audit, and profiling now identify model calls
  directly by model, API protocol, and endpoint, and display the model name
  without internal routing prefixes in Web
  headers, Session navigation, and Shell startup/thinking/final status lines.

### Security

- Restricted persisted Session credentials to owner-only storage and kept API
  keys, MCP headers, and MCP environment secrets out of browser projections,
  prompts, topics, and audit output.
- Kept API-key and MCP-secret reveal replies scoped to the requesting
  authenticated WebSocket and cleared browser-held plaintext on panel close,
  Session change, reconnect, mem switch, or save.

## [1.0.2] - 2026-07-24

### Fixed

- Kept Web tool details open by default so active and completed Bash/tool
  commands do not collapse themselves while a task is running.
- Fixed Web session creation defaults so Runtime Settings changes are applied
  to newly created sessions, and dialog-level runtime overrides are reflected
  in the created Session's runtime profile.
- Canonicalized the default Web workspace before validation to avoid registered
  workspace mismatches on equivalent paths.

## [1.0.1] - 2026-07-22

### Fixed

- Fixed Web Thought/Action tool rows so expandable Bash/tool entries no longer
  render a blank chevron-only line or duplicate collapse controls before the
  command detail.
- Increased the live Web working/ToolGen heading size and blue activity pulse
  so an active task is easier to notice than completed work stream rows.

### Fixed

- Public Web startup now prints a directly usable host URL instead of the
  `<server-ip>` placeholder, supports `TIMEM_PUBLIC_HOST`/`--public-host` for
  multi-interface deployments, and skips opening a browser on the server.

### Added

- Added manual per-Session ToolGen preservation. A ToolGen button on each
  completed task can open an optional-guidance dialog, then run a bounded
  temporary Context against that exact source turn to create or update one or
  more runtime-validated reusable scripts. The original final answer remains
  unchanged on success or failure. The composer opens a default ToolRepo side
  panel and shows its count inside the control; ToolRepo supports code search,
  sorting, file tree, README, rename and terminal-open operations.

- Added a local authenticated `timem-web` host with an assistant-ui conversation
  surface, multiple isolated sessions, file attachments, runtime settings,
  GFM Markdown rendering, syntax-highlighted copyable code blocks, completion
  telemetry, context-compaction activity, and persistent theme/font/text-size
  appearance controls.
- Added Web handling for concurrent host-decision topics and 30-second optional
  AGENTS/CLAUDE loading decisions without blocking other sessions.
- Added task-level Web turn envelopes: the original task, mid-turn supplements,
  and approvals remain together; model/runtime work streams in a bounded process
  frame; final Markdown and token/time telemetry are delivered separately below.
- Added live per-session cwd display in Web navigation and above the composer;
  successful `self_tool cwd/chg_cwd` actions update both locations immediately.
- Added per-session context usage above the Web conversation, live current-task
  and latest-call token usage inside the working frame, and authoritative final
  task token/time telemetry for both successful and non-answer turn endings.
- Added per-session runtime profiles for Web sessions. A new session can select
  its model, API/response protocol, endpoint, token limits, approval
  policy, and process-local API key without changing existing sessions or
  exposing API keys in browser snapshots and topics.
- Added explicit Session/Context/Worker ownership for the Web host. Sessions
  own shared runtime profiles, contexts own workspace state, workers carry
  parent linkage and scoped topic identities, and aggregate state remains
  correct when a child worker finishes before its primary worker.
- Added Session-wide task cancellation with primary-only continuation: Stop
  cancels all internal workers, while the next user turn is sent only to the
  primary worker.
- Working-turn input now uses the normal send icon and a concise `继续输入…`
  placeholder while preserving supplement routing inside the host protocol.
- The Web conversation surface now follows assistant-ui's low-distraction chat
  composition: process updates are visually subordinate, all tool calls use
  compact borderless expandable rows, final telemetry is quiet inline text,
  and long workspace paths retain their useful trailing components.
- Default Web sessions are named `Session0`, `Session1`, and so on. Each Session
  can expand in the sidebar to show its scoped `ID0`-style workers and live
  worker states without exposing or changing routing identifiers.

### Changed

- Production CI now installs locked frontend dependencies, runs Web reducer and
  rendering tests, rebuilds the embedded frontend, and builds both CLI and Web
  release binaries on Linux and macOS.
- Install and uninstall scripts now manage both `timem` and `timem-web`.

### Fixed

- Long Web conversations now retain bounded client state while progressively
  mounting only the latest task window. New user tasks stay visible after the
  window starts evicting old DOM nodes, earlier-history loading preserves the
  reader's visual anchor, and overflowing process frames follow new events only
  while the reader remains near the bottom.
- New Web sessions now receive an explicit creation response instead of relying
  on lifecycle timing, disconnected sends no longer create local ghost turns,
  and decision requests render inside their owning session's chat flow so
  concurrent agents cannot overwrite or obscure one another.
- Web turn events now carry stable ids for reconnect-safe deduplication, retain
  bounded per-turn history under burst load, and use `Compact` as the user-facing
  label for context-reduction token statistics.
- Local Web responses now include CSP, no-referrer, and nosniff headers; browser
  command size, upload size/name, workspace selection, and numeric launch
  options fail closed at their host boundaries.
- Web action start/finish topics now coalesce into one lifecycle row, so a
  completed final answer cannot retain a stale `run_bash · running` entry.
- Web activity rendering no longer invents completion/reasoning captions,
  duplicates activity titles, or exposes internal model request/response and
  work-instruction bookkeeping as conversation content.
- Task frames no longer repeat `You` or the active Session name above the user
  message, process stream, and final answer; their existing visual treatment
  already communicates ownership.
- The Session drawer button is now hidden on desktop, where the Session sidebar
  is already visible, and appears only on mobile layouts where it opens the
  off-canvas navigation.
- The chat header no longer repeats the `SESSION` label and active Session name;
  it retains only a subdued model identifier.
- Uploaded files now behave as pending composer attachments: they can be removed
  before sending, long names remain inspectable without breaking the composer,
  the next submitted task consumes them into a compact user-message file row,
  and later turns do not receive stale upload context.

## [1.0.0] - 2026-07-21

Timem 1.0 is the first release with the browser host treated as a first-class
product surface. The terminal and browser hosts share one local-first agent
core, memory/session store, capability system, model transport, and structured
topic protocol.

### Highlights

- Added the authenticated `timem-web` browser workspace built on assistant-ui.
- Added isolated multi-session Web use with per-session model service profiles,
  persistent history, paged restore, mem switching, and cross-host resume.
- Added live Web rendering for Thought/Action work, tool lifecycle, inline
  decisions, supplements, cancellation, runtime disconnects, context compact,
  attachments, Markdown/code output, and final token/time telemetry.
- Added responsive desktop/mobile layout, appearance settings, keyboard and
  accessibility behavior, and bounded rendering for long conversations/output.
- Kept the terminal host as a supported first-class interface using the same
  core and persisted session data.

### Release Quality

- Rust host/core and Web frontend tests pass locally.
- Web frontend production assets are rebuilt and embedded into `timem-web`.
- CI covers Linux/macOS builds, Web tests/build, capability/protocol checks,
  session isolation, resume, cancellation pressure, and performance guards.
- Manual release smoke remains required for Safari, Firefox, iTerm2,
  Terminal.app, tmux, SSH, clean-machine installation, and live-model use.

## [0.9.10] - 2026-07-12

### Fixed

- Rust test functions and fixture corpora now live under each crate's `tests`
  directory instead of being embedded in production implementation files. CI
  rejects new `#[test]` functions under `src` or capability tool sources.

- XML protocol repair now inspects malformed root structure and returns a
  branch-matched correction skeleton. Content placed before `<response>`, such
  as a stray `<free_talk>`, receives an explicit instruction to move every tag
  inside the single root; realtime repair audit records the same guidance.
- XML repair classification is guarded by a 30-case raw-response corpus.
  Consecutive top-level `<response>` documents are rejected as trailing root
  content, while XML examples inside final-answer text remain opaque data.
- Builtin tool callback panics are now contained at the capability registry
  boundary and returned as audited internal action failures instead of
  unwinding through the Timem process.
- `run_bash` and command-backed capabilities now report Unix signal termination
  explicitly. A child command that receives SIGSEGV no longer appears as an
  ordinary `Exit code: -1`, and the current session remains usable.
- Action results are now budgeted before their prompt Delta is committed. If a
  sudden result would push estimated input beyond 95% of
  `TIMEM_MAX_LLM_INPUT`, the large output is omitted and a bounded SYSTEM note
  asks the model to narrow the action or compact context.
- Explicit local `E2BIG` and model service input/context-too-large failures now
  remove the most recent action-result Delta once, append a compact SYSTEM
  recovery note, and continue the same turn instead of immediately stopping or
  retrying forever. The recovery is recorded in the API audit.
- Model request JSON is streamed to `curl` through stdin instead of being
  placed in the process argument list, preventing large prompts from failing
  locally with `Argument list too long (os error 7)` before any HTTP request.
- Model transport now drains stdout and stderr concurrently while retaining
  cancellation polling, avoiding pipe backpressure on unusually large model API
  responses or error bodies.
- SIGINT handler registration now uses an explicit function-pointer conversion,
  eliminating the newer Rust `function_casts_as_integer` warning on Linux while
  preserving macOS behavior.

## [0.9.9] - 2026-07-11

### Added

- Added unified `core.model.response` topic events carrying model status,
  free talk, progress, final-answer metadata, and global working-worker count
  for shell/native/web host rendering.
- Added session-worker runtime state that atomically tracks active worker turns
  across concurrent workers and publishes the count in model-response topics.
- Added runtime `/config` control for `TIMEM_WORK_INSTRUCTIONS` so
  AGENTS/CLAUDE loading can be switched between `silent`, `ask`, and `off`.
- Added audit sidecar JSONL rollover for large API audit files to avoid
  rewriting large JSON audit documents on every event.
- Added streaming JSONL entry counting for `/prof` storage metrics so large
  memory/scratch files are not loaded fully into memory.
- Added `scripts/performance_guard.sh` and CI coverage for large prompt render,
  topic fan-out, and long Thought / Action panel rendering hot paths.

### Changed
- Improved `self_tool` prompt description to remind the model to use `chg_cwd` instead of repeating `cd` in every `run_bash` command, reducing redundant output.
- Default response protocol changed from JSON to XML.
- Consolidated core protocol and shell runtime into agent_core.
- Reorganized capability tools into resources/capabilities/tools/.
- Renamed working action section to working_still_action.
- Renamed foreground bash mode to normal mode.
- Successful assistant responses are now replayed into the next prompt delta as
  raw model output by default; the previous extracted free_talk/final-answer
  replay remains available through `AssistantReplayMode::ExtractedFields`.

- The Thought / Action panel now renders model `free_talk` and progress from a
  single model-response topic before action rows, keeping UI updates coherent.
- Direct shell turns now mark `working_worker_count` as `1` while work
  continues and `0` when the current turn is finished, matching the multi-worker
  topic semantics.
- Startup notices are grouped into a startup status block, and runtime command
  help is routed through `/help`.
- Agent core now caches the fully expanded static prompt and refreshes it only
  when response protocol or capability registry changes.

### Fixed
- Concrete JSON, Markdown, and XML protocol examples are now executed by
  their matching runtime parser in tests, so prompt examples cannot silently
  drift beyond executable runtime behavior.
- Tracked shell jobs (background and timeout) now execute under `/bin/bash -lc` instead of `/bin/sh -lc`, matching normal `run_bash` execution semantics and enabling bash-specific syntax such as heredoc with backticks.
- Heredoc delimiters (e.g. `<<'EOF'`) in tracked background and timeout jobs are now preserved correctly; the runtime no longer wraps the command in a shell wrapper that would corrupt multi-line heredoc syntax.
- Tool job status routed through capmgr.
- Bash action results naturalized for model readability.
- Model-visible deltas simplified.
- Worker name used as assistant heading.
- Uncached response format trailer appended correctly.
- CI removed Microsoft apt repos returning 403.
- CI replaced private fixture data with safe fixtures.

- Malformed model responses that require protocol repair no longer publish a
  model-response topic from the invalid response before the repair round.
- Observation panel row trimming avoids front-removal loops on long wrapped
  content.
- Model responses with more than two actions now publish observation metadata
  for every action the core will execute, so the UI does not hide later actions.
- Repeated `思考中...` updates now use idempotent transient rendering and do not
  show duplicate `x2` status for a single active turn.
- Startup config tables now keep long env keys such as
  `TIMEM_WORK_INSTRUCTIONS` on one row instead of splitting a trailing
  character into a separate line.
- XML response parsing now ignores protocol-looking tags inside CDATA action
  strings, so valid action args containing examples such as `<status>` or
  `<working_still_action>` are kept as data instead of parsed as control tags.
- XML response parsing now uses a protocol-specific tag scanner for the small
  `<response>` vocabulary. `final_answer`, `free_talk`, and context compact
  `summary` are extracted as raw text and are not scanned for nested protocol
  tags.
- XML response prompt guidance now uses a single strict System Response Protocol
  section, including explicit stream order, mutually exclusive state branches,
  CDATA action JSON, and a final `Protocol Loaded` marker.
- XML response parsing now accepts a whole response wrapped in a documentation
  ```xml fence while still parsing the inner `<response>` through the same
  protocol scanner, and rejects XML replies that mix multiple state branches.
- XML `<action_json>` now passes the extracted JSON text directly to the JSON
  parser and requires a top-level workflow array; old `{ "action": ..., "args":
  ... }` and `{ "order": ..., "actions": ... }` objects are rejected for repair.
- Cross-protocol response tests now assert full action-group structure, not
  only flattened action order, for complex valid JSON/Markdown/XML responses.

- Finished background and timeout job exit updates now include exit status code and
  final output, so the model receives the complete job result without a separate
  follow-up command.
- Tracked shell jobs are reaped by a shared ShellJobWatcher thread using
  Child::try_wait instead of per-pid kill -0 polling, preventing zombie processes
  and improving reliability on macOS and Linux.

## [0.8.1] - 2026-07-03

### Added

- Auto-wrap bare array of action objects as `next_actions` in model response parsing, improving tolerance for non-envelope responses.
- Expanded envelope detection and added Markdown fence stripping for model responses.

### Fixed
- Tool job status routed through capmgr.
- Bash action results naturalized for model readability.
- Model-visible deltas simplified.
- Worker name used as assistant heading.
- Uncached response format trailer appended correctly.
- CI removed Microsoft apt repos returning 403.
- CI replaced private fixture data with safe fixtures.

- Replaced private fixture data with synthetic test names in core tests to pass `test_contract_check`.
- Applied `cargo fmt` to resolve formatting diffs in CI.

## [0.8.0] - 2026-07-03

### Added

- Added tail-aware KV-cache planning for growing prompt deltas, with replay
  tests that simulate service-side cache matching and guard against the old
  low-hit-rate strategy.
- Added CI coverage for KV-cache replay quality gates and openai-compatible
  cache marker generation.

### Changed
- Default response protocol changed from JSON to XML.
- Consolidated core protocol and shell runtime into agent_core.
- Reorganized capability tools into resources/capabilities/tools/.
- Renamed working action section to working_still_action.
- Renamed foreground bash mode to normal mode.

- Refined the model-facing response envelope wording and regenerated the
  expanded static prompt snapshot.
- Documented the KV-cache tail planning algorithm and replay evidence in the
  architecture and optimization notes.

### Fixed
- Tool job status routed through capmgr.
- Bash action results naturalized for model readability.
- Model-visible deltas simplified.
- Worker name used as assistant heading.
- Uncached response format trailer appended correctly.
- CI removed Microsoft apt repos returning 403.
- CI replaced private fixture data with safe fixtures.

- Stabilized Thought / Action panel rendering when ANSI color sequences are
  present, preventing visible-width miscalculation during long command/status
  redraws.

## [0.7.1] - 2026-07-03

### Fixed
- Tool job status routed through capmgr.
- Bash action results naturalized for model readability.
- Model-visible deltas simplified.
- Worker name used as assistant heading.
- Uncached response format trailer appended correctly.
- CI removed Microsoft apt repos returning 403.
- CI replaced private fixture data with safe fixtures.

- Removed Markdown fenced code blocks from the model-facing static prompt,
  response schema summary, and generated tool action examples. This reduces
  the chance that models copy Markdown fences into protocol responses.
- Updated the expanded static prompt snapshot and regression tests to guard
  against reintroducing prompt fences.

## [0.7.0] - 2026-07-03

### Changed
- Default response protocol changed from JSON to XML.
- Consolidated core protocol and shell runtime into agent_core.
- Reorganized capability tools into resources/capabilities/tools/.
- Renamed working action section to working_still_action.
- Renamed foreground bash mode to normal mode.

- Runtime static prompt source now uses `resources/static_v1.md`, a Markdown
  prompt with explicit injection placeholders for response schema, tool catalog,
  and skill headers.
- Capability tool manifests now use JSON Schema style `input_schema` and
  `output_schema` blocks as the executor-facing IDL for `capmgr` inspection and
  generic runtime validation, while prompt rendering derives a concise Markdown
  capability guide from the same manifests.
- Action parsing is now generic over `action` / `intent` / JSON-object `args`
  and no longer extracts concrete tool options in the top-level parser.
  Tool-specific options and validation live in the manifest-backed executor
  boundary, and unknown legacy action names are rejected instead of silently
  bridged.
- Host-adapter boundaries are documented and tested: `agent_core` stays free of
  terminal UI dependencies and keeps C ABI entry points for future iOS/Web
  integrations, while `agent_core` owns model transport behavior.
- Prompt segment rendering now lives in `agent_core::prompt_render`, keeping
  static prompt enrichment and visible delta/slice rendering behind a single
  module boundary.
- A generated read-only static prompt snapshot documents the fully expanded
  `prompt_0` after schema and capability injection; CI checks that it stays
  current.
- Model-facing tool catalog is now a concise natural-language capability guide
  instead of a verbose JSON Schema dump; runtime validation still uses the full
  manifest schemas internally.
- The release-quality skill is now an optional capability overlay example
  instead of a built-in skill compiled into `agent_core`.
- Added a built-in `self_tool` capability for Timem self-inspection:
  non-secret runtime env read/write, memory/audit path reporting, and software
  about/version/process metadata. Memory path env variables such as
  `TIMEM_DATA_DIR` and `TIMEM_SPACE` are protected as startup-only settings.
- Added focused core scenario replay tests for coding inspection, memory QA,
  Timem self QA/env update, and file-writing output workflows.
- Added session-level regression tests for incremental KV-cache prompt planning
  and profiler cached-token accounting.

### Fixed
- Tool job status routed through capmgr.
- Bash action results naturalized for model readability.
- Model-visible deltas simplified.
- Worker name used as assistant heading.
- Uncached response format trailer appended correctly.
- CI removed Microsoft apt repos returning 403.
- CI replaced private fixture data with safe fixtures.

- Clarified `status:"finished"` protocol semantics in the model prompt and
  schema summary: a finished response closes the current model/action loop, so
  models should use it only with a complete final answer.
- Transient model-service/network failures now retry up to five times with a
  user-visible status line before failing the turn.
- Protocol repair slices now include a focused window around the malformed
  model output, so the model can repair the concrete error without copying an
  oversized response into context.
- Thinking and final status lines now show repair round overhead as
  `⇌N (⚠M)` when protocol repair consumed model calls.
- Protocol repair requests now write structured `model_repair_request` audit
  events with issue, usage, truncation, and repair-count metadata, and also
  append realtime diagnostics to `audit/api_output_repair.json` with the
  malformed assistant response plus the SYSTEM repair message shown to the
  model.
- API payload audit now stores a structured `api_audit.json` document with a
  `version` field and `events` array, while chat-history readers still accept
  legacy JSONL audit files.
- Responses that prematurely combine `status:"finished"` / `final_answer`
  with evidence-gathering `next_actions` are now downgraded to working:
  runtime discards the premature final answer, executes the actions, and asks
  the next model round to answer only from action results.

## [0.6.0] - 2026-07-01

### Added

- Model response protocol now uses `free_talk` plus `continue`.
  Progress can be shown in the Thought/Action panel while actions continue,
  and `continue:false` marks the final user-facing summary.
- Guarded finalize allows `continue:false` plus a final `expect` check to skip
  an extra model round only after runtime-controlled verification passes.
- Unified model-facing memory protocol: `memmgr` now covers durable memory,
  raw chat history, scratch memory, and prompt-context shrink through
  `type`/`op` fields.
- Session-runtime integration tests for `memmgr` durable lookup, scratch
  context offload, and forced context shrink.
- Multi-turn replay integration test covering normal replies, malformed model
  response recovery, durable memory retrieve, scratch context offload, forced
  shrink, audit writes, and observation rendering in one scripted story.
- GitHub Actions CI that runs the same production gate as local development:
  script syntax checks, install logic, contract checks, sensitive scan,
  formatting, full Rust tests, edge regression, release build, real TTY smoke,
  and whitespace checks.
- Thinking status now shows model round count, total token usage, current
  context utilization bar, and latest request token deltas in a compact
  multi-line layout.

### Fixed
- Tool job status routed through capmgr.
- Bash action results naturalized for model readability.
- Model-visible deltas simplified.
- Worker name used as assistant heading.
- Uncached response format trailer appended correctly.
- CI removed Microsoft apt repos returning 403.
- CI replaced private fixture data with safe fixtures.

- Observation panel wraps long intent/action lines instead of truncating them.
- Observation panel renders action details as child rows under the user-facing
  intent, using tree prefixes for Bash and memory/context activity.
- Observation panel hides model-private `thought` content while still showing
  user-facing action intent and Bash commands.
- Model responses wrapped in prose or fenced JSON are parsed for observation
  events when the embedded response envelope is valid.
- Paste recovery no longer reports an untouched `[ pasted N lines ]` marker as
  edited when stale preserved paste records exist from an earlier return-to-edit
  flow.
- Paste recovery Note menu treats Esc as cancel for the current input activity.
- Final response status now uses a concise `ctx[N%]` context label instead of
  mixing current-turn deltas into the completed turn summary.

### Changed
- Default response protocol changed from JSON to XML.
- Consolidated core protocol and shell runtime into agent_core.
- Reorganized capability tools into resources/capabilities/tools/.
- Renamed working action section to working_still_action.
- Renamed foreground bash mode to normal mode.

- Static prompt exposes `memmgr` as the canonical memory/context management
  interface instead of separate memory, chat, scratch, and shrink action names.
- Architecture and feature/test management docs now describe the `memmgr`
  protocol and session-level coverage.
- Default maximum agent interaction rounds increased from 20 to 50; continuing
  after the round limit recharges the task to 50 rounds.

## [0.5.2] - 2026-06-30

### Changed
- Default response protocol changed from JSON to XML.
- Consolidated core protocol and shell runtime into agent_core.
- Reorganized capability tools into resources/capabilities/tools/.
- Renamed working action section to working_still_action.
- Renamed foreground bash mode to normal mode.

- Clarified Ctrl+C and Esc cancellation behavior in shell documentation.

## [0.5.1] - 2026-06-28

### Fixed
- Tool job status routed through capmgr.
- Bash action results naturalized for model readability.
- Model-visible deltas simplified.
- Worker name used as assistant heading.
- Uncached response format trailer appended correctly.
- CI removed Microsoft apt repos returning 403.
- CI replaced private fixture data with safe fixtures.

- Tightened token context status labels and follow-up shell quality fixes after
  v0.5.

## [0.5.0] - 2026-06-28

### Added

- Reedline-based shell input editor with Shift+Enter multiline input, paste
  marker handling, recovery prompts, and real TTY smoke coverage.
- Token/status rendering for context size, model, cache hits, and
  current request token deltas.
- `/prof` runtime profiling for token totals, wait time, local execution time,
  and memory/audit storage size.
- Forced context shrink flow with prompt delta/slice ids and scratch context
  offload.
- Multi-CLI memory guard and durable memory conflict detection.
- Feature/test management documentation with core and UI quality axes.

### Fixed
- Tool job status routed through capmgr.
- Bash action results naturalized for model readability.
- Model-visible deltas simplified.
- Worker name used as assistant heading.
- Uncached response format trailer appended correctly.
- CI removed Microsoft apt repos returning 403.
- CI replaced private fixture data with safe fixtures.

- Repeated shell disconnect and timeout handling problems from earlier shell
  bridge iterations.
- Model-service truncation handling now explains output-token limits and can retry
  with a larger limit during the running shell process.
- Terminal input, cancellation, and paste paths received broad regression
  coverage and real pseudo-TTY smoke.

## [0.4.0] - 2026-06-23

### Added

- Initial public Timem Shell Agent release with local Bash action support,
  local structured memory, model transports, audit logs, install scripts, and
  README run instructions.
