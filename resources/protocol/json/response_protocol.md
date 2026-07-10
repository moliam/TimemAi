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
  "working_still_action": {
    "run_bash": {
      "cmd": "printf '%s\\n' example",
      "timeout_ms": 5000
    }
  }
}

## -------- Example: finish one user's task, compact context --------

{
  "free_talk": "刚刚已经完成了任务 A，总结如下。现在继续进行工作 B，但由于上下文太长且混杂，我先压缩一下。",
  "context_compact": {
    "delta_ids": ["pd_100_1", "pd_100_2"],
    "summary": "This is the summary...."
  }
}

## -------- Example: multiple actions and polling --------

{
  "free_talk": "我会几个阶段: .... 先第一个阶段。这个阶段先做做 xxx ，再执行yyy ，最后执行单个收尾操作。",
  "working_still_action": [
    [
      { "run_bash": { "cmd": "...", "timeout_ms": 5000 } },
      { "run_bash": { "cmd": "...", "timeout_ms": 5000 } }
    ],
    [
      { "run_bash": { "cmd": "...", "timeout_ms": 5000 } },
      { "run_bash": { "cmd": "...", "timeout_ms": 5000 } },
      { "memmgr": { "type": "durable", "op": "sql", "sql": "SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5", "params": ["%...%"], "limit": 5 } }
    ],
    {
      "run_bash": {
        "loop_cmd": "...",
        "interval_ms": 10000,
        "loop_timeout_ms": 600000,
        "once_timeout_ms": 5000
      }
    }
  ]
}
