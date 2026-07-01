# Changelog

All notable changes to TimemAi are tracked here. This project follows a
pragmatic Keep a Changelog style: newest changes first, with release sections
for tagged versions and an `Unreleased` section for work not yet tagged.

## [Unreleased]

### Changed

- Capability tool manifests now use JSON Schema style `input_schema` and
  `output_schema` blocks as the shared IDL for prompt rendering, `capmgr`
  inspection, and generic runtime validation.
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

- Transient provider/network failures now retry up to five times with a
  user-visible status line before failing the turn.
- Protocol repair slices now include a focused window around the malformed
  model output, so the model can repair the concrete error without copying an
  oversized response into context.
- Thinking and final status lines now show repair round overhead as
  `⇌N (⚠M)` when protocol repair consumed model calls.
- Protocol repair requests now write structured `model_repair_request` audit
  events with issue, usage, truncation, and repair-count metadata for later
  diagnosis without storing raw malformed responses.

## [0.6.0] - 2026-07-01

### Added

- Model response protocol now uses `report_job_progress` plus `continue`.
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

- Static prompt exposes `memmgr` as the canonical memory/context management
  interface instead of separate memory, chat, scratch, and shrink action names.
- Architecture and feature/test management docs now describe the `memmgr`
  protocol and session-level coverage.
- Default maximum agent interaction rounds increased from 20 to 50; continuing
  after the round limit recharges the task to 50 rounds.

## [0.5.2] - 2026-06-30

### Changed

- Clarified Ctrl+C and Esc cancellation behavior in shell documentation.

## [0.5.1] - 2026-06-28

### Fixed

- Tightened token context status labels and follow-up shell quality fixes after
  v0.5.

## [0.5.0] - 2026-06-28

### Added

- Reedline-based shell input editor with Shift+Enter multiline input, paste
  marker handling, recovery prompts, and real TTY smoke coverage.
- Token/status rendering for context size, provider/model, cache hits, and
  current request token deltas.
- `/prof` runtime profiling for token totals, wait time, local execution time,
  and memory/audit storage size.
- Forced context shrink flow with prompt delta/slice ids and scratch context
  offload.
- Multi-CLI memory guard and durable memory conflict detection.
- Feature/test management documentation with core and UI quality axes.

### Fixed

- Repeated shell disconnect and timeout handling problems from earlier shell
  bridge iterations.
- Provider truncation handling now explains output-token limits and can retry
  with a larger limit during the running shell process.
- Terminal input, cancellation, and paste paths received broad regression
  coverage and real pseudo-TTY smoke.

## [0.4.0] - 2026-06-23

### Added

- Initial public Timem Shell Agent release with local Bash action support,
  local structured memory, provider adapters, audit logs, install scripts, and
  README run instructions.
