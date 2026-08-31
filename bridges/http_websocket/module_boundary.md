# HTTP/WebSocket Bridge Boundary

`bridges/http_websocket` owns communication state needed between a reconnectable browser Interface
and Core. It may depend inward on `core/ui_contract` and other Core crates, but it must not become a
UI layer or a second owner of Agent, Session, or Turn semantics.

## Current layout

- `src/lib.rs`: revisioned delivery of authoritative Core Turn projections and a bounded,
  deduplicated FIFO for future-turn command delivery.
- `src/routes.rs`: fixed browser HTTP/WebSocket paths, HTTP method placement, global and
  endpoint-specific request-body bounds, static fallback routing, and the browser-wide security
  response layer. Product hosts inject authenticated handlers and state; the Bridge does not build
  snapshots, authorize users, dispatch commands, or own Session/Turn state.
- `src/transport.rs`: Axum WebSocket frame splitting, bounded JSON text decoding/encoding,
  same-origin request validation, and browser-safe response headers. It does not authenticate a
  product, build snapshots, dispatch commands, or own Session/Turn state.
- Unit tests prove route/method ownership, request bounds, monotonic revisions, monotonic revisions, duplicate/stale rejection, queue bounds, ordering,
  serialization round trips, bounded frame decoding, same-origin validation, and response-header
  policy used by reconnectable clients.

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
