# Timem Self-Capability Test Matrix

This matrix is the release contract for Timem's ability to understand its own
identity, environment, task continuity, tools, long-running context, and runtime
results. `scripts/self_capability_check.sh` executes the focused deterministic
checks below. The full production gate additionally runs every Rust and Web test,
real TTY stories, real Chrome acceptance, release builds, and OS-specific smoke.

| Dimension | Required product evidence | Deterministic executable evidence | User-surface evidence | Current status |
|---|---|---|---|---|
| Who | The next model request contains Session, Context, Worker, runtime surface, command target, and current prompt identity. Scoped lifecycle topics cannot cross Sessions or Workers. | Session prompt identity and runtime-surface test; real concurrent Web worker routing test. | Shell renders lifecycle identity; Web snapshots and topics remain Session-scoped. | Automated. |
| Where | Runtime reports the real OS/cwd, exposes only matching command tools, and synchronizes an accepted cwd change to the owning Session. | Platform-profile catalog selection, dynamic OS/Bash description, and Session-scoped cwd update tests. | Linux/macOS platform smokes, Windows workspace tests, Shell `/workspace`, Web cwd projection. | Automated on the applicable CI hosts. |
| What | A restart never silently re-drives interrupted work. A direct continuation is explicit, shares the Core path, and receives the interruption/recovery context before the hidden continuation input. | Prompt-component ordering, Host direct-resume construction, and restart cwd-decision tests. | Shell direct-resume confirmation, Web composer resume tests, cross-host resume smoke. | Automated. |
| How | The prompt catalog is generated from active manifests and the same registry has executable callbacks. Host/platform filtering changes both the visible and executable capability set. | Active-protocol catalog, builtin callback registry, and host-profile filtering tests. | Full response-protocol, tool execution, Shell observation, and Web action rendering suites. | Automated for registered capabilities. Live third-party services remain opt-in release smoke. |
| Long work | Context pressure triggers bounded compaction; discarded context can be offloaded to scratch; the replacement summary survives and work continues. | Forced-shrink threshold, scratch offload continuation, and compact-summary persistence tests. | Shell/Web compact rendering plus repeated edge and performance guards. | Automated. |
| Runtime completeness | Prompt construction observes a coherent terminal/input cut: terminal projection is immutable, each job exit is delivered once, and late unconsumed input becomes a distinct next Turn without replacing the first final answer. | Terminal-order, concurrent refresh, late-supplement ownership, and Web Host final-answer tests. | TTY supplement story, Web reducer/Host suites, reconnect smoke, real Chrome Stop acceptance. | Automated for listed deterministic windows. |

## Release interpretation

Passing this matrix means the named self-capability contracts are present and
executable; it does **not** prove that every possible user workflow is bug-free.
The broader release claim also requires the complete production CI gate on
Linux, macOS, and Windows.

The following stronger certification remains open and must not be described as implemented until executable gates exist: four seeded heavy concurrency
scenarios (PromptCut/terminal ownership, Stop/Start storm, WebSocket/FIFO
ownership, and real-Chrome interaction latency) at 300 PR, 1,000 release, and
10,000 soak iterations with replayable seeds and named stage traces. Existing
focused deterministic tests and two-pass edge regression provide useful
coverage, but are not substitutes for that certification.

Safari, Firefox, distribution-specific desktop launchers, and live external
model/MCP services remain manual or opt-in checks documented in
`docs/manual-release-smoke.md`.
