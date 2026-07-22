# web_ui Module Boundary

`web_ui` is Timem's browser presentation layer. It uses assistant-ui primitives
for the conversation surface and renders structured host/core events.

It may contain:

- React/assistant-ui components, Markdown and syntax highlighting, responsive
  layout, accessibility, themes, animation, and browser-local preferences.
- Session selection and rename controls, composer behavior, file-picker UI,
  session-scoped inline decision queues, activity rendering, completion telemetry, and context
  compaction presentation.
- Bounded client history and strict session-aware reducers for WebSocket
  events, plus progressive DOM mounting and UI-owned scroll anchoring for long
  conversations.

It must not contain:

- Provider/model networking, prompt or response-protocol parsing, memory/tool
  execution, command approval policy, or audit persistence.
- Reinterpretation of core topic semantics from unstructured strings when a
  shared structured field exists.
- The upstream assistant-ui monorepo as committed source. The ignored vendor
  checkout is only a pinned development reference.

The browser may understand every public topic field and choose its own visual
representation. It must not merge events from different session or request ids.
