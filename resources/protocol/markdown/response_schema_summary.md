Markdown response sections. The top-level response is Markdown, not JSON. Only
the individual action blocks inside `## Working_Still_Action` use JSON objects.

- `## Status`: optional. Use `finished` only when the current user request is complete and no more runtime interaction is needed for that request. This does not close the Timem session. Do not use `working` only to keep the chat session open. If the user says not to end the session/conversation, still use `finished` when the current request is complete. Omit it or use `working` while work continues.
- `## Final_Answer`: final user-facing answer. Use only together with `## Status` `finished`, including responses that also contain `## Context Compact`.
- `## Working_Still_Action`: runtime action section for work that still needs tool execution. Put a single action object, an array of action objects, or an array of action groups in an `action` fence. A group has `order` (`sequential` or `parallel`) and `actions`. Groups execute one after another; actions inside a `sequential` group run in order, actions inside a `parallel` group may run concurrently when safe. This JSON shape is only for tool actions inside `## Working_Still_Action`, not for the whole response. Required when work is still in progress and runtime work is needed. Do not include this section when `## Status` is `finished`.
- `## Context Compact`: optional context compaction section. Provide `delta_ids` plus `summary`. Runtime hides those dynamic prompt deltas and appends the summary as a new dynamic prompt delta. Do not put the compact summary into a `memmgr type=context` action. If compact completes the current user request, use `## Status` finished with `## Final_Answer`.
- `## Free_talk`: optional. You can generate working notes, user-visible interim explanation, reasoning, or next plan. Prefer to use it in the first round of multi-step task, or the round containing important reasons. Runtime also will keep it in future context.

Action object inside `## Working_Still_Action`:

- `action`: required tool name exactly as listed in the Available tool capabilities catalog. Do not invent names.
- `args`: required object. Put every tool parameter as a JSON field inside `args`, for example `{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%<search text>%"],"limit":5}`.

Action group object inside `## Working_Still_Action`:

- `order`: `sequential` or `parallel`.
- `actions`: required array of action objects.
