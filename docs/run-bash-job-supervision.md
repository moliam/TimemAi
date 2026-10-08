# `run_bash` in-process job supervision

## Goals

`run_bash` jobs belong only to the current runtime instance. They are not persisted,
restored, adopted, or inferred from historical PIDs after restart.

The implementation must provide one authoritative lifecycle per spawned Job:

- one supervisor owns and reaps the Bash launcher;
- stdout and stderr are drained concurrently into bounded buffers;
- on Linux, a delegated cgroup v2 directory provides kernel-backed ownership of every descendant, including `setsid` escapees;
- kernel-owned descendants (or members that remain in the managed process group) remain cancellable after the launcher exits;
- foreground completion, timeout-to-background, cancellation, session cancellation, and
  runtime shutdown have deterministic ownership and delivery semantics;
- completed jobs do not accumulate in the manager.

## Ownership model

`ShellJobManager` is an in-memory index of lightweight job handles. It does not own
`Child`, poll every child, or derive lifecycle state by scanning the index.

Each job has one supervisor thread. The supervisor exclusively owns `Child` and is the
only component that publishes the terminal result. Output drain threads own the two pipe
readers and are joined by the supervisor. The manager and callers may only:

- inspect a job snapshot;
- atomically promote a still-running direct job to background delivery;
- signal its kernel-owned Job (or its process group in explicit degraded mode);
- wait on its condition variable;
- claim a terminal result through the permitted delivery path.

No OS process query or signal is performed while the manager index lock is held.

## Kernel Job ownership and runtime-wide fallback

Every direct child created by Timem follows one explicit exit-status ownership chain:

1. prepare the per-Job kernel container when the platform supports it;
2. configure child self-placement before `exec`;
3. spawn the child and immediately register its PID as managed;
4. keep the registration guard with the component that owns `Child`;
5. wait for the child and capture its exit status;
6. release the registration only after that wait completes.

On Linux the hierarchy is `timem.jobs/runtime-<pid>-<start_ticks>/session-<opaque-key>/job-*` below a writable delegated cgroup v2 subtree. Runtime and Session parents remain empty aggregation/observation scopes; only Job leaves contain user processes. The opaque deterministic Session key does not expose the raw Session id, and PID plus `/proc` start ticks prevents PID-reuse ownership mistakes. The child writes `0` to the already-opened Job `cgroup.procs` descriptor from `pre_exec`, before user-controlled code can fork. Therefore later descendants, including processes that call `setsid`, remain members of that Job. Ownership is never inferred from timing, PID proximity, or subreaper adoption. `cgroup.kill` terminates only that Job, and `cgroup.events: populated 0` is the completion fact.

At Agent/Runtime startup, the model receives both the current Runtime aggregate observation path and the current Session aggregate observation path once after those scopes first exist. Ordinary requests do not append duplicate reminders; successful context compression re-arms the reminder because prior model context may have been removed. The Runtime does not pre-read or render `memory.current`, `pids.current`, `cpu.stat`, or similar files—the path is sufficient for an explicit inspection decision. Previous-Runtime scopes with live members remain visible as anomalies for the exact Session, but are never silently adopted or killed. Empty stale leaves and parents are reclaimed with bounded, name-filtered, kernel-confirmed cleanup that does not traverse or delete a populated sibling Session.

Timem may ask the user for the minimum permission needed to create this subtree. A formal systemd service must use `Delegate=yes` (or an equivalently restricted delegated scope). Internal command capabilities currently fall back to process-group execution when exact ownership is unavailable; this preserves execution compatibility but does not guarantee cleanup of descendants that leave the group. This fallback must not be certified as exact containment. `run_bash` may continue in an explicitly reported degraded process-group mode when permission is denied; this mode must not be described as exact containment.

Synchronous `Command` users must use the platform `command_status` or `command_output` helpers, which preserve the registered owner's exit status.

Linux subreaper supervision remains a runtime-wide fallback for descendants whose normal ownership chain is broken. It uses repeated `(PID, process start time)` observations, bounded state, and exact `waitpid`. Only current actionable fallback state (live or zombie awaiting final reap) is model-visible and is not attributed to the most recently completed Job. Normal adopted-to-reaped history is internal bookkeeping and is not injected into model context. Per-Job code never sweeps all adopted children.

The three layers are intentionally not interchangeable: the Job backend answers membership/control/completion, direct-child registration reserves exit status for the owning supervisor, and the subreaper fallback performs final Unix reaping when no registered owner remains. Job termination does not reap zombies; `waitpid` ownership does not identify or contain a descendant tree.

## macOS process-group limits

macOS currently has no exact per-Job backend and no Linux-style subreaper fallback.
Kernel start-time identity distinguishes process instances; it does not establish
ownership of descendants that leave a process group. Cancellation covers members
that remain in the managed group, with SIGTERM followed by SIGKILL when still live.
Group enumeration uses `proc_listpgrppids`: its result is a PID count, while its
buffer argument is bytes. Full buffers require bounded retries; unknown query
failures must not certify group completion. Unrelated protected system processes
are not queried. Zombie members do not count as live work.

A descendant that calls `setsid` can escape this mode. If it closes inherited
pipes, group completion can be reported while it remains alive; if it retains
pipes, output capture now stops after a 250 ms EOF grace period once the known
group is empty. Both reader threads are joined and the incomplete capture is
reported as `output_capture_incomplete`, not successful completion. This deadline
is an I/O policy, not evidence of descendant identity or termination. There is
still no all-descendant termination guarantee on macOS. Neither process scanning nor
launch timing may be used to guess ownership or kill unrelated processes. These
are capability limits, not repaired by the PID-count correction. Exact containment
requires a separately verified native ownership backend.

## Comparable disk-pressure samples

Platform owns the filesystem snapshot facade: local mount discovery, working-path
queries, device deduplication and omission of unavailable samples. Agent supplies
working paths and owns observation gates, baseline comparison and notices.
Disk pressure compares aggregate free space only for the same sorted set of
filesystem device IDs and capacities. Mount/unmount, a missing member, replacement
with a different device ID, or resize establishes a new baseline without a pressure
notice. Path aliases and enumeration order do not change the scope. A missing
whole sample leaves the window due; a later successful comparable sample can alert.
The existing 10-observation/3-minute gates and min(200 MiB, 8% capacity) threshold
are unchanged, including accumulation of sub-threshold drops and startup seeding.
This is sampled device identity, not continuous mount-generation tracking: reuse
of the same device ID and capacity entirely between samples is not detectable.

## Lifecycle and delivery state

A job starts in one of two delivery modes:

- `Direct`: normal foreground execution; only its initiating action may claim a result.
- `Background`: explicit background execution; the owning session receives one exit update.

A direct job is atomically promoted to `Background` only if it is still running when the
foreground wait budget or long-running handoff point is reached. This makes the boundary
race deterministic:

- completion wins the state lock: return the final direct result;
- promotion wins the state lock: return a running PID and later emit one background update.

Terminal results are immutable. Claiming a direct result or consuming a background update
removes the job from the manager. Thus the index contains only running jobs plus terminal
jobs awaiting exactly one legitimate consumer.

## Completion definition

The supervisor publishes `Finished` only after all of these hold:

1. the Bash launcher has been reaped and its exit status captured;
2. Linux cgroup v2 reports `populated 0` for that Job (or the process group is empty in explicit degraded mode);
3. stdout and stderr drain threads were joined, either at EOF or with an explicit
   capture failure after the bounded EOF grace period. The launcher exit status
   is retained separately; capture failure makes the action failed even if the
   launcher returned zero.

Drain threads run while descendants are alive so pipe backpressure cannot block them. If the launcher dies from a signal or Job-state observation fails, the supervisor terminates that Job before publishing a terminal result.

## Cancellation and shutdown

Cancellation targets only handles selected from the current manager instance. Selection
happens under the index lock; signalling happens after releasing it.

- action cancellation signals its own job, waits for supervisor convergence, then removes it;
- session cancellation signals unfinished jobs owned by that session;
- runtime shutdown signals all unfinished jobs and joins all supervisors;
- dropping the last manager performs the same ownership cleanup; the degraded-mode limits below also apply;
- a newly constructed manager cannot observe or signal another manager's jobs.

Runtime restart does not read historical job metadata or signal historical PIDs. Known
legacy `shell_jobs` directories are deleted without parsing their contents.

## Output policy

stdout and stderr are always drained concurrently to avoid pipe deadlock. Each stream is
bounded independently to 1 MiB:

- `tail_out=false`: retain the first 1 MiB;
- `tail_out=true`: retain the last 1 MiB.

The terminal result stores separate stdout/stderr plus normalized combined output.
Platform owns timed pipe reads (Unix readiness polling, Windows pipe availability);
Agent owns the 250 ms post-scope EOF grace policy. Finite command bindings and tool-repository self-tests also share a bounded post-exit capture helper (`command_output`); both streams are joined on success, error, timeout, or drop, and missing EOF rejects successful completion/tool publication. Read errors, reader panics and
missing EOF produce an explicit capture error without fabricating bytes in either
stream. A continuously writing out-of-scope process cannot extend the deadline.
The supervisor joins both readers before publication; no blocked reader is detached.

## Test matrix

Tests must cover:

- direct success, non-zero exit, signal exit, spawn failure, invalid timeout;
- exact completion-vs-timeout and completion-vs-long-running handoff boundaries;
- explicit background completion and one-shot notification;
- timeout handoff retaining partial output and later final output;
- cancellation before launcher exit and after launcher exit with live descendants;
- concurrent session cancellation isolation and repeated cancellation idempotence;
- runtime shutdown/drop with active jobs and no cross-manager signalling;
- launcher exit while descendants keep pipes open, and descendants that close pipes but live;
- macOS escaped pipe holders: bounded completion, cancellation and manager drop,
  explicit partial-output failure and one-shot exit delivery;
- platform idle-pipe read deadline, buffered-data-before-EOF and empty-buffer rejection;
- real macOS disk-image mount/unmount discovery, separate from physical USB hotplug;
- large simultaneous stdout/stderr, UTF-8 split boundaries, head/tail truncation;
- completed-result removal and sustained many-job runs without index growth;
- legacy directory cleanup without PID adoption;
- real Web Stop/cancel flows in addition to manager-level tests;
- registered-child exit status isolation from the fallback reaper;
- spawn-to-registration race tolerance without premature adoption;
- cgroup self-placement before exec and containment of `setsid` descendants;
- timeout handoff leaving an owned `setsid` descendant running until explicit cancellation;
- terminal delivery waiting for an owned `setsid` descendant to exit;
- internal command timeout/error cleanup of all owned descendants;
- exact `adopted -> fallback_active -> reaped` lifecycle for genuinely unowned fallback processes;
- synchronous command helpers preserving exit status and captured streams.
