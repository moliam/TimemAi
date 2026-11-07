# Core Platform Boundary

`core/platform` is the only Core module that owns operating-system policy and
low-level process primitives shared across Timem hosts.

## Layout

- `src/api.rs`: general stable, UI-neutral platform API consumed by Core, including filesystem usage snapshots (local mount discovery, platform device identity, per-device deduplication and unavailable-path handling) and instance resource caps (kernel-enforced bounds on how much of the machine's total CPU capacity and total memory the current instance's managed job tree may consume).
- `src/child_input.rs`: exclusively owned nonblocking child stdin; reports backpressure and errors without choosing deadlines or creating writer threads.
- `src/child_output.rs`: exclusively owned child stdout/stderr pipes with bounded, interruptible reads; no job attribution or deadline policy.
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
- Platform-neutral facades and their diagnostics must not name a single backend mechanism; cgroup/Job Object vocabulary stays inside the target modules.
- A process-Job backend must establish ownership before user code can fork, target destructive operations only at that Job, and expose kernel-derived membership/completion facts.
- Aggregate observation scopes are platform-owned, read-only facades to Agent/Core. Linux creates stable empty `runtime-<pid>-<start_ticks>/session-<opaque-key>` parents above per-Job leaves, exposes both current Runtime and current Session locations, and retains them until Runtime cleanup; raw Session ids must not enter platform paths.
- Restart cleanup may remove only kernel-confirmed empty managed scopes. A previous Runtime scope with live members is never silently adopted, signalled, or deleted.
- Target-specific modules compile only on their matching target.
- Unsupported targets fail closed for ownership/destructive decisions.
- Platform behavior changes require tests under `core/platform/tests`.
- Session resource accounting is a read-only, bounded facade: it reports the Session scope's aggregated CPU time and memory charge plus the effective caps of the managed job tree. It never creates scopes, signals processes, or decides when to notify a model; sampling schedules, thresholds, and runtime notices belong to the Host/Agent.
- Startup relocation is a narrow platform policy: `instance_scope_needs_isolation` is a read-only structural verdict (shared root, non-`domain` root, or missing `memory` delegation), and `relaunch_in_private_user_scope` only re-execs the current process into a transient delegated user scope, returning on failure. It never manages other processes, writes system units outside that transient scope, or changes resource caps by itself.
- Instance resource caps are applied once per process, are percentages of machine totals, and bind only the managed `timem.jobs` subtree (never Timem itself or sibling scopes). Memory is written as a hard `memory.max` plus a soft `memory.high` at 85% of it, so the kernel throttles/reclaims before it has to OOM-kill. The main process lives in the unlimited `timem.self` sibling under the same instance root; enabling controllers under a populated root makes the kernel force-convert the cgroup to a threaded subtree that carries only cpu, so the escalation plan always migrates before enablement. On a user-owned root the same repair runs without root (unprivileged migrate + enable along the writable ancestor chain), and a graceful shutdown removes the instance's managed subtrees (best effort: populated groups keep their fence). An instance root shared with sibling (non-Timem) processes never migrates: memory stays `Unavailable` there (fail closed) while cpu escalates exactly as before, so shared scopes keep full cpu availability. A cap that cannot be enforced is reported (`Unsupported`/`Unavailable`) instead of silently ignored; platform gaps (e.g. no kernel quota facility) are disclosed, never faked.
- macOS mount enumeration uses caller-owned `getfsstat` storage and bounded field decoding; never retain or traverse `getmntinfo` process-global storage across concurrent callers.
- Filesystem snapshots expose device identity, capacity and free bytes; consumers supply working paths. Sampling schedules, comparable-baseline policy, pressure thresholds and model notices belong to Agent, not Platform. Agent must not reconstruct mount/device sampling outside this facade.
- Windows Platform behavior follows `docs/windows-support-matrix.md`; a compiling Platform backend
  does not by itself claim Windows support for Agent, Bridge, Interface, host, or installation layers.


## Process lifecycle responsibility split

These mechanisms are complementary and must remain separate:

- `process_job`: establishes kernel-backed Job membership before user code can fork; controls only that Job; reports membership and completion. Linux uses cgroup v2, while other platforms may provide Job Objects or native equivalents behind the same facade.
- direct-child wait registration: records which in-process owner has exclusive responsibility for `Child::wait` and its exit status. It neither attributes descendants nor terminates processes.
- subreaper/orphan fallback: adopts and finally `waitpid`s descendants whose normal parent chain disappeared, preventing zombies and exposing genuinely unowned processes. It never infers Job ownership from timing; a platform Job backend remains authoritative for attribution.

Killing a Job does not reap Unix child exit records, and reaping a child does not establish or control Job membership. Combining these responsibilities would either lose exit status or risk signalling unrelated work.

## macOS conditional cleanup contract

- Finite command cleanup supports the direct child and descendants that remain
  in its managed process group. This is ordinary command lifecycle management,
  not a security boundary against code deliberately escaping that group.
- Descendants using `setsid`, changing process groups, or delegating work to an
  external service are outside this cleanup guarantee. No privileged service
  or host reconfiguration is required by this supported mode.
- Bounded pipe capture prevents an escaped pipe holder from indefinitely
  delaying the caller. An incomplete-capture error is evidence about output,
  not proof that all descendants have terminated. An escapee that closes the
  pipes may not be detected; successful command exit is not a process-tree
  emptiness certificate.
- Do not kill guessed PIDs or unrelated processes to approximate stronger
  ownership. Stronger containment, if added later, requires a separate explicit
  platform contract and native verification.
