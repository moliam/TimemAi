"""Wait for both matrices, then produce completeness-gated result artifacts."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import shutil
import subprocess
import time
from pathlib import Path


AGENTS = (
    "timem",
    "pi",
    "openhands",
    "goose",
    "aider",
    "sweagent",
    "openharness",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--run-tag", required=True)
    parser.add_argument("--campaign-status", type=Path, required=True)
    parser.add_argument("--status", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--poll-seconds", type=int, default=60)
    return parser.parse_args()


def write_status(path: Path, phase: str, **details: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(
            {"phase": phase, "updated_at": time.time(), **details},
            sort_keys=True,
        )
        + "\n"
    )


def campaign_complete(path: Path) -> bool:
    if not path.is_file():
        return False
    try:
        return json.loads(path.read_text()).get("phase") == "second_model_complete"
    except (OSError, json.JSONDecodeError):
        return False


def gpt_progress(root: Path) -> dict[str, Path]:
    return {
        "timem": root / "full-timem-max300-gpt56-20260815/progress.jsonl",
        "pi": root / "full-pi-20260815/progress.jsonl",
        "openhands": root / "full-openhands-20260815/progress.jsonl",
        "goose": root / "full-goose-20260815/progress.jsonl",
        "aider": root / "full-aider-20260815/progress.jsonl",
        "sweagent": root / "full-sweagent-20260815/progress.jsonl",
        "openharness": root / "full-openharness-20260815/progress.jsonl",
    }


def model_progress(root: Path, run_tag: str) -> dict[str, Path]:
    return {
        agent: root / "runs" / run_tag / agent / "progress.jsonl"
        for agent in AGENTS
    }


def run(command: list[str], cwd: Path) -> None:
    completed = subprocess.run(command, cwd=cwd)
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed with return code {completed.returncode}: "
            + " ".join(command[:4])
        )


def summary_command(
    python: Path,
    root: Path,
    progress: dict[str, Path],
    output: Path,
) -> list[str]:
    command = [
        str(python),
        "-m",
        "benchmarks.terminal_bench.summarize_runs",
        "--dataset",
        str(root / "dataset"),
        "--require-complete",
        "--output",
        str(output),
    ]
    for agent in AGENTS:
        command.extend(["--run", f"{agent}={progress[agent]}"])
    return command


def checksums(paths: list[Path]) -> dict[str, str]:
    return {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in paths
    }


def finalize(args: argparse.Namespace) -> list[Path]:
    root = args.root
    adapter = root / "adapter"
    python = root / "tools/python/cpython-3.12.12-linux-x86_64-gnu/bin/python3"
    output = args.output_dir
    output.mkdir(parents=True, exist_ok=True)
    gpt_summary = output / "gpt56-summary.json"
    glm_summary = output / "glm52-summary.json"
    cross = output / "cross-model.json"
    gpt_report = output / "gpt56-results.md"
    glm_report = output / "glm52-results.md"
    final_report = output / "RESULTS.md"
    gpt_manifest = adapter / "benchmarks/terminal_bench/evaluation_manifest.json"
    glm_manifest = adapter / "benchmarks/terminal_bench/evaluation_manifest_glm52.json"
    leaderboard = adapter / "benchmarks/terminal_bench/official_leaderboard_snapshot.json"
    timem_patch = adapter / "benchmarks/terminal_bench/timem-max-rounds-300.patch"
    protocol_readme = adapter / "benchmarks/terminal_bench/README.md"
    method_validation = adapter / "benchmarks/terminal_bench/method_validation.json"

    run(summary_command(python, root, gpt_progress(root), gpt_summary), adapter)
    run(summary_command(python, root, model_progress(root, args.run_tag), glm_summary), adapter)
    run(
        [
            str(python),
            "-m",
            "benchmarks.terminal_bench.compare_model_summaries",
            "--left",
            f"gpt-5.6-sol={gpt_summary}",
            "--right",
            f"kivy-glm-5_2={glm_summary}",
            "--output",
            str(cross),
        ],
        adapter,
    )
    for summary, manifest, report in (
        (gpt_summary, gpt_manifest, gpt_report),
        (glm_summary, glm_manifest, glm_report),
    ):
        command = [
            str(python),
            "-m",
            "benchmarks.terminal_bench.render_report",
            "--summary",
            str(summary),
            "--manifest",
            str(manifest),
            "--output",
            str(report),
        ]
        if leaderboard.is_file():
            command.extend(["--leaderboard", str(leaderboard)])
        run(command, adapter)
    dual_command = [
        str(python),
        "-m",
        "benchmarks.terminal_bench.render_dual_model_report",
        "--left-summary",
        str(gpt_summary),
        "--left-manifest",
        str(gpt_manifest),
        "--right-summary",
        str(glm_summary),
        "--right-manifest",
        str(glm_manifest),
        "--cross-model",
        str(cross),
        "--output",
        str(final_report),
    ]
    if leaderboard.is_file():
        dual_command.extend(["--leaderboard", str(leaderboard)])
    run(dual_command, adapter)
    copied_artifacts = []
    for source, name in (
        (gpt_manifest, "evaluation_manifest.json"),
        (glm_manifest, "evaluation_manifest_glm52.json"),
        (leaderboard, "official_leaderboard_snapshot.json"),
        (timem_patch, "timem-max-rounds-300.patch"),
        (protocol_readme, "BENCHMARK_PROTOCOL.md"),
        (method_validation, "method_validation.json"),
    ):
        destination = output / name
        shutil.copy2(source, destination)
        copied_artifacts.append(destination)
    completion_audit = output / "completion-audit.json"
    run(
        [
            str(python),
            "-m",
            "benchmarks.terminal_bench.audit_final_artifacts",
            "--dataset",
            str(root / "dataset"),
            "--binary",
            str(root / "build/release/timem-native-rs"),
            "--output-dir",
            str(output),
            "--output",
            str(completion_audit),
        ],
        adapter,
    )
    artifacts = [
        gpt_summary,
        glm_summary,
        cross,
        gpt_report,
        glm_report,
        final_report,
        *copied_artifacts,
        completion_audit,
    ]
    (output / "SHA256SUMS.json").write_text(
        json.dumps(checksums(artifacts), indent=2, sort_keys=True) + "\n"
    )
    return artifacts + [output / "SHA256SUMS.json"]


def main() -> int:
    args = parse_args()
    if args.poll_seconds < 10:
        raise SystemExit("poll-seconds must be at least 10")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    with (args.output_dir / "finalizer.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise SystemExit("another campaign finalizer is already active")
        while not campaign_complete(args.campaign_status):
            write_status(args.status, "waiting_for_both_matrices")
            time.sleep(args.poll_seconds)
        while True:
            try:
                write_status(args.status, "generating_artifacts")
                artifacts = finalize(args)
            except Exception as error:
                write_status(
                    args.status,
                    "artifact_generation_retry",
                    error=type(error).__name__,
                )
                time.sleep(max(args.poll_seconds, 300))
                continue
            write_status(
                args.status,
                "complete",
                artifacts=[str(path) for path in artifacts],
            )
            return 0


if __name__ == "__main__":
    raise SystemExit(main())
