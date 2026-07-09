XML response tags. The top-level response is XML. Tool actions are JSON objects
inside `<action_json><![CDATA[...]]></action_json>` blocks so the runtime can
parse tool parameters exactly.

Required output shape:

1. `<response>` root.
2. Optional `<free_talk>` visible working note.
3. Exactly one state branch:
   - `<working_still_action>` when more tools are needed.
   - `<status>ALL_FINISHED</status>` followed by `<final_answer>` when all
     active/pending user prompts are complete.
   - `<context_compact>` when context must be compacted.

Text fields:

- `<free_talk>`, `<final_answer>`, and context compact `<summary>`
  are text fields. If they need to contain literal XML tags or XML examples,
  wrap the whole text in `<![CDATA[...]]>`.
- `<final_answer>` contains the final Markdown response to the user. Use only
  with `<status>ALL_FINISHED</status>`.

Actions:

- `<working_still_action>` contains one or more `<action_json>` blocks.
- Each `<action_json>` block contains raw JSON, not markdown fences.
- JSON may be a single action object, an action group object, or an array of
  action/group objects.
- Action object fields: `action` required, `args` required object.
- Group object fields: `order` is `parallel` or `sequential`; `actions` is a
  required array of action objects. Workflow array entries execute in array
  order; inside each group, `order` controls whether actions run in parallel or
  sequentially.
