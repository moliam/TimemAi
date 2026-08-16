"""Requirement-by-requirement audit for a completed dual-model campaign."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from itertools import combinations
from pathlib import Path

from benchmarks.terminal_bench.summarize_runs import (
    first_valid_scored,
    record_is_valid,
)


AGENTS = (
    "timem",
    "pi",
    "openhands",
    "goose",
    "aider",
    "sweagent",
    "openharness",
)
TOTAL_TASKS = 89
DATASET_REVISION = "2fd12b88aafdd04a52c298e3940bcb189f9766d6"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def progress_paths(root: Path, model: str) -> dict[str, Path]:
    if model == "gpt-5.6-sol":
        names = {
            "timem": "full-timem-max300-gpt56-20260815",
            "pi": "full-pi-20260815",
            "openhands": "full-openhands-20260815",
            "goose": "full-goose-20260815",
            "aider": "full-aider-20260815",
            "sweagent": "full-sweagent-20260815",
            "openharness": "full-openharness-20260815",
        }
        return {agent: root / name / "progress.jsonl" for agent, name in names.items()}
    return {
        agent: root / "runs/glm52-20260815" / agent / "progress.jsonl"
        for agent in AGENTS
    }


def raw_trial_model(job_dir: Path) -> str | None:
    for result_path in sorted(job_dir.glob("*/result.json")):
        try:
            result = json.loads(result_path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        return str(((result.get("config") or {}).get("agent") or {}).get("model_name") or "") or None
    return None


def audit(args: argparse.Namespace) -> dict[str, object]:
    checks: list[dict[str, object]] = []

    def check(name: str, condition: bool, **details: object) -> None:
        checks.append(
            {
                "name": name,
                "status": "pass" if condition else "fail",
                **details,
            }
        )

    output = args.output_dir
    root = args.dataset.parent
    gpt = json.loads((output / "gpt56-summary.json").read_text())
    glm = json.loads((output / "glm52-summary.json").read_text())
    cross = json.loads((output / "cross-model.json").read_text())
    gpt_manifest = json.loads((output / "evaluation_manifest.json").read_text())
    glm_manifest = json.loads((output / "evaluation_manifest_glm52.json").read_text())
    method = json.loads((output / "method_validation.json").read_text())

    dataset_tasks = {
        path.parent.name for path in args.dataset.glob("*/task.toml")
    }
    revision = subprocess.check_output(
        ["git", "-C", str(args.dataset), "rev-parse", "HEAD"], text=True
    ).strip()
    check(
        "dataset_identity",
        len(dataset_tasks) == TOTAL_TASKS and revision == DATASET_REVISION,
        task_count=len(dataset_tasks),
        revision=revision,
    )

    manifests = (
        ("gpt-5.6-sol", gpt_manifest),
        ("kivy-glm-5_2", glm_manifest),
    )
    for model, manifest in manifests:
        check(
            f"manifest_{model}",
            manifest["model"]["label"] == model
            and manifest["benchmark"]["task_count"] == TOTAL_TASKS
            and manifest["benchmark"]["dataset_revision"] == DATASET_REVISION
            and set(manifest["agents"]) == set(AGENTS)
            and manifest["agents"]["timem"]["benchmark_max_rounds"] == 300,
        )
    check(
        "agent_versions_identical_across_models",
        gpt_manifest["agents"] == glm_manifest["agents"],
    )
    binary_hash = sha256(args.binary)
    check(
        "timem_binary_hash",
        binary_hash
        == gpt_manifest["agents"]["timem"]["benchmark_binary_sha256"]
        == glm_manifest["agents"]["timem"]["benchmark_binary_sha256"],
        observed=binary_hash,
    )

    for model, summary, expected_path in (
        ("gpt-5.6-sol", gpt, "full-timem-max300-gpt56-20260815"),
        ("kivy-glm-5_2", glm, "jobs/glm52-20260815"),
    ):
        evaluation = summary["evaluation"]
        check(
            f"{model}_matrix_identity",
            evaluation["dataset_tasks"] == TOTAL_TASKS
            and evaluation["dataset_revision"] == DATASET_REVISION
            and set(evaluation["agents"]) == set(AGENTS)
            and evaluation["common_tasks_all_agents"] == TOTAL_TASKS,
        )
        check(
            f"{model}_agent_coverage",
            all(summary[agent]["tasks"] == TOTAL_TASKS for agent in AGENTS),
        )
        expected_pairs = {f"{a}_vs_{b}" for a, b in combinations(AGENTS, 2)}
        check(
            f"{model}_within_model_pairs",
            set(summary["pairwise"]) == expected_pairs
            and all(
                row["common_tasks"] == TOTAL_TASKS
                for row in summary["pairwise"].values()
            ),
            pair_count=len(summary["pairwise"]),
        )
        complete_tasks = (
            set(summary["task_results"])
            == set(summary["trial_provenance"])
            == dataset_tasks
        )
        complete_rows = complete_tasks and all(
            set(summary["task_results"][task]) == set(AGENTS)
            and set(summary["trial_provenance"][task]) == set(AGENTS)
            for task in dataset_tasks
        )
        check(f"{model}_per_task_rows", complete_rows)

        jobs_exist = True
        rewards_match = True
        clean_timeouts = True
        timem_path_ok = True
        no_old_timem = True
        timem_runtime_ok = True
        raw_model_ok = True
        first_valid_ok = True
        unique_valid_attempt = True
        ledgers = progress_paths(root, model)
        selected = {
            agent: first_valid_scored(ledgers[agent], agent)
            for agent in AGENTS
        }
        for agent, progress in ledgers.items():
            valid_counts: dict[str, int] = {}
            for line in progress.read_text().splitlines():
                if not line.strip():
                    continue
                record = json.loads(line)
                task = str(record.get("task") or "")
                if record.get("status") == "scored" and record_is_valid(record, agent):
                    valid_counts[task] = valid_counts.get(task, 0) + 1
            unique_valid_attempt &= all(count == 1 for count in valid_counts.values())
        if complete_rows:
            for task in dataset_tasks:
                for agent in AGENTS:
                    provenance = summary["trial_provenance"][task][agent]
                    job_dir = Path(str(provenance["job_dir"]))
                    jobs_exist &= (job_dir / "result.json").is_file()
                    rewards_match &= (
                        float(provenance["reward"])
                        == float(summary["task_results"][task][agent])
                    )
                    first_valid_ok &= (
                        task in selected[agent]
                        and str(selected[agent][task].get("job_dir")) == str(job_dir)
                    )
                    raw_model_ok &= raw_trial_model(job_dir) == model
                    if provenance["timed_out"]:
                        clean_timeouts &= provenance[
                            "timeout_process_cleanup_completed"
                        ] is True
                    if agent == "timem":
                        timem_runtime_ok &= provenance.get(
                            "agent_runtime_overrides"
                        ) == {"max_rounds": 300}
                        rendered = str(job_dir)
                        timem_path_ok &= expected_path in rendered
                        no_old_timem &= "/jobs/full-20260814/" not in rendered
        check(f"{model}_raw_jobs_exist", jobs_exist)
        check(f"{model}_reward_provenance", rewards_match)
        check(f"{model}_first_valid_provenance", first_valid_ok)
        check(f"{model}_unique_valid_attempt_per_task", unique_valid_attempt)
        check(f"{model}_raw_trial_model_identity", raw_model_ok)
        check(f"{model}_timeout_cleanup_proof", clean_timeouts)
        check(f"{model}_timem_runtime_provenance", timem_runtime_ok)
        check(f"{model}_timem_max300_job_namespace", timem_path_ok)
        check(f"{model}_old_timem_campaign_excluded", no_old_timem)

    cross_evaluation = cross["evaluation"]
    check(
        "cross_model_coverage",
        cross_evaluation["dataset_tasks"] == TOTAL_TASKS
        and cross_evaluation["common_tasks_all_agents_both_models"] == TOTAL_TASKS
        and set(cross_evaluation["agents"]) == set(AGENTS)
        and all(
            cross["agents"][agent]["common_tasks"] == TOTAL_TASKS
            for agent in AGENTS
        ),
    )
    check(
        "method_validation",
        bool(method["checks"])
        and all(row.get("status") == "pass" for row in method["checks"]),
        check_count=len(method["checks"]),
    )
    reports = (output / "gpt56-results.md", output / "glm52-results.md", output / "RESULTS.md")
    check(
        "reports_rendered",
        all(path.is_file() and path.stat().st_size > 1000 for path in reports),
    )
    credential_patterns = (
        re.compile(rb"\bcpx_[A-Za-z0-9_-]{20,}\b"),
        re.compile(
            rb"\beyJ[A-Za-z0-9_-]{10,}\."
            rb"[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b"
        ),
    )
    artifact_files = [path for path in output.iterdir() if path.is_file()]
    check(
        "artifact_credentials_absent",
        not any(
            pattern.search(path.read_bytes())
            for path in artifact_files
            for pattern in credential_patterns
        ),
        files_scanned=len(artifact_files),
    )
    return {
        "status": (
            "pass" if all(row["status"] == "pass" for row in checks) else "fail"
        ),
        "checks": checks,
    }


def main() -> int:
    args = parse_args()
    result = audit(args)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    return 0 if result["status"] == "pass" else 2


if __name__ == "__main__":
    raise SystemExit(main())
