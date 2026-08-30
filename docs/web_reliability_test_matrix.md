# Web UI / host / core reliability test matrix

This matrix defines the observable invariants for commands crossing the browser,
web host, and agent core boundary.  It deliberately distinguishes a browser
socket write from an accepted command and from a durable committed effect.

## Delivery stages

| Stage | Meaning | UI behavior after disconnect |
| --- | --- | --- |
| `pending` | Command exists in the browser outbox but has not been acknowledged. | Retain and retry with the same `command_id`. |
| `accepted` | Host owns the command and promises either a terminal `committed` or `rejected` result. | Retain; reconnect and query/retry with the same `command_id`. |
| `committed` | The non-idempotent effect is durable and recoverable by snapshot/event replay. | Remove from the outbox. |
| `rejected` | No effect will be applied for this `command_id`. | Retain user content and expose a retry/edit action. |

`WebSocket.send()` is not evidence for either `accepted` or `committed`.

## Required executable cases

| Case | Fault injection point | Required invariant |
| --- | --- | --- |
| Disconnect before accept | Close the socket after the browser write but before host dequeue. | UI retains the command and a reconnect retry applies it once. |
| Disconnect after accept | Close after `accepted`, before the handler finishes. | Host finishes the owned command; retry returns its terminal result without re-execution. |
| Disconnect after commit | Drop the terminal ack. | Retry with the same ID returns `committed`; effect count remains one. |
| Queue full | Fill the bounded command queue, then send one more command. | A correlated `rejected` ack names the rejected `command_id`; no unrelated command is cleared. |
| Duplicate command | Submit the same `command_id` concurrently on one and two sockets. | Handler invocation and durable effect count are exactly one; all callers receive the same terminal result. |
| Distinct commands | Submit distinct IDs with identical payloads. | Both commands execute in the server-defined order; payload equality must not deduplicate user intent. |
| Snapshot handshake | Mutate state while a connection is taking its initial snapshot. | Client observes the mutation either in the snapshot or in a sequenced event, never neither. |
| Command/event ordering | Make Core emit immediately while `turn_submit` is committing. | The turn-creation state precedes any event referencing that turn, or the client buffers it until the turn exists. |
| Broadcast lag | Overrun the live channel and reconnect from the last acknowledged sequence. | Every semantic event after that sequence is replayed exactly once. |
| Pending decision reconnect | Disconnect while Core is waiting for a request reply. | The reconnect snapshot/replay recreates the same decision and `request_id`; Core remains answerable. |
| Duplicate decision click | Send the same reply command twice and lose the first ack. | Core resolves the request once; retry reports the recorded terminal result. |
| Cancel then supplement | Race cancellation with a new user message on separate sockets and independent Host/Core threads under seeded jitter. | The message is PromptCut-consumed by the active turn or committed as the next task; it is never inferred from arrival order and never silently cleared. |
| Supplement then final | Seal the model request PromptCut, then accept a supplement while that independent model thread sleeps/finalizes. | The current response cannot claim the unconsumed supplement; terminal commit atomically hands it back as a queued next task with the same command ownership. |
| Final then immediate | Click **immediate** after final state is committed but before UI receives it. | Host classifies by its authoritative turn generation, creating a new task rather than writing to a finished turn. |
| Process crash | Crash after command persistence and before Core handoff. | Recovery replays the durable host outbox into Core without duplicating the user entry. |
| Multi-Session restore | Restore four Sessions concurrently, each with one task and ordered supplements. | All four Sessions enter Core in parallel as isolated atomic batches; no prompt, command ID, completion, or delivery state crosses Session scope. |
| Persistence failure | Fail history/session persistence after validation. | No `committed` ack is emitted and no irreversible Core effect is started. |
| Two browser tabs | Issue mutations against one session from separate sockets. | Per-session revision/order is authoritative at the host; stale mutations are rejected or serialized. |
| Memory-space switch | Accept work on another socket while switching memory spaces. | The switch is an epoch barrier: it rejects active work, prevents new acceptance, and no old-epoch command can execute against the new space. |

### Pressure profile

The cases above are requirements, but the high-risk Turn boundary is certified by four concentrated stress tests rather than one shallow test per row: PromptCut/final ownership, Stop/Start storm, reconnect/FIFO ownership, and release-Chrome latency. The first three use independent execution sides, barriers for exact windows, seeded jitter, and at least 300 PR / 1,000 release / 10,000 soak iterations. The Chrome scenario repeatedly drives the real WebSocket/Host/Core path and reports command-correlated p50/p95/p99/max latency.

Every run asserts exact command/attachment ownership, bounded queues, immutable outcomes, no revived working state, and resource convergence. A failure must print its seed and named stage trace. Increasing sleeps or timeouts, lowering iterations, or checking only the final ready state is not acceptable remediation.

### Durable Core handoff

Persisting a user entry is not proof that Core received it. A task or
supplement therefore needs a durable handoff state separate from its visible
history entry:

- `recorded`: user text and `command_id` are durable, but Core delivery is not
  yet proven;
- `core_accepted`: the current Core worker has accepted the item, keyed by the
  same stable ID. This is a process-local delivery fact, not proof that an
  external tool side effect completed.

After a process crash, unfinished `recorded` and `core_accepted` items must be
redelivered with the same ID. A restored task and its ordered supplements are
redelivered as one atomic initial batch so an immediately final model response
cannot close the mailbox between task and supplement recovery. Core deduplicates
IDs within one process; recovery intentionally favors not losing the user's
work.
Finding `command_id` in chat history alone must not turn a retry into
`committed`, because a crash can occur after the history write and before the
mailbox enqueue (or after turn creation and before `run_turn`).

Strict exactly-once behavior cannot be promised for an irreversible external
tool side effect across a process or machine crash. Such tools must provide
their own idempotency key or reconciliation contract. Timem's recoverable turn
delivery is at-least-once at that boundary.

A terminal dedup-record write failure after a domain effect is also not a
`rejected` command: rejection promises that no effect happened. Keep ownership
non-terminal and retry the commit record, or atomically transact domain state
and the terminal command record.

## UI outbox cases

- Automatic queued dispatch must claim a message without removing it.  Only a
  matching `committed(command_id)` removes it.
- `accepted` leaves the row visible as sending; reconnect does not release it
  into a second distinct command ID.
- `rejected` and connection loss leave text, attachments, editing state, and
  queue position recoverable.
- Ack for command A must never release, delete, or change command B.
- Reordering or deleting an unsent row cannot change the identity of an
  already accepted row.
- Refresh/reload restores pending and accepted commands from durable browser
  storage, not React component state.

## Core-to-UI cases

All semantic Core events require a stable `event_id` and monotonic replay
cursor.  This includes request decisions, action lifecycle changes, tool
results, final answers, turn completion, cancellation, and errors.  Render
windows may be bounded, but truncating a render window must not advance the
delivery cursor past an event that was never reduced into authoritative UI
state.

Purely client-derived animation frames are not Core messages and may be
coalesced.

### Ordered delivery and snapshot recovery

`timem_web::semantic_delivery::OrderedEventDelivery` supplies the in-memory
linearization point. The WebSocket integration follows this order:

1. Subscribe to live publication.
2. Capture the current delivery sequence before constructing the snapshot.
3. Send `hello(snapshot, event_cursor)`; that cursor is the connection baseline.
4. Discard buffered envelopes at or below the baseline and continue with the
   strictly next sequence.
5. On a sequence gap or broadcast lag, send a new full snapshot baseline (or
   reconnect and receive one). There is no persisted event replay cursor.

Capturing the baseline before the snapshot can make a buffered event redundant
with snapshot state, but cannot omit a post-baseline event. Ordered semantic
envelopes retain the wire field `event_seq`; reducers classify duplicates and
gaps from that sequence and remain idempotent for duplicate stable event IDs.
Sequence allocation and broadcast occur in
one short mutex critical section, so concurrent publishers cannot expose N+1
before N. The common connected path performs no semantic-delivery filesystem
I/O; uncommon reconnect, gap, and lag recovery pay for a full snapshot.

Use ordered envelopes for authoritative event classes:

- session created, renamed, deleted, stopped, and runtime/config updates;
- turn created/updated, user task and supplement acceptance;
- Core topic and worker activity, including request decisions and tool results;
- attachment added/removed, ToolRepo mutation, and MCP mutation state;
- final answer, cancellation/error, and turn completion.

Keep request-scoped or sensitive replies direct:

- command acknowledgements and validation errors;
- API-key and MCP-secret reveal responses;
- history/search/detail query responses and terminal-open results;
- `hello` snapshots.

Every mutation result returned to one requesting socket must also use the
ordered authoritative publication path so other connected tabs converge. The
publication path is downstream of authoritative persistence and cannot turn an
already committed mutation into a rejection.

## Historical failures protected by regression tests

The suite keeps explicit regressions for the earlier snapshot/subscription gap,
socket-disconnect command loss, uncorrelated command results, premature UI queue
deletion, missing reconnect decisions, discarded late supplements, cross-socket
Session reordering, history-before-Core crash windows, and memory-switch races.
Removing the durable outbox, correlated acknowledgements, Core delivery state,
ordered semantic delivery, snapshot-baseline recovery, FIFO Session lanes, or
memory epoch barrier requires a replacement that passes the same cases.
