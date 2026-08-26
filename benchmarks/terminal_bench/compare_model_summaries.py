"""Build a strict, paired comparison of two complete model matrices."""

from __future__ import annotations

import argparse
import datetime as dt
import json
from pathlib import Path

from benchmarks.terminal_bench.summarize_runs import exact_mcnemar


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--left", required=True, metavar="LABEL=SUMMARY_JSON")
    parser.add_argument("--right", required=True, metavar="LABEL=SUMMARY_JSON")
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def load_spec(spec: str) -> tuple[str, dict[str, object]]:
    label, separator, raw_path = spec.partition("=")
    if not separator or not label or not raw_path:
        raise SystemExit(f"invalid model summary specification: {spec}")
    return label, json.loads(Path(raw_path).read_text())


def outcomes(
    summary: dict[str, object],
    agent: str,
) -> dict[str, bool]:
    return {
        task: float(per_agent[agent]) > 0
        for task, per_agent in summary["task_results"].items()
        if agent in per_agent
    }


def require_complete_pair(
    left: dict[str, object],
    right: dict[str, object],
) -> tuple[list[str], set[str]]:
    left_evaluation = left["evaluation"]
    right_evaluation = right["evaluation"]
    fields = ("benchmark", "dataset_revision", "dataset_tasks")
    mismatches = {
        field: [left_evaluation.get(field), right_evaluation.get(field)]
        for field in fields
        if left_evaluation.get(field) != right_evaluation.get(field)
    }
    left_agents = list(left_evaluation["agents"])
    right_agents = list(right_evaluation["agents"])
    if set(left_agents) != set(right_agents):
        mismatches["agents"] = [sorted(left_agents), sorted(right_agents)]
    if mismatches:
        raise SystemExit(
            "model summaries are not comparable: "
            + json.dumps(mismatches, sort_keys=True)
        )

    total = int(left_evaluation["dataset_tasks"])
    expected_tasks = set(left["task_results"])
    if len(expected_tasks) != total or set(right["task_results"]) != expected_tasks:
        raise SystemExit("model summaries do not contain the same complete task set")
    incomplete: dict[str, dict[str, int]] = {}
    for agent in left_agents:
        left_count = len(outcomes(left, agent))
        right_count = len(outcomes(right, agent))
        if left_count != total or right_count != total:
            incomplete[agent] = {"left": left_count, "right": right_count}
    if incomplete:
        raise SystemExit(
            "cross-model apple-to-apple completeness gate failed: "
            + json.dumps(incomplete, sort_keys=True)
        )
    return left_agents, expected_tasks


def compare(
    left_label: str,
    left: dict[str, object],
    right_label: str,
    right: dict[str, object],
) -> dict[str, object]:
    agents, expected_tasks = require_complete_pair(left, right)
    per_agent: dict[str, dict[str, object]] = {}
    for agent in agents:
        left_outcomes = outcomes(left, agent)
        right_outcomes = outcomes(right, agent)
        left_only = sum(
            left_outcomes[task] and not right_outcomes[task]
            for task in expected_tasks
        )
        right_only = sum(
            right_outcomes[task] and not left_outcomes[task]
            for task in expected_tasks
        )
        both_pass = sum(
            left_outcomes[task] and right_outcomes[task]
            for task in expected_tasks
        )
        both_fail = len(expected_tasks) - both_pass - left_only - right_only
        left_passes = sum(left_outcomes.values())
        right_passes = sum(right_outcomes.values())
        per_agent[agent] = {
            "common_tasks": len(expected_tasks),
            f"{left_label}_passes": left_passes,
            f"{right_label}_passes": right_passes,
            "pass_delta_right_minus_left": right_passes - left_passes,
            "pass_at_1_delta_right_minus_left": (
                right_passes - left_passes
            ) / len(expected_tasks),
            "both_pass": both_pass,
            f"{left_label}_only": left_only,
            f"{right_label}_only": right_only,
            "both_fail": both_fail,
            "mcnemar_exact_p": exact_mcnemar(left_only, right_only),
        }
    return {
        "evaluation": {
            "benchmark": left["evaluation"]["benchmark"],
            "dataset_revision": left["evaluation"]["dataset_revision"],
            "dataset_tasks": len(expected_tasks),
            "agents": agents,
            "left_model": left_label,
            "right_model": right_label,
            "common_tasks_all_agents_both_models": len(expected_tasks),
            "generated_at_utc": dt.datetime.now(dt.UTC).isoformat(),
        },
        "agents": per_agent,
    }


def main() -> int:
    args = parse_args()
    left_label, left = load_spec(args.left)
    right_label, right = load_spec(args.right)
    result = compare(left_label, left, right_label, right)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
