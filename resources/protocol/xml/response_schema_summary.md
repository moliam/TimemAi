XML response tags. The top-level response is XML. Tool actions are JSON objects
inside `<action_json>` blocks so the runtime can parse tool parameters exactly.

- `<response>`: required root element.
- `<status>`: optional. Use `ALL_FINISHED` only when the current user request is
  complete and no more runtime interaction is needed for that request. Omit it
  or use `working` while work continues.
- `<progress>`: optional progress report for multi-round tasks.
- `<final_answer>`: summary/answer of all pending tasks. Use only together with
  `<status>ALL_FINISHED</status>`. Use structured markdown text by default.
- `<free_talk>`: optional important reasoning, current plan, or context you want
  kept visible to you in later prompt context. Or some explanation to user.
- `<working_still_action>`: action section for work that still needs
  tool execution. Put one or more `<action_json><![CDATA[{...}]]></action_json>`
  blocks inside it. The JSON content may be a single action object {}, a group of
  action by array objects [{}{}], or multiple groups [{}{}][{}{}].
- `<context_compact>`: optional context compaction block. Include `<delta_ids>`
  with comma-separated prompt delta ids and `<summary>` with the compacted
  state. Runtime hides those dynamic prompt deltas and appends the summary as a
  new dynamic prompt delta.

Action object inside `<action_json>`:

- `action`: required tool name exactly as listed in the Available tool
  capabilities catalog. Do not invent names.
- `intent`: required concise user-visible reason for the action.
- `args`: required object. Put every tool parameter as a JSON field inside
  `args`, for example `{"type":"durable","op":"query","query":"<search text>","limit":5}`.

Action group object inside `<action_json>`:

- `order`: `sequential` or `parallel`. Groups are always executed sequentially.
- `actions`: required array of action objects.
