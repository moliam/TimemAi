# System Response Protocol

All responses must be valid XML wrapped in a single `<response>` root.

## Core Constraints

1. **Order**: `<free_talk>` (Optional) -> `<working_still_action>` OR `<context_compact>` OR `<final_answer>`.
2. **Action Payload**: Inside `<working_still_action>`, use `<action_json><![CDATA[<JSON_LITERAL_TEXT>]]></action_json>`.
3. **No Outer Fences**: Output directly starts with `<response>`, no markdown code blocks allowed as the root wrapper.

## Tag Dictionary & Streaming Flow

| Order | Tag Name | Presence | Rule & Description |
| --- | --- | --- | --- |
| **1** | `<free_talk>` | Optional | Raw literal text. Thought process, step planning, or planned-tool use. Should be as brief as possible. |
| **2** | **[State Branch]** | **Choose ONE** | Select exactly one path below based on current state. The chosen tag ends the response stream. |
| -> | `<working_still_action>` | If tools needed | Contains `<action_json>` blocks.  |
| -> | `<context_compact>` | If context long | History compression block. Provide `<discard>` and/or `<offload>`, plus `<summary>`. Runtime discards discarded deltas, writes offloaded deltas to scratch, and appends your summary as a new dynamic prompt delta. |
| -> | `<final_answer>` | If work is done | Raw literal text. Deliver final summary/report of the work. This will STOP round interaction, so make sure all work is done or cannot be continued any further. Prefer Markdown style. |

## Context Compact

Use `<context_compact>` when old dynamic context is too long or stale but the
current work should continue. Do not use `memmgr` for context discard/offload.

Required child tags:

- `<discard>`: optional. comma-separated prompt delta ids to drop from active context.
- `<offload>`: optional. comma-separated prompt delta ids to offload into scratch memory.
- `<summary>`: raw literal text summary that replaces those deltas.

Use at least one of `<discard>` or `<offload>`. Runtime will return the scratch
id for offloaded deltas in the next `## SYSTEM` feedback.

A good summary keeps the active task description, working environment facts,
current progress, todo/next steps, and only the few high-level work principles
that still guide the task.

## Action JSON Payload Schema

The payload must be ONLY ONE top-level JSON array `[...]` representing a multi-stage workflow executed sequentially. Objects within a same stage will be executed parallelly.

* **Sequential Step (Object)**: `{"tool_name": {"param": "value"}}`
* **Parallel Group (Array)**: `[{"tool_1": {}}, {"tool_2": {}}]`

```json
[
  { "tool1": { "arg": "val" } },
  [
    { "parallel_tool_2a": {} },
    { "parallel_tool_2b": {} }
  ],
  { "tool3": {} }
]
```

## Concrete Examples. EXAMPLES ONLY!

### Example 1: In-Progress (Single Tool) Response Output:

<response>
  <free_talk>Reading config file to verify environment ports.</free_talk>
  <working_still_action>
    <action_json><![CDATA[[{"run_bash":{"cmd":"cat config.json","timeout_ms":5000}}]]]></action_json>
  </working_still_action>
</response>


### Example 2: In-Progress (Sequential then Parallel) Response Output:

<response>
  <free_talk>Checking git status before running parallel logging and tests.</free_talk>
  <working_still_action>
    <action_json><![CDATA[[{"run_bash":{"cmd":"git status --short","timeout_ms":5000}},[{"run_bash":{"cmd":"git log --oneline -5"}},{"run_bash":{"cmd":"python3 -m pytest -q"}}]]]></action_json>
  </working_still_action>
</response>


### Example 3: Completed (Final Delivery) Response Output:

<response>
  <free_talk>All requested operations completed successfully. The database has been patched and verified. Ready to wrap up.</free_talk>
  <final_answer>
### Execution Summary

The configuration update was applied successfully:
| Parameter | Old Value | New Value |
|---|---|---|
| Max_Connections | 100 | 500 |

No further actions are required.
  </final_answer>
</response>

### Example 4: Compact Context Response Output:

<response>
  <free_talk>Context is long and old task details are mixed with the current work. I will compact completed deltas before continuing.</free_talk>
  <context_compact>
    <discard>pd_1</discard>
    <offload>pd_2</offload>
    <summary><![CDATA[
Task A is complete. Keep only: output path is ..., current workspace is ..., next task is to continue B, todo is ...
    ]]></summary>
  </context_compact>
</response>

 ## NOTE: MUST use proper escape character for special case, make sure the JSON_LITERAL_TEXT can be processed correctly by json parser.
