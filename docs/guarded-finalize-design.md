# Guarded Finalize

## Purpose

Final command check is for the narrow case where the model already has a complete
`final_answer`, but wants the runtime to run one final local check before that
answer is shown.

It is not a separate tool protocol. It uses a normal `run_bash` action with a
normal `command` argument.

## Protocol

`status:"finished"` may include one `next_actions` item only when that action is
a final `run_bash` command:

```json
{
  "status": "finished",
  "final_answer": "The task is complete.",
  "next_actions": [
    {
      "action": "run_bash",
      "intent": "Verify the output file exists before finalizing.",
      "args": {
        "command": "test -s output.txt",
        "timeout_ms": 5000
      }
    }
  ]
}
```

Constraints:

- Exactly one action is allowed.
- The action must be `run_bash`.
- `args.command` must be non-empty.
- Background mode is not allowed in final command check. If the model needs more evidence, it must use
  `status:"working"` instead.
- `timeout_ms` is optional and follows the normal shell timeout behavior.

## Runtime Behavior

1. Runtime executes the final command through the same controlled Bash path used
   by normal `run_bash`.
2. Exit code 0 means the guard passed, so runtime shows `final_answer`.
3. Nonzero exit, timeout, approval denial, or command failure means the guard
   failed. Runtime ignores `final_answer`, adds the command result as prompt
   evidence, and asks the model to continue.

The failure prompt slice uses this shape:

```text
Final command check command:
command: <command>
controlled_bash_result:
status: 1
output: ...
verdict: FAIL

Note: 你上轮用 status:finished + final command check command 声明完成，但命令返回非 0。Runtime 已忽略 final_answer，请根据以上命令输出修正后再回复。
```

## Audit

Action audit records:

- `final_command_check_command`
- `final_command_check_command_pass`
- `final_command_check_command_fail`
- `final_command_check_command_needs_user_approval`

## Tests

Required coverage:

- pass: command exits 0 and finalizes without another model round
- fail: command exits nonzero and returns evidence to the model
- approval: bash approval mode still applies
- repair: multiple actions or non-`run_bash` guarded actions are rejected
- downgrade: `status:"finished"` with evidence-gathering actions is converted
  to working and the premature answer is discarded
