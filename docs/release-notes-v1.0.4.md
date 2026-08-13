# TimemAi 1.0.4

TimemAi 1.0.4 is the completed Timem Web release for the Agent Core protocol,
session workspace, MCP, queueing, scrolling, rendering, and hidden-data changes
introduced across the 1.0 development line.

## Highlights

- XML-native actions, parallel groups, bounded recovery, precise repair
  feedback, and a final-answer boundary that cannot terminate through repair.
- Session-owned model credentials and settings, MCP configuration, queued and
  immediate messages, deletion, restoration, cancellation, and race-safe
  transitions.
- Stable long-conversation scrolling and history loading, improved
  Thought/Action continuity, readable Markdown/code typography, and refined
  model/context/runtime controls.
- Hidden `.timem_data/` storage for new environments. A legacy `data/` is used
  only when it contains a verified Timem workspace, Session, or audit
  fingerprint; an unrelated directory named `data` is never claimed.
- One synchronized `1.0.4` version across the Rust workspace, embedded Web
  host, and frontend package, enforced by the production CI gate.

## Verification

The release uses the repository production CI gate on Linux and macOS. It
includes format and Clippy checks, workspace and Web tests, reproducible Web
bundle verification, dependency-license and sensitive-data scans, performance
and repeated-edge regressions, release builds, cross-host resume, and
pseudo-TTY smoke tests.
