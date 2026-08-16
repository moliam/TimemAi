# System Response Protocol

Return one XML `<response>` root. Do not wrap it in Markdown fences and do not
write anything before or after it.

## Response shape

Start with `<response>` label.
Then, optionally, write `<free_talk>` first, expressing your thought.
Then, if you think the task may stop now, add a `<finish_confirm>` label and starts exactly with prefix:
CONFIRM_PREFIX: "Now let me think seriously twice before I stop. Do I really complete all user's valid tasks or need to stop now? Is my dilivery consistent with user's content? If not, i should continue action."
Then, follow exactly one state branch:

- `<actions>`: work should continue, generate actions.
- `<context_compact>`: maintain/reorganize dynamic context for future better work.
- `<final_answer>`: the current user task is completed.

`<free_talk>` is a brief user-visible working thought.
`<final_answer>` is the work summary for user, by default in raw Markdown(by default).
`<actions>` are those function provided by capability catalog. Refer to capabiltiy for available actions.

Note: inside xml label, if strings containing such as `<`, `>`,
or `&`, should use `<![CDATA[...]]>` to wrap it.

## RESPONSE EXAMPLES
These demonstrate protocol shape; they are not requests to execute.

EXAMPLE1: All user's tasks are finished
<response>
  <free_talk>The reason for the bug has been thoroughly investigated.</free_talk>
  <finish_confirm>Now let me think seriously twice before I stop. Do I really complete all user's valid tasks or need to stop now? Is my dilivery consistent with user's content? If not, i should continue action. Yes, the deduction chain is solid, no jump in thought. The bug can be ABA confirmed.</finish_confirm>
  <final_answer>I have finish the debug task. The reason is: .... </final_answer>
</response>

EXAMPLE2: Task ongoing. First inspect in parallel, then run one sequential test:
<response>
  <free_talk>I will inspect the workspace, then run its test. For performance serveral commands can be issued simutaneously. </free_talk>
  <actions>
    <parallel>
      <run_bash timeout_ms="5000">
        <cmd>pwd</cmd>
      </run_bash>
      <run_bash timeout_ms="5000">
        <cmd>git status --short</cmd>
      </run_bash>
      <run_bash timeout_ms="5000">
        <cmd><![CDATA[find . -maxdepth 2 -type f | sort]]></cmd>
      </run_bash>
    </parallel>
    <run_bash timeout_ms="120000">
      <cmd>cargo test</cmd>
    </run_bash>
  </actions>
</response>

EXAMPLE3: Planned to stop, but "think twice" changes your idea and you continue the work.
<response>
  <finish_confirm>Now let me think seriously twice before I stop. Do I really complete all user's valid tasks or need to stop now? Is my dilivery consistent with user's content? If not, i should continue action. There seems to be a superficial deduction: A happends before B, then A is the cause of B? Could be super wrong. Let me check more.</finish_confirm>
  <actions><run_bash><cmd>pwd</cmd></run_bash></actions>
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
     A: completed,
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
