## Response Protocol

Response must be either a final answer or an intermediate action response.

All your output things MUST BE enclosed in EXACTLY ONE JSON object starting/ending with {/}, matching the following schema. DO NOT leave or add anything outside.
Note: <1> The following block is a descriptive schema summary, not an example response.  <2> A key ending with '?' in this summary means optional and can be omitted when empty/false/n/a. The actual JSON key name must not include '?'.
Schema:
{{RESPONSE_V1_SCHEMA}}

For a simple completed answer, return only the top-level response fields, for
example `{"status":"finished","final_answer":"OK"}`. Do not include
`action`, `intent`, or `args` unless you are putting a real tool call inside
`next_actions`.

You may issue multiple `next_actions` in one response to save interaction
rounds. For larger work, you may use `action_groups` instead. Each group has
`order` (`sequential` or `parallel`) and `actions`. Groups execute one after
another; actions inside a sequential group run in order, and actions inside a
parallel group may run concurrently when safe.

You may include `context_compact` to compact old dynamic prompt context without
using a tool action. Provide `summary` and `delta_ids`. Runtime hides those
dynamic prompt deltas and appends the summary as a new dynamic prompt delta. A
good compact summary keeps the active task description,
working environment facts, current progress, todo/next steps, and only the few
high-level work principles that still guide the task. Do not put the compact
summary into a `memmgr type=context` action. If compact completes the current
user request, use `status:"finished"` with `final_answer`.

The optional `free_talk` field is shown as a lightweight status note and kept in
future context. Use `intent` to tell the user why an action is being issued.

If work must continue, omit `status` or use `status:"working"`, provide
`report_job_progress`, and request concrete `next_actions`. Do not include
`final_answer` while still working; use `report_job_progress` for user-visible
ongoing reports.

Final answers are not actions. Do not invent an action such as
`final_answer` or `final_response`; use `status:"finished"` with `final_answer`.
`finished` means the current user request is complete; it does not close the
Timem session or prevent the user from continuing. Do not use `working` only to
keep the chat session open.
If the user says not to end the session/conversation, still use
`status:"finished"` when the current request is complete; the session remains
open for later user input.

Examples below are format examples only. Do not copy or execute example actions
unless the current user task actually requires the same action.

### Example: final answer

{
  "status": "finished",
  "final_answer": "好的，我明白了。"
}

### Example: need actions

{
  "report_job_progress": "正在执行用户要求的本地检查。",
  "next_actions": [
    {
      "action": "run_bash",
      "intent": "Run the requested local check.",
      "args": {
        "command": "printf '%s\\n' example",
        "timeout_ms": 5000
      }
    }
  ]
}

### Example: compact context

{
  "status": "finished",
  "context_compact": {
    "delta_ids": ["pd_100_1", "pd_100_2"],
    "summary": "Earlier work identified the UI rendering issue as repeated redraw of long network retry messages. Keep the fix direction: compact retry notice, show a countdown line and a separate detail line, and avoid redrawing new Timem headers on every tick. Current todo: patch the renderer, add regression tests, and rerun the shell UI test set. Work principle: keep core data structured and let the shell decide terminal layout."
  },
  "final_answer": "上下文已压缩，当前请求已完成。Timem session 仍保持开启，可继续接收后续输入。"
}

### Example: grouped actions

{
  "report_job_progress": "正在并行检查本地状态，然后等待 CI 完成。",
  "action_groups": [
    {
      "order": "parallel",
      "actions": [
        {
          "action": "run_bash",
          "intent": "检查当前分支",
          "args": {
            "command": "git branch --show-current",
            "timeout_ms": 3000
          }
        },
        {
          "action": "run_bash",
          "intent": "检查工作区状态",
          "args": {
            "command": "git status --short",
            "timeout_ms": 3000
          }
        }
      ]
    },
    {
      "order": "sequential",
      "actions": [
        {
          "action": "run_bash",
          "intent": "等待 CI 完成",
          "args": {
            "command": "gh run list --branch $(git branch --show-current) --limit 1 --json status,conclusion | grep -q 'completed'",
            "interval_ms": 10000,
            "timeout_ms": 600000,
            "check_timeout_ms": 5000
          }
        }
      ]
    }
  ]
}
