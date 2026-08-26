"""Run integration smoke trials for the newly added comparison harnesses."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import subprocess
import time
from pathlib import Path

from benchmarks.terminal_bench.run_full_server import MODEL_ENV_KEYS, result_summary


AGENTS = {
    "openhands": "benchmarks.terminal_bench.openhands_agent:OpenHandsAgent",
    "goose": "benchmarks.terminal_bench.goose_agent:GooseAgent",
    "aider": "benchmarks.terminal_bench.aider_agent:AiderAgent",
    "sweagent": "benchmarks.terminal_bench.sweagent_agent:SWEAgent",
    "openharness": "benchmarks.terminal_bench.openharness_agent:OpenHarnessAgent",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--concurrency", type=int, default=2)
    parser.add_argument("--agent", action="append", choices=sorted(AGENTS))
    parser.add_argument("--run-suffix", default="")
    return parser.parse_args()


def asset_environment(root: Path) -> dict[str, str]:
    env = os.environ.copy()
    env.update(
        {
            "PYTHONPATH": str(root / "adapter"),
            "TIMEM_BENCH_BINARY": str(root / "build/release/timem-native-rs"),
            "PYTHON_BENCH_ARCHIVE": str(root / "tools/archives/python-3.12.12.tar.gz"),
            "AIDER_BENCH_ARCHIVE": str(root / "tools/archives/aider-0.86.0-site.tar.gz"),
            "OPENHANDS_BENCH_ARCHIVE": str(root / "tools/archives/openhands-1.13.0-site.tar.gz"),
            "OPENHARNESS_BENCH_ARCHIVE": str(root / "tools/archives/openharness-0.1.9-site.tar.gz"),
            "SWEAGENT_BENCH_ARCHIVE": str(root / "tools/archives/swe-agent-1.1.0-site.tar.gz"),
            "SWEAGENT_SOURCE_ARCHIVE": str(root / "tools/archives/swe-agent-1.1.0-source.tar.gz"),
            "GOOSE_BENCH_ARCHIVE": str(root / "tools/goose-1.46.0.tar.gz"),
        }
    )
    return env


def run_one(
    root: Path,
    label: str,
    agent_import: str,
    env: dict[str, str],
    run_suffix: str,
) -> dict[str, object]:
    jobs_dir = root / "jobs/comparison-smoke-open-source"
    job_name = f"smoke-{label}{run_suffix}"
    job_dir = jobs_dir / job_name
    work_dir = root / "new-agent-smokes-20260815"
    jobs_dir.mkdir(parents=True, exist_ok=True)
    work_dir.mkdir(parents=True, exist_ok=True)
    if (job_dir / "result.json").is_file():
        summary = result_summary(job_dir, agent_import=agent_import)
        if summary["status"] == "scored":
            return {"agent": label, **summary, "reused": True}
        job_name = f"{job_name}-retry-{int(time.time())}"
        job_dir = jobs_dir / job_name
    command = [
        str(root / "bin/harbor"),
        "run",
        "--path",
        str(root / "dataset/regex-log"),
        "--agent",
        agent_import,
        "--model",
        os.environ["TIMEM_MODEL"],
        "--allow-agent-host",
        "10.125.112.83/32",
        "--n-concurrent",
        "1",
        "--yes",
        "--jobs-dir",
        str(jobs_dir),
        "--job-name",
        job_name,
    ]
    with (work_dir / f"{label}.log").open("w") as log:
        completed = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
    if not (job_dir / "result.json").is_file():
        return {
            "agent": label,
            "status": "infra_error",
            "return_code": completed.returncode,
            "job_dir": str(job_dir),
        }
    return {
        "agent": label,
        **result_summary(job_dir, agent_import=agent_import),
        "return_code": completed.returncode,
    }


def main() -> int:
    args = parse_args()
    if args.concurrency < 1:
        raise SystemExit("concurrency must be positive")
    missing = [key for key in MODEL_ENV_KEYS if not os.environ.get(key)]
    if missing:
        raise SystemExit("missing model environment: " + ", ".join(missing))
    env = asset_environment(args.root)
    results = []
    selected = set(args.agent or AGENTS)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as executor:
        futures = {
            executor.submit(
                run_one, args.root, label, agent_import, env, args.run_suffix
            ): label
            for label, agent_import in AGENTS.items()
            if label in selected
        }
        for future in concurrent.futures.as_completed(futures):
            result = future.result()
            results.append(result)
            print(json.dumps(result, sort_keys=True), flush=True)
    output = args.root / "new-agent-smokes-20260815/results.json"
    output.write_text(json.dumps(sorted(results, key=lambda item: str(item["agent"])), indent=2) + "\n")
    return 0 if all(result.get("status") == "scored" for result in results) else 2


if __name__ == "__main__":
    raise SystemExit(main())
