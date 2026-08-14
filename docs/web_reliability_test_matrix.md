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
| Cancel then supplement | Race cancellation with a new user message on separate sockets. | The message is either committed to the active turn and consumed, or committed as the next task; it is never silently cleared. |
| Supplement then final | Accept a supplement while the model is finalizing. | Finalization consumes it in another model round or atomically hands it back as a queued next task. |
| Final then immediate | Click **immediate** after final state is committed but before UI receives it. | Host classifies by its authoritative turn generation, creating a new task rather than writing to a finished turn. |
| Process crash | Crash after command persistence and before Core handoff. | Recovery replays the durable host outbox into Core without duplicating the user entry. |
| Persistence failure | Fail history/session persistence after validation. | No `committed` ack is emitted and no irreversible Core effect is started. |
| Two browser tabs | Issue mutations against one session from separate sockets. | Per-session revision/order is authoritative at the host; stale mutations are rejected or serialized. |
| Memory-space switch | Accept work on another socket while switching memory spaces. | The switch is an epoch barrier: it rejects active work, prevents new acceptance, and no old-epoch command can execute against the new space. |

### Durable Core handoff

Persisting a user entry is not proof that Core received it. A task or
supplement therefore needs a durable handoff state separate from its visible
history entry:

- `recorded`: user text and `command_id` are durable, but Core delivery is not
  yet proven;
- `enqueued`: Core owns the item, keyed by the same stable ID;
- `consumed`: Core incorporated it into a model round or returned it to the
  Host as explicitly unconsumed.

After a process crash, `recorded` and `enqueued` items must be redelivered with
the same ID. Core must deduplicate that ID when it has already consumed it.
Finding `command_id` in chat history alone must not turn a retry into
`committed`, because a crash can occur after the history write and before the
mailbox enqueue (or after turn creation and before `run_turn`).

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

### Sequenced journal integration

`timem_web::event_journal::EventJournal` supplies the durable cursor and replay
primitive. The WebSocket integration must follow this order:

1. Subscribe to live publication.
2. Capture the journal cursor **before** constructing the snapshot.
3. Send `hello(snapshot, event_cursor)`.
4. Replay journal entries with `event_seq > event_cursor`.
5. Continue live delivery, discarding only entries whose sequence was already
   sent. A reconnect supplies its last reduced sequence and replays after it.

Capturing the cursor before the snapshot can duplicate an event that the
snapshot already reflects, but cannot omit it. The UI reducer must therefore be
idempotent by `event_seq`/stable event ID. Capturing the cursor after a snapshot
is unsafe unless snapshot state and journal publication share one transaction.

Journal and broadcast these authoritative event classes:

- session created, renamed, deleted, stopped, and runtime/config updates;
- turn created/updated, user task and supplement acceptance;
- Core topic and worker activity, including request decisions and tool results;
- attachment added/removed, ToolRepo mutation, and MCP mutation state;
- final answer, cancellation/error, and turn completion.

Keep request-scoped or sensitive replies direct and outside the journal:

- command acknowledgements and validation errors;
- API-key and MCP-secret reveal responses;
- history/search/detail query responses and terminal-open results;
- initial hello snapshots.

Every mutation result currently returned only to the requesting socket must
also use the authoritative publication path. A direct result may still be sent
for latency, but it cannot be the only notification because other connected
tabs and reconnecting clients must converge.

## Known pre-hardening failures

- The server sends the initial snapshot before subscribing to the broadcast
  channel, leaving a snapshot-to-subscribe loss window.
- Browser commands have no durable ownership boundary; disconnect aborts the
  per-socket command worker and may discard queued accepted work.
- Command results and Core broadcasts use independent select branches and have
  no shared sequence, so a turn event can arrive before its `TurnUpdated`.
- Queued UI messages are removed immediately after `WebSocket.send()`.
- Reconnect clears UI decisions, while the snapshot does not reconstruct every
  outstanding Core request.
- A structured Core stop closes and drains the supplement mailbox; the current
  caller discards those unconsumed messages even if the host already recorded
  them as active-turn supplements.
- Command ordering is per WebSocket, not per session across WebSockets.
- A persisted turn `command_id` currently conflates visible history with proof
  of Core delivery, leaving a crash window that can acknowledge never-executed
  work.
- Memory-space switching checks for active sessions before the switch but does
  not yet exclude commands accepted concurrently on another socket or queued
  under the old memory epoch.
