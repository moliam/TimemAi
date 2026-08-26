"""Render a complete, controlled Terminal-Bench comparison as Markdown."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


DISPLAY_NAMES = {
    "timem": "Timem",
    "pi": "Pi",
    "openhands": "OpenHands",
    "goose": "Goose",
    "aider": "Aider",
    "sweagent": "SWE-agent",
    "openharness": "OpenHarness",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--leaderboard", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def percent(value: float | None) -> str:
    return "—" if value is None else f"{100 * value:.1f}%"


def number(value: float | None, digits: int = 0) -> str:
    if value is None:
        return "—"
    return f"{value:,.{digits}f}"


def require_complete(
    summary: dict[str, object],
    manifest: dict[str, object],
    labels: list[str],
) -> None:
    evaluation = summary["evaluation"]
    total = int(evaluation["dataset_tasks"])
    manifest_total = int(manifest["benchmark"]["task_count"])
    manifest_labels = set(manifest["agents"])
    if total != manifest_total:
        raise SystemExit(
            f"summary has {total} tasks but manifest requires {manifest_total}"
        )
    if set(labels) != manifest_labels:
        raise SystemExit(
            "summary/manifest agent mismatch: "
            + json.dumps(
                {
                    "summary": sorted(labels),
                    "manifest": sorted(manifest_labels),
                }
            )
        )
    if evaluation.get("dataset_revision") != manifest["benchmark"].get(
        "dataset_revision"
    ):
        raise SystemExit("summary/manifest dataset revision mismatch")
    failures = {
        label: int(summary[label]["tasks"])
        for label in labels
        if int(summary[label]["tasks"]) != total
    }
    common = int(evaluation["common_tasks_all_agents"])
    bad_pairs = {
        pair: int(stats["common_tasks"])
        for pair, stats in summary["pairwise"].items()
        if int(stats["common_tasks"]) != total
    }
    if failures or common != total or bad_pairs:
        raise SystemExit(
            "refusing to render an incomplete comparison: "
            + json.dumps(
                {
                    "agent_coverage": failures,
                    "common_all": common,
                    "bad_pairs": bad_pairs,
                },
                sort_keys=True,
            )
        )


def controlled_table(summary: dict[str, object], labels: list[str]) -> list[str]:
    ranked = sorted(
        labels,
        key=lambda label: (
            -float(summary[label]["pass_at_1"]),
            DISPLAY_NAMES[label],
        ),
    )
    lines = [
        "| Rank | Agent | Passed | Pass@1 | Wilson 95% CI | Tokens/task* | Median agent time | Exceptions |",
        "|---:|---|---:|---:|---:|---:|---:|---:|",
    ]
    for rank, label in enumerate(ranked, 1):
        result = summary[label]
        low, high = result["wilson_95"]
        exceptions = sum(int(value) for value in result["exceptions"].values())
        lines.append(
            "| "
            + " | ".join(
                [
                    str(rank),
                    DISPLAY_NAMES[label],
                    f'{result["passes"]}/{result["tasks"]}',
                    percent(result["pass_at_1"]),
                    f"{percent(low)}–{percent(high)}",
                    number(result["tokens_per_task"]),
                    (
                        "—"
                        if result["median_agent_seconds"] is None
                        else f'{number(result["median_agent_seconds"], 1)} s'
                    ),
                    str(exceptions),
                ]
            )
            + " |"
        )
    return lines


def dimension_table(
    summary: dict[str, object], labels: list[str], dimension: str
) -> list[str]:
    keys = sorted(
        {
            key
            for label in labels
            for key in summary[label][dimension]
        }
    )
    lines = [
        "| " + dimension.title() + " | " + " | ".join(DISPLAY_NAMES[x] for x in labels) + " |",
        "|---|" + "---:|" * len(labels),
    ]
    for key in keys:
        cells = []
        for label in labels:
            value = summary[label][dimension].get(key)
            cells.append("—" if value is None else percent(value["score"]))
        lines.append(f"| {key} | " + " | ".join(cells) + " |")
    return lines


def pairwise_table(summary: dict[str, object]) -> list[str]:
    lines = [
        "| Pair | Left only | Right only | Exact McNemar p |",
        "|---|---:|---:|---:|",
    ]
    for pair, stats in sorted(summary["pairwise"].items()):
        left, right = pair.split("_vs_", 1)
        p_value = stats["mcnemar_exact_p"]
        lines.append(
            f"| {DISPLAY_NAMES[left]} vs {DISPLAY_NAMES[right]} | "
            f'{stats[f"{left}_only"]} | {stats[f"{right}_only"]} | '
            f'{"—" if p_value is None else f"{p_value:.4f}"} |'
        )
    return lines


def leaderboard_table(path: Path) -> list[str]:
    snapshot = json.loads(path.read_text())
    lines = [
        "| Agent | Model | Official score | Date |",
        "|---|---|---:|---|",
    ]
    for row in snapshot["entries"]:
        score = percent(float(row["score"]))
        if row.get("ci"):
            score += f' ± {100 * float(row["ci"]):.1f} pp'
        lines.append(
            f'| [{row["agent"]}]({row["url"]}) | {row["model"]} | '
            f'{score} | {row["date"]} |'
        )
    return lines


def main() -> int:
    args = parse_args()
    summary = json.loads(args.summary.read_text())
    manifest = json.loads(args.manifest.read_text())
    labels = list(summary["evaluation"]["agents"])
    require_complete(summary, manifest, labels)
    benchmark = manifest["benchmark"]
    harness = manifest["harness"]
    model = manifest["model"]
    execution = manifest["execution"]

    lines = [
        "# Timem Agent Terminal-Bench 2.0 evaluation",
        "",
        "## Result",
        "",
        *controlled_table(summary, labels),
        "",
        "*Tokens/task counts reported input plus output tokens when the harness exposes both; cache-read tokens are not double-counted. Token telemetry differs by harness and should not be treated as a perfectly normalized efficiency metric.*",
        "",
        "## Controlled protocol",
        "",
        f'- Benchmark: Terminal-Bench {benchmark["version"]}, {benchmark["task_count"]} tasks, dataset commit `{benchmark["dataset_revision"]}`.',
        f'- Official runner: Harbor {harness["version"]}; timeout multiplier `{harness["timeout_multiplier"]}` and no task resource overrides.',
        f'- Model: `{model["label"]}` through the same private {model["protocol"]} endpoint for all agents.',
        f'- Host: `{execution["architecture"]}` with {execution["container_runtime"]}; at most {execution["max_concurrent_trials"]} trials in parallel.',
        "- Build transport: task definitions, resources, agent timeouts, and verifier timeouts are unchanged. Base-image registry, APT mirror, and Git HTTP transport substitutions are recorded in the manifest and applied identically before all seven agents share each task image.",
        "- Metric: one independent trial per agent/task (Pass@1). All 89 verifiers are binary 0/1.",
        "- Timem unattended setting: round tranche 300. Its product default 50 opens an interactive continue prompt; in `--once-json`, `NoopTurnUi` automatically accepts continue, retains task context, and recharges another 300 rounds, so this is not a hard agent-step cap. Every earlier max-50 trial is excluded and the reported Timem matrix starts from task 1 under this setting. A run that genuinely exhausts the unchanged official wall-clock timeout remains scoreable.",
        "- Validity: a trial must have a completed Harbor result and verifier reward plus evidence of a model response, deterministic provider safety refusal, or a clean official timeout. A timeout is scoreable only when the adapter proves its in-container process tree was terminated before verification; historical timeouts without this marker and setup/transport failures are retried. Safety refusals are combined agent-model capability failures and remain scored rather than retried.",
        "- Attempt selection: the first infrastructure-valid scored trial per agent/task is authoritative. Later retries cannot replace or improve it. A completed Harbor job missing only its progress-ledger append is recovered before rescheduling.",
        "",
        "## Score by category",
        "",
        *dimension_table(summary, labels, "category"),
        "",
        "## Score by difficulty",
        "",
        *dimension_table(summary, labels, "difficulty"),
        "",
        "## Paired task comparison",
        "",
        "All pairs below use the same 89 tasks. The exact McNemar test uses only discordant pass/fail outcomes.",
        "",
        *pairwise_table(summary),
    ]
    if args.leaderboard:
        lines.extend(
            [
                "",
                "## Official leaderboard context",
                "",
                *leaderboard_table(args.leaderboard),
                "",
                "This is a selected snapshot of the historical [Terminal-Bench 2.0 leaderboard](https://www.tbench.ai/leaderboard/terminal-bench/2.0), so it uses the same benchmark version but is still context rather than a controlled head-to-head comparison: official rows use five trials per task and different model, effort, and agent versions, while the matrix above fixes one model and uses one trial per task. The current official benchmark is [Terminal-Bench 2.1](https://www.tbench.ai/leaderboard/terminal-bench/2.1).",
            ]
        )
    lines.extend(
        [
            "",
            "## Reproducibility artifacts",
            "",
            "- `evaluation_manifest.json`: pinned versions and protocol.",
            "- `summary.json`: aggregate metrics, all 89 per-task outcomes, and all pairwise comparisons.",
            "- `method_validation.json`: dataset, timeout isolation, completeness-gate, and credential-persistence checks.",
            "- Harbor job directories: raw configs, trajectories, verifier logs, exceptions, and `result.json` files.",
            "",
        ]
    )
    args.output.write_text("\n".join(lines))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
