[TOOL_GEN_TASK] Please preserve the reusable work module(such as information extraction, log
search, env config, etc) from the completed task just now as one(could be
multiple) script tool, define properly input/output, so it can be run directly
in future work. And the user can also use it to do useful work standalone
without you.

Follow the ToolGen repository standard:

- Generate one or multiple tools when the completed work contains independent
  reusable modules. Do not merge unrelated interfaces into one script. For one
  tool, use the supplied draft directory itself. For multiple tools, create one
  semantic subdirectory per tool under that draft directory and publish each
  subdirectory separately.
- Work only in the exact temporary staging directory supplied by runtime. Do not
  modify the user's project while generating the tool.
- Each published tool directory must contain:
  - `README.md`: a short purpose, synopsis, prerequisites, input/output contract,
    and one standalone usage example.
  - `.timem-tool.json`: `name`, `type`, `language`, `entrypoint`, `synopsis`, and
    `self_test` with `args` and `timeout_ms`; retain `tool_id` when updating.
  - The executable entrypoint and only the supporting files genuinely needed.
- Define stable inputs and outputs. Remove machine-specific absolute paths,
  secrets, transient task data, and assumptions that only the model can satisfy.
- Make every self-test bounded, deterministic, and safe.
- Call `toolgen` with `op=publish` only after a draft is ready. Runtime validates
  the files and independently executes the declared self-test. Correct validation
  failures within the available rounds.
- A tool is ready only after runtime returns `status: ready`.
- State which tools were generated or updated. Keep the final answer short; the
  completed task's original final answer remains unchanged.

Reference layout for one tool:

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
    Input: a readable log path. Output: one integer followed by a newline.
    Example: `./count_errors.sh build.log`

Minimal `count_errors.sh`:

    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "${1:-}" == "--self-test" ]]; then
      sample=$(mktemp)
      trap 'rm -f "$sample"' EXIT
      printf 'INFO ok\nERROR first\nERROR second\n' > "$sample"
      [[ "$(grep -c ERROR "$sample")" == "2" ]]
      exit
    fi
    [[ $# -eq 1 ]] || { echo "usage: count_errors.sh <log-file>" >&2; exit 2; }
    grep -c ERROR "$1" || [[ $? -eq 1 ]]

Keep `README.md` short: state the purpose, synopsis, prerequisites, input,
output, and one standalone example. The model defines the self-test in the
manifest; runtime executes it independently before publication.
