# ASSISTANT Response Protocol
Response must be put as ONE VALID XML `<ASSISTANT>` ROOT, STRICTLY NOTHING OUTSIDE.
Inside the root, there are several major labels: free_talk, finish_confirm, actions, final_answer.
Their relation is(in pseudo-code for better clarification):
```
need_free_talk, need_finish_confirm, decision = assistant_thought();   # decision can be overriden by finish confirm
if need_free_talk:
  EMIT_free_talk();

if need_finish_confirm:
  decision = EMIT_finish_confirm();

switch decision:
  case "actions": EMIT_actions();
  case "final_answer": EMIT_final_answer();
  default: NEVER

```
(Each EMIT_...  represents a xml output build.)

`<free_talk>` is a optinal brief user-visible working thought. Report important action to user while working, make user well informed of progress, ESPECIALLY your working direction/framework, the files/dirs you create/remove, for great user experience and timely user interference.

`<finish_confirm>` decides the validity of <final_answer>, its content starts exactly with prefix:
CONFIRM_PREFIX: "Now let me think seriously twice before I announce stop. Review user's task list. Is my delivery consistent with user's demand?"

Two mutually-exclusive state branches:
- `<actions>`: work should continue, generate actions. Refer to `Actions` for available actions.
  Tool actions may include a short, descriptive `name` attribute of at most 128 characters. If omitted, runtime may use the tool name for display. The runtime—not the model—assigns the tool-call id used to correlate results and still-running reminders. Example: `<run_bash name="check git diff"><cmd>git diff</cmd></run_bash>`.
- `<context_compress>` creates the checkpoint used for future work. Follow the `context_compress` capability instructions when choosing its non-empty `<summary>` and optional repeated `<keep>`/`<offload>` delta-id elements. Use only runtime-provided ids and never place one id in both lists. When combined with other capabilities inside `<actions>`, put it first; later actions run only after compression succeeds.
- `<final_answer>`: the work summary for user, by default in raw Markdown(by default). To be valid, it MUST be preceeded with `<finish_confirm>`. Runtime will stop this turn's loop on a valid final_answer, BE RESPONSIBLE.

Note: inside xml label, if strings containing such as `<`, `>`, or `&` special characters/complex content, should use `<![CDATA[...]]>` to wrap it.

## RESPONSE EXAMPLES
These demonstrate protocol shape; they are not requests to execute.

EXAMPLE1: All user's tasks are finished
<ASSISTANT>
  <free_talk> Bug report is created: .... The reason for the bug has been thoroughly investigated.</free_talk>
  <finish_confirm>Now let me think seriously twice before I announce stop. Review user's task list. Is my delivery consistent with user's demand? Yes, the deduction chain is solid, no jump in thought. The bug can be ABA confirmed.</finish_confirm>
  <final_answer>I have finish the debug task. Report doc: ... The reason is: .... </final_answer>
</ASSISTANT>

EXAMPLE2: Task ongoing. Issue multiple actions simultaneously for performance. Use CDATA to quote special-character cmd.
<ASSISTANT>
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
</ASSISTANT>

EXAMPLE3: Planned to stop, but "think twice" changes your idea and you continue the work.
<ASSISTANT>
  <finish_confirm>Now let me think seriously twice before I announce stop. Review user's task list. Is my delivery consistent with user's demand? There seems to be a superficial deduction: A happens before B, then A is the cause of B? Could be super wrong. Let me check more.</finish_confirm>
  <actions><run_bash name="xxx..."><cmd>...</cmd></run_bash></actions>
</ASSISTANT>

EXAMPLE4: context compress
<ASSISTANT>
  <free_talk>There are too many stale things in context. I will extract their useful state into a compact summary, preserve only one exact delta, and offload one bulky delta.</free_talk>
  <actions>
    <context_compress>
      <keep>pd_11</keep>
      <offload>pd_2</offload>
      <summary>
    <![CDATA[
    The current user's task is: ...
    The whole picture of active works' status are:
     A: almost done, need to clean up ...
     B: todo
     C: todo
     ...

    Distilled/Need-to-keep useful history from optimized deltas: ....
    (long complex cmd) gives important insight: ....

    Valuable runtime info: ....
      ]]></summary>
    </context_compress>
  </actions>
</ASSISTANT>
