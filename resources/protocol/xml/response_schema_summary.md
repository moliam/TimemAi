XML response tags. The top-level response is XML. Tool actions are JSON objects
inside `<action_json><![CDATA[...]]></action_json>` blocks so the runtime can
parse tool parameters exactly.

Required output shape:

1. `<response>` root.
2. Optional `<free_talk>` visible working note.
3. Exactly one state branch:
   - `<working_still_action>` when more tools are needed.
   - `<context_compact>` when context must be compacted.
   - `<final_answer>` when all active/pending user prompts are complete.

Text fields:

- `<free_talk>`, `<final_answer>`, and context compact `<summary>`
  are raw text fields. Extract them as text, not as nested protocol.
- `<final_answer>` contains the final Markdown response to the user.

Actions:

- `<working_still_action>` contains one or more `<action_json>` blocks.
- Each `<action_json>` block contains the JSON payload directly. CDATA is
  recommended so string values can safely contain punctuation, Markdown, or XML-
  looking text.
- The JSON payload must be a top-level array.
- A single action object inside the array is `{ "tool_name": { ...tool parameters... } }`.
- A direct array of action objects inside the outer array is one parallel group.
- Entries execute in array order; inner arrays execute in parallel.
- Do not use `action`/`args` fields or `{ "order": "...", "actions": [...] }`
  group objects.
