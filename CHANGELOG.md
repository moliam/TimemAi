# Changelog

All notable changes to TimemAi are tracked here. This project follows a
pragmatic Keep a Changelog style: newest changes first, with release sections
for tagged versions and an `Unreleased` section for work not yet tagged.

## [Unreleased]

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

### Changed

- Static prompt exposes `memmgr` as the canonical memory/context management
  interface instead of separate memory, chat, scratch, and shrink action names.
- Architecture and feature/test management docs now describe the `memmgr`
  protocol and session-level coverage.

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
