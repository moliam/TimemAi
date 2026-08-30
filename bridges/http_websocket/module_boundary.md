# HTTP/WebSocket Bridge Boundary

`bridges/http_websocket` owns communication state needed between a reconnectable browser Interface
and Core. It may depend inward on `core/ui_contract` and other Core crates, but it must not become a
UI layer or a second owner of Agent, Session, or Turn semantics.

## Current layout

- `src/lib.rs`: revisioned delivery of authoritative Core Turn projections and a bounded,
  deduplicated FIFO for future-turn command delivery.
- Unit tests prove monotonic revisions, duplicate/stale rejection, queue bounds, ordering, and
  serialization round trips used for reconnect recovery.

## Placement rule

- Pure semantic command, event, and projection data belongs in `core/ui_contract`.
- HTTP/WebSocket ordering, serialization, replay, reconnect, deduplication, backpressure, and
  delivery metadata belongs in this Bridge.
- Browser rendering and human interaction belong in `interfaces/web`.

## Prohibitions

- Do not define an architectural `host` layer or restore a `host_projection` package.
- Do not invent or reinterpret Turn identity, admission, activity, cancellation, or outcome.
- Do not depend on `timem_web` or any Interface crate; dependency direction remains inward.
- Do not make in-process Interfaces pay serialization, networking, or reconnect overhead.
