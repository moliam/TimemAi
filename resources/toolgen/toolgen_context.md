[TOOL_GEN_TASK] Please extract the reusable function(such as information extraction, formatted log
search, env config, etc) from the completed task's experience just now as one(could be
multiple as a suite) script tool, define properly input/output, so it can be run directly
in future work. And the user can also use it to do useful work standalone
without you.

Follow the ToolGen repository standard:

- For multiple tools, create one
  semantic subdirectory per tool under that draft directory and publish each
  subdirectory separately.
- Work only in the exact temporary staging directory supplied by runtime. Do not
  modify the user's project while generating the tool.
- Each published tool directory must contain:
  - `README.md`: keep it very short. State what the tool does, a synopsis,
    prerequisites if any, and one standalone usage example.
  - One main script or executable entrypoint. The tool itself must support
    `--help` so users and future model turns can inspect how to run it, and `--self-test` so that it can be tested.
  - `.timem-tool.json`: lightweight repository metadata for indexing and
    publishing: `name`, `type`, `language`, `entrypoint`, `synopsis`, and
    `self_test` with `args` and `timeout_ms`; retain `tool_id` when updating.
- Abstract general reusable inputs and outputs from task-specific completed tasks. Try to make the tools reusable to other or new target.
- When creating multi-line files through `run_bash`, keep the action JSON valid.
  Escape every JSON string quote correctly, or use a short command that runs a
  script to write the files.
- Make every self-test bounded, deterministic, and safe.
- Call `toolgen` with `op=publish` only after a draft is ready. Runtime validates
  the files and independently executes the declared self-test. Correct validation
  failures within the available rounds.
- State which tools were generated or updated. Keep the final answer short; the
  completed task's original final answer remains unchanged.

Example Reference layout for one tool:

log-error-counter/
├── README.md
├── .timem-tool.json
└── count_errors.sh

Minimal `.timem-tool.json`:

    {
      "name": "log-error-counter",
      "type": "log-analysis",
      "language": "bash",
      "entrypoint": "count_errors.sh",
      "synopsis": "count_errors.sh <log-file>",
      "self_test": {
        "args": ["--self-test"],
        "timeout_ms": 5000
      }
    }

Minimal `README.md`:

    # log-error-counter
    Count ERROR lines in one log file.
    Synopsis: `count_errors.sh <log-file>`
    Example: `./count_errors.sh build.log`

Keep `README.md` short. Future turns can search this ToolRepo, inspect the
directory and run the script's `--help` when a tool may be useful.
