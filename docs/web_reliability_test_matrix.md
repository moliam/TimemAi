# Web UI / host / core reliability test matrix

This matrix defines the observable invariants for commands crossing the browser,
web host, and agent core boundary.  It deliberately distinguishes a browser
socket write from an accepted command and from a durable committed effect.

## Live delivery stages

| Stage | Meaning | UI behavior after disconnect |
| --- | --- | --- |
| socket write | The browser called `WebSocket.send()` once on an open, snapshot-ready connection. This does not prove Host receipt. | Do not persist, retry, or display a per-command waiting state. |
| `accepted` | The current Host process reserved the correlation ID and queued/started handling it. This is transport control, not business success. | Do not replay. Reconnect from a fresh authoritative snapshot. |
| `committed` | Host completed command handling and the corresponding authoritative state/event, when any, is available. | Update business UI only from the projection/event, not from the ACK. |
| `rejected` | Host explicitly rejected this live command. | Show the explicit error and let the user choose whether to act again. |

`WebSocket.send()` is not evidence for either Host receipt or a committed Core/Session effect. An ACK is correlated live control data; it does not replace the authoritative projection.

## UI continuity and disconnect baseline

A live browser connection is part of the product experience contract, not an implementation detail. While the Host process remains alive and the transport/protocol is valid, ordinary UI and runtime activity must preserve one WebSocket connection. The browser must not close or rebuild it merely because React rerendered, a user clicked or switched Sessions, a queue projection changed, or Thought/Action/tool progress arrived in a burst.

An automatic disconnect is justified only by observable transport failure, Host shutdown, authentication failure, malformed/incompatible protocol data, or a sequence condition that cannot be recovered by an authoritative snapshot baseline. Broadcast lag is recoverable and must first establish a fresh baseline without showing a false runtime failure.

The executable browser baseline must assert all of the following:

- initial and reconnect `hello` snapshots adopt the Host's exact non-zero `event_cursor`;
- the next semantic event is accepted as `event_cursor + 1`, without a gap report or reconnect;
- at least 32 consecutive Thought/Action-style progress events stay on the same socket;
- ordinary commands, queue updates, Session state updates, rerenders, and Session switching do not increase the WebSocket connection count;
- no `Runtime event gap`, `Runtime error`, `Runtime disconnected`, or connection-lost banner appears during valid traffic;
- one genuine failure produces one bounded notice per disconnect episode rather than a notification storm;
- reconnect restores authoritative working/terminal/queue state without browser command replay.

A test that checks only the eventual DOM state is insufficient: it must also observe connection count, close/reconnect behavior, and user-visible error notices.

## Required executable cases

| Case | Fault injection point | Required invariant |
| --- | --- | --- |
| Disconnect before Host receipt | Close the socket after the browser write while preventing Host dequeue. | No browser replay occurs. If Host did not receive the command, no effect occurs; reconnect restores the current snapshot. |
| Disconnect after Host acceptance | Close after `accepted`, before handler completion. | Host handling already owned by the live process may finish independently of the socket; reconnect observes only the resulting authoritative snapshot. |
| Disconnect after commit | Drop the terminal ACK. | The ACK loss does not roll back the domain effect and does not trigger browser retry; reconnect snapshot/event state remains authoritative. |
| Queue full | Fill the bounded command queue, then send one more command. | A correlated `rejected` ACK names the rejected ID; queue memory stays within its configured capacity. |
| Duplicate command in one process | Submit the same `command_id` concurrently while its dedup record remains resident. | Handler invocation is one; callers receive the resident result/control response. This guarantee does not cross Host restart or cache eviction. |
| Distinct commands | Submit distinct IDs with identical payloads. | Both are distinct user actions; payload equality must not deduplicate them. |
| Dedup capacity | Fill the process-local cache with accepted records. | New IDs are explicitly rejected, record count stays at `COMMAND_DEDUP_CAPACITY`, and no disk ledger/file is created. Terminal records are evictable. |
| Snapshot handshake | Mutate authoritative state while a connection is taking its initial snapshot. | Client observes the mutation either in the snapshot or in a sequenced event, never neither. |
| Command/event ordering | Make Core emit immediately while `turn_submit` is committing. | The turn-creation state precedes an event referencing it, or the reducer waits for an authoritative state that can own it; timestamp order alone is not treated as causality. |
| Broadcast lag | Overrun the bounded live channel. | Host supplies a new full snapshot baseline; it does not grow or replay a disk event log. |
| Pending decision reconnect | Disconnect while Core is waiting for a request reply. | The reconnect snapshot recreates the authoritative decision and `request_id`; Core remains answerable. |
| Duplicate decision click | Trigger two synchronous clicks. | The browser event guard suppresses the duplicate in that event window; later explicit actions are new commands. |
| Cancel then supplement | Race cancellation with a new user message under seeded jitter. | Core/Host ownership fields decide whether the message belongs to the active Turn or a distinct next task; arrival timestamps do not decide it. |
| Supplement then final | Seal the model request PromptCut, then accept a supplement while finalization proceeds independently. | PromptCut and terminal ownership decide whether the supplement was consumed or becomes a next task; visible final timing does not. |
| Final then immediate | Click **immediate** after authoritative final commit but before UI receives it. | Host classifies against its authoritative Turn generation, not the stale browser picture. |
| Process restart | Restart after persisted Session state exists. | Running/unfinished work restores according to Session interruption rules; browser commands and generic dedup records are not redriven. |
| Persistence failure | Fail authoritative Session/history persistence after validation. | No successful projection is published for state that was not persisted; explicit errors remain visible. |
| Two browser tabs | Issue mutations against one Session from separate sockets. | Host lane/revision rules serialize or reject them; browser cross-tab storage is not a command bus. |
| Memory-space switch | Accept work while switching memory spaces. | The MEM epoch barrier prevents old-epoch execution in the new space and resets process-local dedup state without creating per-MEM command fragments. |

### Pressure profile

The cases above define the target certification, but the four concentrated stress gates are **not implemented yet**: PromptCut/final ownership, Stop/Start storm, reconnect/FIFO ownership, and release-Chrome latency. The intended first three gates use independent execution sides, barriers for exact windows, seeded jitter, and at least 300 PR / 1,000 release / 10,000 soak iterations. The intended Chrome gate repeatedly drives the real WebSocket/Host/Core path and reports command-correlated p50/p95/p99/max latency.

Until those executable gates exist, focused deterministic race tests, the two-pass edge regression, Web Host tests, TTY stories, and Chrome Stop acceptance are strong partial evidence, not the full pressure-profile certification. Future stress runs must assert exact command/attachment ownership, bounded queues, immutable outcomes, no revived working state, and resource convergence; failures must print a replayable seed and named stage trace. Increasing sleeps or timeouts, lowering iterations, or checking only the final ready state is not acceptable remediation.

### Authoritative Core handoff

Persisting a visible user entry and handing work to Core are distinct facts. Their relationship must be represented by the existing bounded Session/Turn ownership model, not inferred from write order and not duplicated into a generic ever-growing command ledger.

- Browser delivery is one-shot and has no durable outbox.
- `core_accepted` is an authoritative, bounded Session/Turn handoff field for work already owned by Core. It is not a browser delivery state, generic command ledger, or instruction to redrive after reconnect/restart.
- Host command queues, accepted ownership sets, event channels, and dedup records are hard-bounded.
- Generic command dedup is process-local only; it neither loads nor writes `web_command_dedup.json`.
- Durable user-visible work uses the existing authoritative Session store and its bounded next-turn FIFO. Do not create one fragment file per command/event.
- A Host/Core restart is a hard execution boundary: unfinished running work restores as interrupted and generic browser commands are not automatically redriven.
- Strict exactly-once behavior cannot be promised for irreversible external effects across process or machine failure. Such effects require command-specific idempotency or reconciliation; generic transport ACKs cannot prove execution ownership.
- Reconnect and restore tests cover four Sessions concurrently and assert bounded, isolated Session/Turn state without browser command replay or cross-talk.

## Browser one-shot command cases

- Send only when the socket is open and the initial snapshot is ready.
- Do not enqueue in localStorage/sessionStorage, retry on reconnect, synchronize through `storage`, or restore pending commands after refresh.
- Do not show `Sending…`, `Waiting for confirmation…`, or `Retrying…` for an individual command.
- Keep the previous authoritative business UI until a Host projection/event changes it.
- Ignore `accepted` as a business-state transition. A correlated `rejected` response may show an explicit error.
- A short synchronous event guard may suppress one accidental duplicate click, but it must not become persistent business state or unbounded memory.
- If a command did not reach Host, it did not happen; the user may explicitly perform a new action.

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

`timem::semantic_delivery::OrderedEventDelivery` supplies the in-memory
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

The suite keeps explicit regressions for the snapshot/subscription gap,
uncorrelated command errors, missing reconnect decisions, discarded late
supplements, cross-socket Session ordering, unbounded caches/queues, accidental
command-state files, and memory-switch races. Correlated live acknowledgements,
ordered semantic delivery, snapshot-baseline recovery, bounded FIFO Session
lanes, bounded process-local deduplication, and the memory epoch barrier remain
required. Browser persistence/replay is intentionally prohibited.
