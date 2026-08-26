"""Render the final two-model, seven-agent Terminal-Bench report."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from benchmarks.terminal_bench.render_report import (
    DISPLAY_NAMES,
    controlled_table,
    dimension_table,
    leaderboard_table,
    pairwise_table,
    percent,
    require_complete,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--left-summary", type=Path, required=True)
    parser.add_argument("--left-manifest", type=Path, required=True)
    parser.add_argument("--right-summary", type=Path, required=True)
    parser.add_argument("--right-manifest", type=Path, required=True)
    parser.add_argument("--cross-model", type=Path, required=True)
    parser.add_argument("--leaderboard", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def cross_model_table(cross: dict[str, object]) -> list[str]:
    evaluation = cross["evaluation"]
    left = evaluation["left_model"]
    right = evaluation["right_model"]
    total = int(evaluation["dataset_tasks"])
    lines = [
        f"| Agent | {left} passed | {right} passed | Δ {right}−{left} | {left} only | {right} only | Exact McNemar p |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for agent in evaluation["agents"]:
        row = cross["agents"][agent]
        p_value = row["mcnemar_exact_p"]
        lines.append(
            f"| {DISPLAY_NAMES[agent]} | {row[f'{left}_passes']}/{total} | "
            f"{row[f'{right}_passes']}/{total} | "
            f"{row['pass_delta_right_minus_left']:+d} | "
            f"{row[f'{left}_only']} | {row[f'{right}_only']} | "
            f"{'—' if p_value is None else f'{p_value:.4f}'} |"
        )
    return lines


def validate_cross(
    cross: dict[str, object],
    left_summary: dict[str, object],
    right_summary: dict[str, object],
) -> None:
    evaluation = cross["evaluation"]
    total = int(left_summary["evaluation"]["dataset_tasks"])
    if (
        int(evaluation["dataset_tasks"]) != total
        or int(evaluation["common_tasks_all_agents_both_models"]) != total
        or evaluation["dataset_revision"]
        != left_summary["evaluation"]["dataset_revision"]
        or evaluation["dataset_revision"]
        != right_summary["evaluation"]["dataset_revision"]
        or set(evaluation["agents"])
        != set(left_summary["evaluation"]["agents"])
        or set(evaluation["agents"])
        != set(right_summary["evaluation"]["agents"])
    ):
        raise SystemExit("refusing to render an incomplete cross-model comparison")
    incomplete = {
        agent: int(row["common_tasks"])
        for agent, row in cross["agents"].items()
        if int(row["common_tasks"]) != total
    }
    if incomplete:
        raise SystemExit(
            "cross-model agent coverage is incomplete: "
            + json.dumps(incomplete, sort_keys=True)
        )


def main() -> int:
    args = parse_args()
    left_summary = json.loads(args.left_summary.read_text())
    left_manifest = json.loads(args.left_manifest.read_text())
    right_summary = json.loads(args.right_summary.read_text())
    right_manifest = json.loads(args.right_manifest.read_text())
    cross = json.loads(args.cross_model.read_text())
    left_agents = list(left_summary["evaluation"]["agents"])
    right_agents = list(right_summary["evaluation"]["agents"])
    require_complete(left_summary, left_manifest, left_agents)
    require_complete(right_summary, right_manifest, right_agents)
    validate_cross(cross, left_summary, right_summary)
    if left_manifest["agents"] != right_manifest["agents"]:
        raise SystemExit("agent versions differ between model matrices")

    left_model = left_manifest["model"]["label"]
    right_model = right_manifest["model"]["label"]
    benchmark = left_manifest["benchmark"]
    harness = left_manifest["harness"]
    execution = left_manifest["execution"]
    lines = [
        "# Timem and open-source agent evaluation on Terminal-Bench 2.0",
        "",
        "## Cross-model result",
        "",
        *cross_model_table(cross),
        "",
        "Positive Δ means the right-hand model passed more of the same 89 tasks. The exact McNemar test uses only per-task discordant outcomes.",
        "",
        f"## Controlled result: {left_model}",
        "",
        *controlled_table(left_summary, left_agents),
        "",
        f"## Controlled result: {right_model}",
        "",
        *controlled_table(right_summary, right_agents),
        "",
        "*Token telemetry differs by harness and is reported as observed rather than treated as a perfectly normalized efficiency metric.*",
        "",
        f"## Category scores: {left_model}",
        "",
        *dimension_table(left_summary, left_agents, "category"),
        "",
        f"## Category scores: {right_model}",
        "",
        *dimension_table(right_summary, right_agents, "category"),
        "",
        f"## Paired agent comparison: {left_model}",
        "",
        *pairwise_table(left_summary),
        "",
        f"## Paired agent comparison: {right_model}",
        "",
        *pairwise_table(right_summary),
        "",
        "## Controlled protocol",
        "",
        f'- Benchmark: Terminal-Bench {benchmark["version"]}, {benchmark["task_count"]} tasks, dataset commit `{benchmark["dataset_revision"]}`.',
        f'- Runner: Harbor {harness["version"]}, official timeout multiplier `{harness["timeout_multiplier"]}`, no resource overrides, one trial per agent/task.',
        f'- Matrix: the same seven pinned agent versions are each run on all 89 tasks with `{left_model}`, then independently with `{right_model}`.',
        f'- Host: `{execution["architecture"]}`, {execution["container_runtime"]}, at most {execution["max_concurrent_trials"]} trials concurrently.',
        "- Timem unattended setting: round tranche 300. Its product default 50 opens an interactive continue prompt; in `--once-json`, `NoopTurnUi` automatically accepts continue, retains task context, and recharges another 300 rounds, so this is not a hard agent-step cap. Every earlier max-50 trial is excluded, and both reported matrices start Timem from task 1 under this setting. A run that genuinely exhausts the unchanged official wall-clock timeout remains scoreable.",
        "- Validity: completed Harbor result and verifier reward plus evidence of a model response, deterministic provider safety refusal, or a clean official timeout. Setup and transport failures are retried. A genuine official timeout remains scored only when the adapter proves its in-container process tree was terminated before verification; historical timeouts without that proof are rerun regardless of reward. Safety refusals remain scored as combined agent-model capability failures.",
        "- Attempt selection: the first infrastructure-valid scored trial per agent/task is authoritative. Later retries cannot replace or improve it. A completed Harbor job missing only its progress-ledger append is recovered before rescheduling.",
        "- Metric: binary Pass@1. Wilson intervals describe each 89-task proportion; exact McNemar tests compare paired task outcomes.",
    ]
    if args.leaderboard:
        lines.extend(
            [
                "",
                "## Official leaderboard context",
                "",
                *leaderboard_table(args.leaderboard),
                "",
                "This is a selected snapshot of the historical [Terminal-Bench 2.0 leaderboard](https://www.tbench.ai/leaderboard/terminal-bench/2.0). It uses the same benchmark version, but official rows use five trials per task and different model, effort, and agent versions; it is therefore context rather than a controlled head-to-head result. The current benchmark is [Terminal-Bench 2.1](https://www.tbench.ai/leaderboard/terminal-bench/2.1), whose [community submissions are currently closed](https://github.com/harbor-framework/terminal-bench-2-1#submitting-to-the-leaderboard).",
            ]
        )
    lines.extend(
        [
            "",
            "## Reproducibility artifacts",
            "",
            "- `evaluation_manifest.json` and `evaluation_manifest_glm52.json`: pinned models, agents, dataset, runner, resources, and exceptions.",
            "- Per-model `summary.json` files: aggregate metrics plus every task outcome and every within-model pair.",
            "- `cross_model.json`: same-agent, same-task GPT-versus-GLM paired comparisons.",
            "- `method_validation.json`: dataset, timeout isolation, completeness-gate, and credential-persistence checks.",
            "- `completion-audit.json`: final requirement-by-requirement audit; artifact generation fails unless every check passes.",
            "- Harbor job directories: raw configs, trajectories, verifier logs, exceptions, and results.",
            "",
        ]
    )
    args.output.write_text("\n".join(lines))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
