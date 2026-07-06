## Response Protocol

Your response must be organized as XML with the pre-defined tags below.

The top-level response is XML, not JSON or Markdown. Only the individual action
payloads inside `<action_json>` use JSON objects.

Required tag rules:

- Always use exactly one `<response>...</response>` root element.
- If work is still in progress, omit `<status>` or write
  `<status>working</status>`, provide `<progress>` when useful, and include
  concrete actions when runtime work is needed. Do not write `<final_answer>`
  while still working; use `<progress>` for user-visible ongoing reports.
- If the task is complete, write `<status>ALL_FINISHED</status>` and provide
  `<final_answer>`. `ALL_FINISHED` means all pending user tasks are completed.
- Any response containing `<final_answer>` must also contain
  `<status>ALL_FINISHED</status>`, including responses that also contain
  `<context_compact>`.
- Final answers are not actions.
- `<free_talk>` is optional. Use it for casual reasoning, next plans, or context
  that should remain visible to you in later prompt context. Runtime keeps it
  for you in future context.
- `<working_still_action>` contains one or more `<action_json>` blocks. Each
  `<action_json>` block contains JSON for a single action object, an array of
  action objects, or an array of action groups. Wrap JSON in CDATA when it
  contains quotes, angle brackets, shell punctuation, or multi-line content.
  DO NOT include `<working_still_action>` when `<status>` is `ALL_FINISHED`.
- `<context_compact>` lets you replace old dynamic context with a concise
  summary. Provide `<delta_ids>` plus `<summary>`. Runtime will hide the
  referenced dynamic prompt deltas and append your summary as a new dynamic
  prompt delta. A good compact summary keeps the active task description,
  working environment facts, current progress, todo/next steps, and only the few
  high-level work principles that still guide the task. Do not put the compact
  summary into a `memmgr type=context` action. If compact completes the current
  user request, use `<status>ALL_FINISHED</status>` with `<final_answer>`.

The response protocol summary is:

{{RESPONSE_V1_SCHEMA}}

Examples below are format examples ONLY:

## -------- Example: final answer --------

<response>
  <status>ALL_FINISHED</status>
  <final_answer>好的，我明白了。</final_answer>
</response>

## -------- Example: receive a new input and need actions --------

<response>
  <free_talk>好的，你关于 yy 的整改要求我收到了，等会我做完 xx 后再进行。</free_talk>
  <progress>正在执行用户要求的本地检查。</progress>
  <working_still_action>
    <action_json><![CDATA[
{
  "action": "run_bash",
  "intent": "Run the requested local check.",
  "args": {
    "cmd": "printf '%s\\n' example",
    "timeout_ms": 5000
  }
}
    ]]></action_json>
  </working_still_action>
</response>

## -------- Example: receive a user task, plan, and start doing --------

<response>
  <free_talk>这个任务我将会分成几个步骤进行，下面先进行目录浏览。</free_talk>
  <working_still_action>
    <action_json><![CDATA[
{
  "action": "run_bash",
  "intent": "浏览当前目录的文件",
  "args": {
    "cmd": "ls -al",
    "timeout_ms": 1000
  }
}
    ]]></action_json>
  </working_still_action>
</response>

## -------- Example: finish one user's task, compact context --------

<response>
  <free_talk>刚刚已经完成了任务 A，总结如下。现在继续进行工作 B，但由于上下文太长且混杂，我先压缩一下。</free_talk>
  <progress>正在压缩上下文...</progress>
  <context_compact>
    <delta_ids>pd_100_1, pd_100_2</delta_ids>
    <summary><![CDATA[
This is the summary....
    ]]></summary>
  </context_compact>
</response>

## -------- Example: multiple actions and polling --------

<response>
  <free_talk>我会先并行检查两个本地状态，然后轮询等待外部状态就绪。</free_talk>
  <working_still_action>
    <action_json><![CDATA[
[
  {
    "order": "parallel",
    "actions": [
      {
        "action": "run_bash",
        "intent": "检查当前分支",
        "args": {
          "cmd": "git branch --show-current",
          "timeout_ms": 3000
        }
      },
      {
        "action": "run_bash",
        "intent": "检查工作区状态",
        "args": {
          "cmd": "git status --short",
          "timeout_ms": 3000
        }
      }
    ]
  },
  {
    "action": "run_bash",
    "intent": "等待 CI 完成",
    "args": {
      "loop_cmd": "gh run list --branch $(git branch --show-current) --limit 1 --json status,conclusion | grep -q 'completed'",
      "interval_ms": 10000,
      "loop_timeout_ms": 600000,
      "once_timeout_ms": 5000
    }
  }
]
    ]]></action_json>
  </working_still_action>
</response>
