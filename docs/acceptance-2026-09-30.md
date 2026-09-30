# 2026-09-30 local acceptance

Scope: recent context/compaction, memo, terminal supplement handoff, process
supervision, restart/history, stream/reasoning UI, and associated uncommitted fixes.
Environment: Ubuntu Linux, actual debug/release Host, Google Chrome and Expect PTY.
No production memory was migrated; no cloud-provider request was required.

## Executed evidence

| Area | Execution | Result |
| --- | --- | --- |
| Shared runtime, memo, context, process ownership, persistence | `cargo test --workspace --locked -- --test-threads=1` | Passed; ignored diagnostics are not counted as executed |
| Web reducers/layout/contracts | `pnpm --dir interfaces/web test` | 526 passed |
| Browser interaction | `pnpm --dir interfaces/web test:browser` | Passed: Stop, reconnect, archives, ordering, reasoning notices, stream continuity, responsive counts |
| Real Host + HTTP + Chrome | Product scenarios in browser suite | Passed XML/JSON/native normal streaming and non-streaming; XML malformed response, broken connection, Stop/next Send, supplement, interaction, tools |
| Release process lifecycle | cross_host_resume, web_runtime_lifecycle, web_public_runtime, linux_web_platform smoke scripts | All passed with release binary (also debug earlier) |
| Real terminal | real_tty_smoke.expect, real_tty_supplement_smoke.expect | Both passed |
| Runtime I/O | runtime_io_guard.py | Passed |
| Performance | performance_guard.sh | Passed Core/Shell and five Web hot-path tests |
| Turn concurrency | turn_concurrency_stress.sh | 300 seeded Core/Worker iterations passed |
| Repeated edge cases | edge_regression.sh | Two iterations passed; 57 session runtime tests actually ran per iteration |
| Static/build gates | fmt, workspace Clippy, cargo doc, Web build, module/architecture/test-contract, matrix, self-capability, static prompt, sensitivity, licenses, install logic, KVC replay | Passed |

## Review corrections

- Compaction reports surviving refs after removal instead of listing removed refs;
  stale refs remain idempotent and separately reported. Native/JSON regression added.
- Removed needless borrowed reference in indexed history append (Clippy).
- Scoped restart-path font assertion to its CSS rule; unrelated monospace rules
  no longer create false failures while the original UI-font constraint remains.
- Matrix evidence lookup no longer recursively traverses the repository root,
  avoiding disappearing compiler temporaries and dependency/build false evidence.
  Verified source evidence succeeds while build/dependency-only evidence fails.
- Edge regression now selects `agent_core` session runtime tests, not their old
  Shell location; an empty discovery result fails instead of silently passing.

## Limits

HTTP model responses in product tests are controlled fixtures, not live cloud
models. Browser fixture tests and actual Host product tests are distinct evidence.
Windows/macOS, real mobile Safari and all cloud-provider combinations were not run.
The stress script explicitly leaves Host attachment FIFO, Stop/start, WebSocket FIFO
and Chrome latency stress pending; ordinary browser scenarios are not substitutes.
Manual-compaction/memo edge behavior has Core/Worker/Host regression evidence, not
an exhaustive cloud-backed browser matrix. Full `ci.sh` was not executed as one
invocation; the component checks above were run separately.
