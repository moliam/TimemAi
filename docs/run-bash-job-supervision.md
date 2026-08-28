# `run_bash` in-process job supervision

## Goals

`run_bash` jobs belong only to the current runtime instance. They are not persisted,
restored, adopted, or inferred from historical PIDs after restart.

The implementation must provide one authoritative lifecycle per spawned process group:

- one supervisor owns and reaps the Bash launcher;
- stdout and stderr are drained concurrently into bounded buffers;
- descendants in the managed process group remain cancellable after the launcher exits;
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
- signal its process group;
- wait on its condition variable;
- claim a terminal result through the permitted delivery path.

No OS process query or signal is performed while the manager index lock is held.

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
2. stdout and stderr drain threads reached EOF and were joined;
3. the runtime-created process group no longer contains a live process.

If the launcher dies from a signal, the remaining process group is killed to prevent a
signalled launcher from leaving unmanaged descendants.

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
- real Web Stop/cancel flows in addition to manager-level tests.
