"""Summarize comparable Terminal-Bench Pass@1 runs from progress JSONL files."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import statistics
import subprocess
import tomllib
from collections import defaultdict
from itertools import combinations
from pathlib import Path

from benchmarks.terminal_bench.run_full_server import (
    agent_safety_refused,
    agent_timed_out,
    result_summary,
)
from benchmarks.terminal_bench.process_cleanup import cleanup_completed


AGENT_IMPORTS = {
    "timem": "benchmarks.terminal_bench.timem_agent:TimemShellAgent",
    "pi": "benchmarks.terminal_bench.pi_agent:PiAgent",
    "openhands": "benchmarks.terminal_bench.openhands_agent:OpenHandsAgent",
    "goose": "benchmarks.terminal_bench.goose_agent:GooseAgent",
    "aider": "benchmarks.terminal_bench.aider_agent:AiderAgent",
    "sweagent": "benchmarks.terminal_bench.sweagent_agent:SWEAgent",
    "openharness": "benchmarks.terminal_bench.openharness_agent:OpenHarnessAgent",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", type=Path, required=True)
    parser.add_argument(
        "--run",
        action="append",
        required=True,
        metavar="LABEL=PROGRESS_JSONL",
    )
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--require-complete",
        action="store_true",
        help="Fail unless every run and every pair cover the full dataset.",
    )
    return parser.parse_args()


def wilson(successes: int, total: int, z: float = 1.96) -> tuple[float, float]:
    if total == 0:
        return (0.0, 0.0)
    p = successes / total
    denominator = 1 + z * z / total
    centre = (p + z * z / (2 * total)) / denominator
    margin = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total**2))
    margin /= denominator
    return centre - margin, centre + margin


def task_metadata(dataset: Path) -> dict[str, dict[str, str]]:
    output = {}
    for task_file in dataset.glob("*/task.toml"):
        metadata = tomllib.loads(task_file.read_text()).get("metadata", {})
        output[task_file.parent.name] = {
            "category": str(metadata.get("category", "unknown")),
            "difficulty": str(metadata.get("difficulty", "unknown")),
        }
    return output


def dataset_revision(dataset: Path) -> str | None:
    try:
        return subprocess.check_output(
            ["git", "-C", str(dataset), "rev-parse", "HEAD"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def first_valid_scored(
    progress: Path,
    label: str,
) -> dict[str, dict[str, object]]:
    """Select the first valid attempt so retries cannot improve Pass@1."""
    records: dict[str, dict[str, object]] = {}
    for line in progress.read_text().splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        task = record.get("task")
        if (
            task
            and str(task) not in records
            and record.get("status") == "scored"
            and record_is_valid(record, label)
        ):
            records[str(task)] = record
    return records


def record_is_valid(record: dict[str, object], label: str) -> bool:
    """Reject Harbor trials that received a verifier reward after agent failure."""
    if record.get("reward") is None:
        return False
    raw_job_dir = record.get("job_dir")
    if not raw_job_dir:
        return True
    result_path = Path(str(raw_job_dir)) / "result.json"
    if not result_path.is_file():
        return False
    agent_import = AGENT_IMPORTS.get(label)
    if agent_import is None:
        return False
    try:
        verified = result_summary(
            Path(str(raw_job_dir)),
            agent_import=agent_import,
        )
        return (
            verified["status"] == "scored"
            and verified.get("reward") is not None
            and float(verified["reward"]) == float(record["reward"])
        )
    except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError):
        return False


def exact_mcnemar(left_only: int, right_only: int) -> float | None:
    """Two-sided exact McNemar/binomial p-value for discordant task outcomes."""
    discordant = left_only + right_only
    if discordant == 0:
        return None
    tail = sum(
        math.comb(discordant, k) for k in range(min(left_only, right_only) + 1)
    ) / 2**discordant
    return min(1.0, 2 * tail)


def trial_details(job_dir: Path) -> tuple[float | None, str | None]:
    trials = [
        path for path in job_dir.glob("*/result.json")
        if path.parent != job_dir
    ]
    if not trials:
        return None, None
    result = json.loads(trials[0].read_text())
    timing = result.get("agent_execution") or {}
    duration = None
    if timing.get("started_at") and timing.get("finished_at"):
        from datetime import datetime

        start = datetime.fromisoformat(timing["started_at"].replace("Z", "+00:00"))
        finish = datetime.fromisoformat(timing["finished_at"].replace("Z", "+00:00"))
        duration = (finish - start).total_seconds()
    exception = (result.get("exception_info") or {}).get("exception_type")
    return duration, exception


def summarize(
    label: str,
    records: dict[str, dict[str, object]],
    metadata: dict[str, dict[str, str]],
) -> dict[str, object]:
    valid = {
        task: record
        for task, record in records.items()
        if record_is_valid(record, label)
    }
    passes = sum(float(record["reward"]) > 0 for record in valid.values())
    low, high = wilson(passes, len(valid))
    groups: dict[str, dict[str, list[float]]] = {
        "category": defaultdict(list),
        "difficulty": defaultdict(list),
    }
    durations: list[float] = []
    exceptions: dict[str, int] = defaultdict(int)
    tokens = {"input": 0, "cache": 0, "output": 0}
    for task, record in valid.items():
        reward = float(record["reward"])
        info = metadata.get(task, {})
        for dimension in groups:
            groups[dimension][info.get(dimension, "unknown")].append(reward)
        for output_key, record_key in (
            ("input", "input_tokens"),
            ("cache", "cache_tokens"),
            ("output", "output_tokens"),
        ):
            tokens[output_key] += int(record.get(record_key) or 0)
        if record.get("job_dir"):
            duration, exception = trial_details(Path(str(record["job_dir"])))
            if duration is not None:
                durations.append(duration)
            if exception:
                exceptions[exception] += 1
    # Cache reads are normally a subset of input tokens, so do not double-count
    # them in per-task/per-pass totals.
    token_total = tokens["input"] + tokens["output"]
    return {
        "tasks": len(valid),
        "passes": passes,
        "pass_at_1": passes / len(valid) if valid else None,
        "wilson_95": [low, high],
        "tokens": tokens,
        "tokens_per_task": token_total / len(valid) if valid else None,
        "tokens_per_pass": token_total / passes if passes else None,
        "median_agent_seconds": statistics.median(durations) if durations else None,
        "exceptions": dict(sorted(exceptions.items())),
        "category": {
            key: {"tasks": len(values), "score": sum(values) / len(values)}
            for key, values in sorted(groups["category"].items())
        },
        "difficulty": {
            key: {"tasks": len(values), "score": sum(values) / len(values)}
            for key, values in sorted(groups["difficulty"].items())
        },
    }


def main() -> int:
    args = parse_args()
    metadata = task_metadata(args.dataset)
    output = {}
    records_by_label = {}
    for item in args.run:
        label, separator, path = item.partition("=")
        if not separator:
            raise SystemExit(f"invalid --run value: {item}")
        records = first_valid_scored(Path(path), label)
        records_by_label[label] = {
            task: record for task, record in records.items() if task in metadata
        }
        output[label] = summarize(label, records_by_label[label], metadata)
        output[label]["dataset_tasks"] = len(metadata)
        output[label]["coverage"] = (
            output[label]["tasks"] / len(metadata) if metadata else None
        )
    pairwise = {}
    for left, right in combinations(records_by_label, 2):
        left_records = records_by_label[left]
        right_records = records_by_label[right]
        common = sorted(set(left_records) & set(right_records))
        counts = {
            "both_pass": 0,
            f"{left}_only": 0,
            f"{right}_only": 0,
            "both_fail": 0,
        }
        for task in common:
            left_pass = float(left_records[task]["reward"]) > 0
            right_pass = float(right_records[task]["reward"]) > 0
            if left_pass and right_pass:
                counts["both_pass"] += 1
            elif left_pass:
                counts[f"{left}_only"] += 1
            elif right_pass:
                counts[f"{right}_only"] += 1
            else:
                counts["both_fail"] += 1
        pairwise[f"{left}_vs_{right}"] = {
            "common_tasks": len(common),
            **counts,
            "mcnemar_exact_p": exact_mcnemar(
                counts[f"{left}_only"], counts[f"{right}_only"]
            ),
        }
    common_all = set(metadata)
    for records in records_by_label.values():
        common_all &= set(records)
    output["evaluation"] = {
        "benchmark": "Terminal-Bench 2.0",
        "dataset_revision": dataset_revision(args.dataset),
        "dataset_tasks": len(metadata),
        "agents": list(records_by_label),
        "common_tasks_all_agents": len(common_all),
        "generated_at_utc": dt.datetime.now(dt.UTC).isoformat(),
        "score_semantics": "binary verifier Pass@1",
    }
    output["task_results"] = {
        task: {
            label: float(records[task]["reward"])
            for label, records in records_by_label.items()
            if task in records
        }
        for task in sorted(metadata)
    }
    output["trial_provenance"] = {
        task: {
            label: {
                "job_dir": record.get("job_dir"),
                "reward": float(record["reward"]),
                "agent_runtime_overrides": (
                    {"max_rounds": 300} if label == "timem" else {}
                ),
                "timed_out": (
                    agent_timed_out(Path(str(record["job_dir"])))
                    if record.get("job_dir")
                    else False
                ),
                "model_safety_refusal": (
                    agent_safety_refused(Path(str(record["job_dir"])))
                    if record.get("job_dir")
                    else False
                ),
                "timeout_process_cleanup_completed": (
                    cleanup_completed(Path(str(record["job_dir"])))
                    if record.get("job_dir")
                    else False
                ),
            }
            for label, records in records_by_label.items()
            if task in records
            for record in (records[task],)
        }
        for task in sorted(metadata)
    }
    output["pairwise"] = pairwise
    if args.require_complete:
        incomplete = {
            label: len(records)
            for label, records in records_by_label.items()
            if len(records) != len(metadata)
        }
        incomplete_pairs = {
            label: stats["common_tasks"]
            for label, stats in pairwise.items()
            if stats["common_tasks"] != len(metadata)
        }
        if incomplete or incomplete_pairs:
            raise SystemExit(
                "apple-to-apple completeness gate failed: "
                + json.dumps(
                    {
                        "dataset_tasks": len(metadata),
                        "incomplete_runs": incomplete,
                        "incomplete_pairs": incomplete_pairs,
                    },
                    sort_keys=True,
                )
            )
    rendered = json.dumps(output, indent=2, sort_keys=True)
    if args.output:
        args.output.write_text(rendered + "\n")
    else:
        print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
