# Core Platform Boundary

`core/platform` is the only Core module that owns operating-system policy and
low-level process primitives shared across Timem hosts.

## Layout

- `src/api.rs`: general stable, UI-neutral platform API consumed by Core.
- `src/process_job.rs`: platform-neutral per-Job process-management facade and internal backend contract.
- `src/shared.rs`: Unix primitives shared by macOS and Linux.
- `src/macos.rs`: macOS policy and kernel-derived process identity.
- `src/linux.rs`: Linux policy, `/proc` identity, and cgroup v2 process-Job backend.
- `src/windows/`: Windows command, process, filesystem, and host policy backends.

## Rules

- This crate must not depend on an Interface or Bridge.
- `agent_core` may consume only the public API; it must not duplicate platform
  selection or operating-system process lifecycle primitives.
- Agent/Core consumers depend only on platform-neutral facades; cgroup, Job Object, and other backend types must not leak outside this crate.
- A process-Job backend must establish ownership before user code can fork, target destructive operations only at that Job, and expose kernel-derived membership/completion facts.
- Aggregate observation scopes are platform-owned, read-only facades to Agent/Core. Linux uses empty `runtime-<pid>-<start_ticks>/session-<opaque-key>` parents above per-Job leaves; raw Session ids must not enter platform paths.
- Restart cleanup may remove only kernel-confirmed empty managed scopes. A previous Runtime scope with live members is never silently adopted, signalled, or deleted.
- Target-specific modules compile only on their matching target.
- Unsupported targets fail closed for ownership/destructive decisions.
- Platform behavior changes require tests under `core/platform/tests`.
- Windows Platform behavior follows `docs/windows-support-matrix.md`; a compiling Platform backend
  does not by itself claim Windows support for Agent, Bridge, Interface, host, or installation layers.


## Process lifecycle responsibility split

These mechanisms are complementary and must remain separate:

- `process_job`: establishes kernel-backed Job membership before user code can fork; controls only that Job; reports membership and completion. Linux uses cgroup v2, while other platforms may provide Job Objects or native equivalents behind the same facade.
- direct-child wait registration: records which in-process owner has exclusive responsibility for `Child::wait` and its exit status. It neither attributes descendants nor terminates processes.
- subreaper/orphan fallback: adopts and finally `waitpid`s descendants whose normal parent chain disappeared, preventing zombies and exposing genuinely unowned processes. It never infers Job ownership from timing; a platform Job backend remains authoritative for attribution.

Killing a Job does not reap Unix child exit records, and reaping a child does not establish or control Job membership. Combining these responsibilities would either lose exit status or risk signalling unrelated work.
