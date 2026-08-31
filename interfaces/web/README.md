# Timem Web Interface

`interfaces/web` is Timem's browser Interface. It is a TypeScript application
built on assistant-ui primitives and consumes the authenticated HTTP/WebSocket
contract exposed by the Host. It does not import or call Rust Session or Agent
crates.

## Ownership

The browser owns:

- Session navigation and presentation;
- composer behavior, attachments, next-turn queue input, active-turn supplements,
  and inline decisions;
- rendering of free talk, actions, repairs, context compaction, runtime requests,
  and final answers;
- Markdown, syntax highlighting, telemetry presentation, themes, fonts,
  responsive layout, and accessibility;
- transient UI state such as drafts, panel state, and Role/message drag previews.

The browser does not own model calls, prompt parsing, memory, tools, command
approval policy, Session scheduling, or Turn lifecycle. It does not persist a
command outbox or replay business commands after refresh.

Session groups are selected at Session creation and then fixed. The Host validates
and persists that selection. Sessions restored from older metadata without a
`group_id` appear in the permanent built-in Unsorted group; there is no Session
move interaction. Persisted groups keep creation order and cannot be dragged or
otherwise reordered; they can be deleted only while empty.

## Authoritative state contract

Every WebSocket event is scoped by `session_id`; Context- and Worker-scoped topics
must also match their target scope. The Host's snapshots, projections, and
semantic events are authoritative. Command ACKs report delivery or handling and
do not substitute for business state.

Once the Host accepts work, browser disconnect, refresh, closure, or device lock
does not cancel or transfer that work. Reconnect restores the view from the Host
snapshot baseline and bounded subsequent events.

The reducer and tests cover queued turns, supplements, duplicate pressure,
concurrent Sessions, decisions, attachments, bounded event windows, rendering,
and long-history behavior. Read [`module_boundary.md`](module_boundary.md) and
[Web reliability test matrix](../../docs/web_reliability_test_matrix.md) before
changing this contract.

## Model endpoints

The model label opens a MEM-scoped endpoint library shared by Sessions. Selecting
an endpoint applies its model, API and response protocols, Base URL, API key,
context limit, and output limit to the current idle Session. API keys remain in
the Host MEM with private permissions and are redacted from browser snapshots
and browser-persistent data.

## Development

Install dependencies:

```bash
pnpm --dir interfaces/web install --frozen-lockfile
```

Validate changes:

```bash
pnpm --dir interfaces/web test
pnpm --dir interfaces/web build
git diff --exit-code -- interfaces/web/dist
cargo test -p timem
```

Commit source, tests, applicable lockfile changes, and rebuilt `dist` assets
together. Do not commit `node_modules` or optional vendor checkouts.
