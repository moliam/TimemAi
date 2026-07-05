## Response Protocol

Your response must be organized as a markdown chaptered with pre-defined names as below.

The top-level response is Markdown, not JSON. Only the individual action blocks
inside `## Intermediate_Actions` use JSON objects.

Required section rules:

- If work is still in progress, omit `## Status` or write `## Status` with
  `working`, provide progress when useful, and include concrete actions when
  runtime work is needed. Do not write `## Final_Answer` while still working; use
  `## Progress` for user-visible ongoing reports.
- If the task is complete, write `## Status` with `finished` and provide
  `## Final_Answer`. `finished` means the current user request is complete; it does
  not close the Timem session or prevent the user from continuing. Do not use
  `working` only to keep the chat session open.
- Any response containing `## Final_Answer` must also contain `## Status` with
  `finished`, including responses that also contain `## Context Compact`.
- If the user says not to end the session/conversation, still use `finished`
  when the current request is complete; the session remains open for later
  user input.
- Final answers are not actions.
- `## Free_talk` is optional. Use it for casual reasoning, next plans, or
  context that should remain visible to you in later prompt context. Runtime
  keeps it for you in future context. User may input many questions in a turn, you can use
  free talk to answer intermediately and keep working.
- `## Intermediate_Actions` contains one or more action objects. Each action object must
  match the tool catalog exactly.
  DO NOT include `## Intermediate_Actions` when `## Status` is `finished`.
- `## Context Compact` lets you replace old dynamic context with a concise
  summary. Provide delta_ids plus a summary. Runtime will hide the referenced
  dynamic prompt deltas and append your summary as a new dynamic prompt delta. A
  good compact summary keeps the active task description, working
  environment facts, current progress, todo/next steps, and only the few
  high-level work principles that still guide the task. Do not put the compact
  summary into a `memmgr type=context` action. If compact completes the current
  user request, use `## Status` finished with `## Final_Answer`.

The response protocol summary is:

{{RESPONSE_V1_SCHEMA}}

Examples below are format examples ONLY:

## -------- Example: final answer --------

## Status
finished

## Final_Answer
好的，我明白了。

## -------- Example: receive a new input and need actions --------

## Free_Talk
好的，你关于yy的整改要求我收到了，等会我做完 xx 后再进行

## Progress
正在执行用户要求的本地检查。

## Intermediate_Actions
```action
{
  "action": "run_bash",
  "intent": "Run the requested local check.",
  "args": {
    "command": "printf '%s\\n' example",
    "timeout_ms": 5000
  }
}
```

## -------- Example:  receive a user task, plan, and start doing --------

## Free_Talk
这个任务我将会分成 ..... 几个步骤进行，下面先进行..

## Intermediate_Actions
```action
{
  "action": "run_bash",
  "intent": "浏览当前目录的文件",
  "args": {
    "command": "ls -al",
    "timeout_ms": 1000
  }
}
```


## -------- Example: finish one of user's tasks, compact context --------

## Free_Talk
刚刚已经完成了任务 A，总结如下：
输出位于....
现在继续进行工作B。但由于上下文太长且混杂我先压缩一下

## Progress
正在压缩上下文...

## Context Compact
delta_ids: pd_100_1, pd_100_2
summary:
Earlier work identified the UI rendering issue as repeated redraw of long
network retry messages. Keep the fix direction: compact retry notice, show a
countdown line and a separate detail line, and avoid redrawing new Timem
headers on every tick. Current todo: patch the renderer, add regression tests,
and rerun the shell UI test set. Work principle: keep core data structured and
let the shell decide terminal layout.
