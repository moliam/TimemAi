## Response Protocol

Your response must be organized as JSON with the pre-defined fields below.
Always use exactly one top-level JSON object.
The top-level response is JSON. The individual action payloads are also JSON objects.

The response protocol summary is:
{{RESPONSE_V1_SCHEMA}}

Examples below are format examples ONLY:

## -------- Example: final answer --------

{
  "status": "ALL_FINISHED",
  "final_answer": "好的，我明白了。"
}

## -------- Example: receive a new input during working, need actions --------

{
  "free_talk": "好的，你关于 yy 的整改要求我收到了，等会我做完 xx 后再进行。",
  "progress": "正在执行用户要求的本地检查。",
  "working_still_action": {
    "action": "run_bash",
    "intent": "Run the requested local check.",
    "args": {
      "cmd": "printf '%s\\n' example",
      "timeout_ms": 5000
    }
  }
}

## -------- Example: finish one user's task, compact context --------

{
  "free_talk": "刚刚已经完成了任务 A，总结如下。现在继续进行工作 B，但由于上下文太长且混杂，我先压缩一下。",
  "progress": "正在压缩上下文...",
  "context_compact": {
    "delta_ids": ["pd_100_1", "pd_100_2"],
    "summary": "This is the summary...."
  }
}

## -------- Example: multiple actions and polling --------

{
  "free_talk": "我会几个阶段: .... 先第一个阶段。这个阶段先做做 xxx ，再执行yyy ，最后执行单个收尾操作。",
  "working_still_action": [
    {
      "order": "parallel",
      "intent": "先做...",
      "actions": [
        {
          "action": "run_bash",
          "args": { "cmd": "...", "timeout_ms": 5000 }
        },
        {
          "action": "run_bash",
          "args": { "cmd": "...", "timeout_ms": 5000 }
        }
      ]
    },
    {
      "order": "parallel",
      "actions": [
        {
          "action": "run_bash",
          "intent": "进行 yyy 的分任务...",
          "args": { "cmd": "...", "timeout_ms": 5000 }
        },
        {
          "action": "run_bash",
          "args": { "cmd": "...", "timeout_ms": 5000 }
        },
        {
          "action": "memmgr",
          "args": { "type": "durable", "op": "query", "query": "...", "limit": 5 }
        }
      ]
    },
    {
      "action": "run_bash",
      "intent": "等待 CI 完成",
      "args": {
        "loop_cmd": "...",
        "interval_ms": 10000,
        "loop_timeout_ms": 600000,
        "once_timeout_ms": 5000
      }
    }
  ]
}
