# `run_bash` in-process job supervision

## Goals

`run_bash` jobs belong only to the current runtime instance. They are not persisted,
restored, adopted, or inferred from historical PIDs after restart.

The implementation must provide one authoritative lifecycle per spawned Job:

- one supervisor owns and reaps the Bash launcher;
- stdout and stderr are drained concurrently into bounded buffers;
- on Linux, a delegated cgroup v2 directory provides kernel-backed ownership of every descendant, including `setsid` escapees;
- descendants remain cancellable after the launcher exits;
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

On Linux the kernel container is a unique directory below a writable delegated cgroup v2 subtree. The child writes `0` to the already-opened `cgroup.procs` descriptor from `pre_exec`, before user-controlled code can fork. Therefore later descendants, including processes that call `setsid`, remain members of that Job. Ownership is never inferred from timing, PID proximity, or subreaper adoption. `cgroup.kill` terminates only that Job, and `cgroup.events: populated 0` is the completion fact. Empty stale Job directories are reclaimed with a bounded scan.

Timem may ask the user for the minimum permission needed to create this subtree. A formal systemd service must use `Delegate=yes` (or an equivalently restricted delegated scope). Internal command capabilities fail closed before execution when Linux cgroup ownership is unavailable because their timeout/error contract requires automatic cleanup of all descendants. `run_bash` may continue in an explicitly reported degraded process-group mode when permission is denied; this mode must not be described as exact containment.

Synchronous `Command` users must use the platform `command_status` or `command_output` helpers, which preserve the registered owner's exit status.

Linux subreaper supervision remains a runtime-wide fallback for descendants whose normal ownership chain is broken. It uses repeated `(PID, process start time)` observations, bounded state, and exact `waitpid`. A fallback process is model-visible but is not attributed to the most recently completed Job. Per-Job code never sweeps all adopted children.

The three layers are intentionally not interchangeable: the Job backend answers membership/control/completion, direct-child registration reserves exit status for the owning supervisor, and the subreaper fallback performs final Unix reaping when no registered owner remains. Job termination does not reap zombies; `waitpid` ownership does not identify or contain a descendant tree.

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
3. stdout and stderr drain threads reached EOF and were joined.

Drain threads run while descendants are alive so pipe backpressure cannot block them. If the launcher dies from a signal or Job-state observation fails, the supervisor terminates that Job before publishing a terminal result.

## Cancellation and shutdown

Cancellation targets only handles selected from the current manager instance. Selection
happens under the index lock; signalling happens after releasing it.

- action cancellation signals its own job, waits for supervisor convergence, then removes it;
- session cancellation signals unfinished jobs owned by that session;
- runtime shutdown signals all unfinished jobs and joins all supervisors;
- dropping the last manager performs the same bounded ownership cleanup;
- a newly constructed manager cannot observe or signal another manager's jobs.

Runtime restart does not read historical job metadata or signal historical PIDs. Known
legacy `shell_jobs` directories are deleted without parsing their contents.

## Output policy

stdout and stderr are always drained concurrently to avoid pipe deadlock. Each stream is
bounded independently to 1 MiB:

- `tail_out=false`: retain the first 1 MiB;
- `tail_out=true`: retain the last 1 MiB.

The terminal result stores separate stdout/stderr plus normalized combined output.

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
