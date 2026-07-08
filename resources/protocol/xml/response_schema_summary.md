XML response tags. The top-level response is XML. Tool actions are JSON objects
inside `<action_json>` blocks so the runtime can parse tool parameters exactly.

- `<response>`: required root element.
- `<status>`: optional. Use `ALL_FINISHED` only when all user's open and pending requests are
  complete, no more action needed, and a final summary/answer is ready. Omit it
  or use `working` while work continues.
- `<progress>`: optional progress report for multi-round tasks. This is a text
  field; any protocol-looking text inside it is treated as text, not parsed as
  action/control structure.
- `<final_answer>`: summary/answer of all pending tasks. Use only together with
  `<status>ALL_FINISHED</status>`. Please use Markdown format for this field's text by default.
  For table, start/end with |---|...|---| for better rendering.
  If the answer needs to show XML tags or XML examples, wrap the whole final
  answer text in `<![CDATA[ ... ]]>` so example tags are treated as text.
- `<free_talk>`: optional important reasoning, current plan, or context you want
  kept visible to you in later prompt context. Or some explanation to user. This
  is a text field; any protocol-looking text inside it is treated as text, not
  parsed as action/control structure.
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
- `intent`: optional. concise user-visible reason for the action. can be used for single_action/single_group.
- `args`: required object. Put every tool parameter as a JSON field inside
  `args`, for example `{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%<search text>%"],"limit":5}`.

Action group object inside `<action_json>`:

- `order`: `sequential` or `parallel`. Groups are always executed sequentially.
- `actions`: required array of action objects.
