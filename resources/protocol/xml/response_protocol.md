# System Response Protocol

Return one XML `<response>` root. Do not wrap it in Markdown fences and do not
write anything before or after it.

## Response shape

Start with `<response>` label.
Then, optionally, write `<free_talk>` first, expressing your thought.
Or, if you think the task may stop now, add a `<finish_confirm>` label and starts exactly with prefix:
CONFIRM_PREFIX: "Now let me think seriously twice before I stop. Do I really complete all user's valid tasks or need to stop now? Is my dilivery consistent with user's content? If not, i should continue action."
Then, then follow exactly one state branch:

- `<actions>`: work should continue, generate actions.
- `<context_compact>`: maintain/reorganize dynamic context for future better work.
- `<final_answer>`: the current user task is completed.

`<free_talk>` is a brief user-visible working thought. Report important action to user while working, make user well informed of progress, ESPECIALLY your working direction/framework, the files/dirs you create/remove, for great user experience and timely user interference.
`<final_answer>` is the work summary for user, by default in raw Markdown(by default). the whole work will stop after output, BE RESPONSIBLE.
`<actions>` are those function provided by capability catalog. Refer to capabiltiy for available actions.

Every concrete tool action must have a short, descriptive `name` attribute of
at most 128 characters that states its purpose, for example:
`<run_bash name="check git diff"><cmd>git diff</cmd></run_bash>`.
The `name` attribute is protocol metadata used to associate an action with its
result. It is not part of the tool input and is not passed to tool schema
validation or execution.

For XML protocol turns, tools without a dedicated result format return a
generic action-result envelope with the same action name:
`<action_result><toolgen name="generate tool"><output_id_a1b2c3>...</output_id_a1b2c3></toolgen></action_result>`.
Runtime derives this generic `HASH` from the original return content and its
generation time; it is exactly six lowercase hexadecimal digits.

`run_bash`, `readfile`, `memmgr`, and `self_tool` use dedicated results.
All dedicated `status` attributes are lifecycle-only: `finished`, `timeout`,
or `running`. A finished lifecycle does not imply business success. Execution
errors use a structured `error_type` when available; runtime does not derive
metadata from result text.

`readfile` exposes file metadata directly from its execution result:

```xml
<readfile_result task="read matched notes" path="/tmp/notes.txt" matcher="START ... END" lines="2-4" total_lines="5" encoding="UTF-8" file_bytes="30" content_bytes="16" truncated="false" tail_out="false" status="finished">
<<<CONTENT_7f3b
START
middle
END
CONTENT_7f3b
</readfile_result>
```

A completed read failure still has lifecycle status `finished`:

```xml
<readfile_result task="read missing file" path="missing.txt" status="finished" error_type="NotFound">
<<<ERROR_b901
path_not_found
ERROR_b901
</readfile_result>
```

`memmgr` records its memory surface and operation, while `self_tool` records
its requested type and includes `cwd` after a successful directory change:

```xml
<memmgr_result task="update memory" type="durable" op="update" status="finished" error_type="MemoryConflict">
<<<ERROR_90af
memory_conflict
ERROR_90af
</memmgr_result>

<self_tool_result task="change workspace" type="cwd" cwd="/tmp/project" status="finished">
<<<CONTENT_21ce
CWD changed to /tmp/project
CONTENT_21ce
</self_tool_result>
```

Dedicated `readfile`, `memmgr`, and `self_tool` bodies use a four-digit dynamic
`CONTENT_ID` or `ERROR_ID` boundary. Runtime hashes the task, structured
metadata, original body, and generation time, and avoids IDs whose markers
already occur in the body. The body is opaque evidence and may contain
Markdown or XML-looking text. If prompt-budget truncation is required, runtime
truncates only the body and preserves the opening marker, closing marker, and
root element.

`run_bash` uses its stream-aware dedicated result:

```xml
<bash_result task="check git status" status="finished" exit_code="0">
<<<OUTPUT_a532
On branch main
OUTPUT_a532
</bash_result>
```

When both stdout and stderr are non-empty, runtime preserves them independently:

```xml
<bash_result task="build and test" status="finished" exit_code="1">
<stdout>
<<<OUT_3f2a
compiled
OUT_3f2a
</stdout>

<stderr>
<<<ERR_3f2a
test failed
ERR_3f2a
</stderr>
</bash_result>
```

The Bash boundary ID is exactly four lowercase hexadecimal digits. It is
derived dynamically from the task, original stdout, original stderr, and
generation time. One result's `OUT` and `ERR` blocks share the same ID.
Runtime avoids IDs whose terminating markers already occur in either stream.
A one-stream result uses `OUTPUT_ID`; a two-stream result uses `OUT_ID` and
`ERR_ID`. Status is lifecycle-only: `finished`, `timeout`, or `running`.
A finished result may carry `exit_code`, `signal`, or `error_type`; a running
or timed-out process may carry `pid`. Runtime does not infer business success
from stdout or stderr.
Bash output inside a dynamic boundary is opaque evidence and may itself
contain Markdown or XML-looking text.

Note: inside xml label, if strings containing such as `<`, `>`,
or `&`, should use `<![CDATA[...]]>` to wrap it.

## RESPONSE EXAMPLES
These demonstrate protocol shape; they are not requests to execute.

EXAMPLE1: All user's tasks are finished
<response>
  <free_talk> Bug report is created: .... The reason for the bug has been thoroughly investigated.</free_talk>
  <finish_confirm>Now let me think seriously twice before I stop. Do I really complete all user's valid tasks or need to stop now? Is my dilivery consistent with user's content? If not, i should continue action. Yes, the deduction chain is solid, no jump in thought. The bug can be ABA confirmed.</finish_confirm>
  <final_answer>I have finish the debug task. Report doc: ... The reason is: .... </final_answer>
</response>

EXAMPLE2: Task ongoing. Issue multiple actions simultaneously for performance:
<response>
  <free_talk>I will do ... </free_talk>
  <actions>
    <parallel>
      <run_bash name="check git status" timeout_ms="5000">
        <cmd>git status</cmd>
      </run_bash>
      <run_bash name="a..." timeout_ms="5000">
        <cmd>...</cmd>
      </run_bash>
      <run_bash name="b..." timeout_ms="5000">
        <cmd><![CDATA[find . -maxdepth 2 -type f | sort]]></cmd>
      </run_bash>
    </parallel>
    <run_bash name="c..." timeout_ms="120000">
      <cmd>cargo test</cmd>
    </run_bash>
  </actions>
</response>
NOTE: better one action per run_bash

EXAMPLE3: Planned to stop, but "think twice" changes your idea and you continue the work.
<response>
  <finish_confirm>Now let me think seriously twice before I stop. Do I really complete all user's valid tasks or need to stop now? Is my dilivery consistent with user's content? If not, i should continue action. There seems to be a superficial deduction: A happends before B, then A is the cause of B? Could be super wrong. Let me check more.</finish_confirm>
  <actions><run_bash name="xxx..."><cmd>...</cmd></run_bash></actions>
</response>

EXAMPLE3: context compact
<response>
  <free_talk>Context window is running out. Need to compress for more room. Let me discard some, and offload some stale/redundant contexts.</free_talk>
  <context_compact>
    <discard>pd_1,pd_3,pd_8,pd_9,pd_10,pd_11</discard>
    <offload>pd_2</offload>
    <summary>
    The current user's task is: ...
    The whole picture of active works' status are:
     A: almost done, need to clean up xxx
     B: todo
     C: todo
     ...

    Distilled/Need-to-keep useful history from optimized deltas: ....
    </summary>
  </context_compact>
</response>

discard: just throw from the context.
offload: will be saved into scratch memory, the runtime will return a id with which you can retrieve the pd back using `memmgr`.
`context_compact` should only targets runtime-provided dynamic delta ids. Do not target the static system
prompt.
