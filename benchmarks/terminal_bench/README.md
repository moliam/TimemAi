# Terminal-Bench evaluation

This directory connects Timem and comparison coding agents to Harbor as
installed agents. Harbor supplies the Terminal-Bench task container, time
limits, and verifier; each adapter supplies its own model loop and tools.

The active comparison adapters are:

- `timem_agent.py`: Timem Shell;
- `pi_agent.py`: Pi Coding Agent;
- `openhands_agent.py`: OpenHands CLI;
- `goose_agent.py`: Goose CLI;
- `aider_agent.py`: Aider;
- `sweagent_agent.py`: SWE-agent;
- `openharness_agent.py`: HKUDS OpenHarness.

For an agent-scaffold comparison, configure every harness with the same model
endpoint and run the exact same task set once (Pass@1). Do not mix
infrastructure failures or zero-model-call trials into the score denominator.

During installation, the adapter also ensures that `curl` and CA certificates
are available in the task container because Timem's model transport invokes
`curl` directly.

The adapter intentionally runs one fresh `--once-json` turn per task with:

- automatic Bash approval;
- work-instruction loading disabled, so repository-local instructions do not
  contaminate benchmark prompts;
- isolated task-local Timem data;
- `TIMEM_MAX_ROUNDS=300` for unattended evaluation. The product default remains
  50, where it opens an interactive continue prompt. In `--once-json`, the
  `NoopTurnUi` host automatically accepts that decision, retains the task
  context, and recharges another 300 rounds, so 300 is a boundary rather than a
  hard agent-step cap;
- the model configuration supplied through `TIMEM_*` environment variables.

All Timem results from the earlier max-50 campaign are excluded as a unit.
Timem starts all 89 tasks again under max 300 for each reported model; this
avoids both UI-truncated failures and selective reruns of only failed tasks.
The official per-task agent timeout is unchanged. A run that exhausts that
wall-clock timeout is unrelated to the auto-continued round boundary.

The Linux binary is expected at:

```text
.benchmark-cache/linux-target/release/timem-native-rs
```

Set `TIMEM_BENCH_BINARY` to use a binary built elsewhere, such as a server-side
release build outside the Git worktree.

Example Harbor command (credentials must be supplied privately in the process
environment, never committed):

```bash
harbor run \
  --dataset terminal-bench@2.0 \
  --agent benchmarks.terminal_bench.timem_agent:TimemShellAgent \
  --model gpt-5.6-sol \
  --allow-agent-host 10.125.112.83/32 \
  --n-concurrent 1 \
  --n-tasks 1 \
  --yes
```

The resulting score measures the combined system
`Timem scaffold + configured model`; it is not a model-only or agent-only score.

`run_full_server.py` builds official task Dockerfiles when a published image is
not locally available, records resumable JSONL progress, and retries unscored
infrastructure failures on the next invocation. `summarize_runs.py` aggregates
Pass@1, Wilson confidence intervals, tokens, agent duration, exceptions,
category, and difficulty after the runs finish.

`run_matrix_server.py` shares each official task image across independent
containers for Timem, Pi, OpenHands, Goose, Aider, SWE-agent, and OpenHarness.
All seven target all 89 Terminal-Bench 2.0 tasks with the same configured model
endpoint. Agent/task combinations in each image batch share one global worker
pool, so a long trial does not leave the other concurrency slots idle. The
runner holds an exclusive lock in its matrix work directory, so a dropped SSH
connection cannot lead to two matrices mutating the same progress files.
`start_matrix_server.py` starts that runner in a new server-side session and
appends output to `full-matrix-shared-20260815/runner.log`; model credentials
are inherited in memory and are not written to the log or command line.

`process_cleanup.py` wraps every installed-agent command, records the root of
its in-container process tree, and terminates that tree before the adapter
returns. This is required because cancelling a host-side Docker exec can leave
the container process alive while Harbor starts the verifier. A timeout is
scoreable only when `benchmark-process-cleanup.json` proves cleanup completed;
all historical timeouts without that marker are treated as infrastructure
trials and rerun regardless of whether their verifier reward was zero or one.
Deterministic provider safety refusals are different: they prove a real model
request but represent a combined agent-model capability failure, so they remain
scoreable verifier outcomes and are not retried as transport failures.

Pass@1 always selects the first infrastructure-valid scored trial for an
agent/task. A later retry cannot replace or improve it. At each runner pass
boundary, missing ledger entries are reconstructed from the earliest valid
immutable Harbor job directory before any task is scheduled; this makes a
runner crash between Harbor completion and `progress.jsonl` append recoverable
without creating a second attempt.

`evaluation_manifest.json` pins the dataset revision, Harbor and agent
versions, model label, architecture, concurrency, and scoring protocol without
including the private endpoint or credential. The final summary must be built
with `summarize_runs.py --require-complete`; that gate rejects the report unless
every agent has 89 valid trials and every pair has the same 89 common tasks.

The controlled campaign consists of two sequential matrices:

1. all seven agents × all 89 tasks with `gpt-5.6-sol`;
2. the same seven pinned agents × the same 89 tasks with `kivy-glm-5_2`.

`queue_second_model.py` keeps the second model configuration in server-process
memory and starts it only after the GPT matrix passes its 7×89 completeness
gate. `compare_model_summaries.py` then requires 89 common tasks for the same
agent across both models and produces paired deltas and exact McNemar tests.
`render_dual_model_report.py` refuses to render if either within-model matrix or
the cross-model matrix is incomplete. `finalize_campaign.py` waits for both
matrices and automatically creates the two summaries, two per-model reports,
the cross-model JSON, the combined `RESULTS.md`, and SHA-256 checksums.

`official_leaderboard_snapshot.json` records a selected, dated snapshot of the
historical Terminal-Bench 2.0 leaderboard for industry context. Those rows use
five trials per task and different model/agent versions, so they are not mixed
into the controlled rankings. The current official benchmark is Terminal-Bench
2.1; its community submissions are closed as of the snapshot date. This k=1
Terminal-Bench 2.0 campaign is therefore not represented as submission-ready.

The Timem manifest records both the base Git revision and the exact
`timem-max-rounds-300.patch`, plus source and Linux binary SHA-256 hashes. This
distinguishes the benchmark binary from an unmodified checkout while preserving
the product default of 50 rounds outside the evaluation adapter.
