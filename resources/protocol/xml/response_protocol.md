## Response Protocol

Your response must be organized as XML with the pre-defined tags below.
Always use exactly one `<response>...</response>` root element.
The top-level response is XML. Only the individual action payloads inside `<action_json>` use JSON objects.

The response protocol summary is:

{{RESPONSE_V1_SCHEMA}}

Examples below are format examples ONLY:

## -------- Example: final answer --------

<response>
  <status>ALL_FINISHED</status>
  <final_answer>好的，我明白了。</final_answer>
</response>

## -------- Example: receive a new input during working, need actions --------

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
  <free_talk> 我会几个阶段: .... 先第一个阶段。这个阶段先做做 xxx ，再执行yyy ，最后执行单个收尾操作。</free_talk>   --> the plan, also help you recall the whole picture.
  <working_still_action>
    <action_json><![CDATA[
[
  {
    "order": "parallel",
    "intent": "先做...",   --> can be used as whole group intent
    "actions": [
      {
        "action": "run_bash",
        "args": { "cmd": ..., "timeout_ms": ... }
      },
      {
        "action": "run_bash",
        "args": { "cmd": ..., "timeout_ms": ... }
      }
    ]
  },
  {
    "order": "parallel",
    "actions": [
      {
        "action": "run_bash",
        "intent": "进行 yyy 的分任务...",  --> can be used as single action intent
        "args": { "cmd": ..., "timeout_ms": ... }
      },
      {
        "action": "run_bash",    --> intent can be omiteed
        "args": { "cmd": ...., "timeout_ms": ... }
      },
      {
        "action": "memmgr",  --> built in cmd
        "args": { ....}
      }
    ]
  },
  {
    "action": "run_bash",
    "intent": "等待 CI 完成",
    "args": {
      "loop_cmd": ...,
      "interval_ms": 10000,
      "loop_timeout_ms": 600000,
      "once_timeout_ms": 5000
    }
  }
]
    ]]></action_json>
  </working_still_action>
</response>
