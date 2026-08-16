"""Run a resumable, image-sharing Terminal-Bench agent comparison."""

from __future__ import annotations

import argparse
import concurrent.futures
import fcntl
import json
import os
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

from benchmarks.terminal_bench.run_full_server import (
    MODEL_ENV_KEYS,
    append_progress,
    image_exists,
    mirrored_dockerfile,
    result_summary,
    run_logged,
    run_task,
    task_image,
)


@dataclass(frozen=True)
class RunConfig:
    label: str
    agent_import: str
    model_label: str
    jobs_dir: Path
    work_dir: Path
    run_prefix: str
    seed_job: Path | None = None
    require_model_call: bool = False


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--concurrency", type=int, default=3)
    parser.add_argument("--image-batch-size", type=int, default=4)
    parser.add_argument("--builder", default="timem-bench-builder")
    parser.add_argument("--max-passes", type=int, default=10)
    parser.add_argument("--model-label", default="gpt-5.6-sol")
    parser.add_argument(
        "--run-tag",
        help=(
            "Isolated output namespace for another model campaign. Omit to "
            "resume the legacy GPT-5.6 paths."
        ),
    )
    return parser.parse_args()


def configs(
    root: Path,
    *,
    model_label: str = "gpt-5.6-sol",
    run_tag: str | None = None,
) -> list[RunConfig]:
    if run_tag is not None:
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", run_tag):
            raise SystemExit("run-tag must contain only letters, digits, dot, dash, underscore")

        def isolated(label: str, agent_import: str) -> RunConfig:
            return RunConfig(
                label,
                agent_import,
                model_label,
                root / "jobs" / run_tag / label,
                root / "runs" / run_tag / label,
                f"{label}-{run_tag}",
                require_model_call=label == "timem",
            )

        return [
            isolated("timem", "benchmarks.terminal_bench.timem_agent:TimemShellAgent"),
            isolated("pi", "benchmarks.terminal_bench.pi_agent:PiAgent"),
            isolated("openhands", "benchmarks.terminal_bench.openhands_agent:OpenHandsAgent"),
            isolated("goose", "benchmarks.terminal_bench.goose_agent:GooseAgent"),
            isolated("aider", "benchmarks.terminal_bench.aider_agent:AiderAgent"),
            isolated("sweagent", "benchmarks.terminal_bench.sweagent_agent:SWEAgent"),
            isolated("openharness", "benchmarks.terminal_bench.openharness_agent:OpenHarnessAgent"),
        ]

    # Preserve the original paths so the in-flight GPT-5.6 campaign can resume
    # without copying or re-running any completed trial.
    return [
        RunConfig(
            "timem",
            "benchmarks.terminal_bench.timem_agent:TimemShellAgent",
            model_label,
            root / "jobs/full-timem-max300-gpt56-20260815",
            root / "full-timem-max300-gpt56-20260815",
            "timem-max300-gpt56",
            require_model_call=True,
        ),
        RunConfig(
            "pi",
            "benchmarks.terminal_bench.pi_agent:PiAgent",
            model_label,
            root / "jobs/full-pi-20260815",
            root / "full-pi-20260815",
            "pi-full",
            root / "jobs/comparison-smoke/smoke-pi",
        ),
        RunConfig(
            "openhands",
            "benchmarks.terminal_bench.openhands_agent:OpenHandsAgent",
            model_label,
            root / "jobs/full-openhands-20260815",
            root / "full-openhands-20260815",
            "openhands-full",
        ),
        RunConfig(
            "goose",
            "benchmarks.terminal_bench.goose_agent:GooseAgent",
            model_label,
            root / "jobs/full-goose-20260815",
            root / "full-goose-20260815",
            "goose-full",
        ),
        RunConfig(
            "aider",
            "benchmarks.terminal_bench.aider_agent:AiderAgent",
            model_label,
            root / "jobs/full-aider-20260815",
            root / "full-aider-20260815",
            "aider-full",
        ),
        RunConfig(
            "sweagent",
            "benchmarks.terminal_bench.sweagent_agent:SWEAgent",
            model_label,
            root / "jobs/full-sweagent-20260815",
            root / "full-sweagent-20260815",
            "sweagent-full",
        ),
        RunConfig(
            "openharness",
            "benchmarks.terminal_bench.openharness_agent:OpenHarnessAgent",
            model_label,
            root / "jobs/full-openharness-20260815",
            root / "full-openharness-20260815",
            "openharness-full",
        ),
    ]


def _job_task_name(config: RunConfig, job_dir: Path) -> str | None:
    prefix = f"{config.run_prefix}-"
    if not job_dir.name.startswith(prefix):
        return None
    task = job_dir.name[len(prefix) :]
    return re.sub(r"-retry-[0-9]+$", "", task) or None


def _job_started_at(job_dir: Path) -> str:
    for result_path in sorted(job_dir.glob("*/result.json")):
        try:
            return str(json.loads(result_path.read_text()).get("started_at") or "")
        except (OSError, json.JSONDecodeError):
            continue
    retry = re.search(r"-retry-([0-9]+)$", job_dir.name)
    return f"9999-{retry.group(1)}" if retry else ""


def load_completed(
    config: RunConfig,
    *,
    recover_jobs: bool = False,
) -> dict[str, dict[str, object]]:
    """Return the first valid scored trial per task.

    Infrastructure-invalid trials never consume Pass@1. When a runner restarts,
    ``recover_jobs`` reconstructs a missing ledger entry from Harbor's immutable
    job directory, choosing the earliest valid attempt and never a later retry.
    """
    progress = config.work_dir / "progress.jsonl"
    completed: dict[str, dict[str, object]] = {}
    if progress.is_file():
        for line in progress.read_text().splitlines():
            if not line.strip():
                continue
            record = json.loads(line)
            if record.get("status") != "scored" or record.get("reward") is None:
                continue
            if record.get("job_dir"):
                verified = result_summary(
                    Path(str(record["job_dir"])),
                    require_model_call=config.require_model_call,
                    agent_import=config.agent_import,
                )
                if verified["status"] != "scored":
                    continue
            completed.setdefault(str(record["task"]), record)
    if recover_jobs and config.jobs_dir.is_dir():
        candidates: list[tuple[str, Path]] = []
        for job_dir in config.jobs_dir.iterdir():
            if not job_dir.is_dir() or not (job_dir / "result.json").is_file():
                continue
            task = _job_task_name(config, job_dir)
            if task is not None and task not in completed:
                candidates.append((task, job_dir))
        candidates.sort(key=lambda item: (_job_started_at(item[1]), item[1].name))
        for task, job_dir in candidates:
            if task in completed:
                continue
            summary = result_summary(
                job_dir,
                require_model_call=config.require_model_call,
                agent_import=config.agent_import,
            )
            if summary["status"] != "scored" or summary.get("reward") is None:
                continue
            recovered = {
                "task": task,
                **summary,
                "reused": True,
                "recovered_from_harbor_job": True,
            }
            append_progress(progress, recovered)
            completed[task] = recovered
    if config.seed_job is not None and "regex-log" not in completed:
        seed = {
            "task": "regex-log",
            **result_summary(
                config.seed_job,
                require_model_call=config.require_model_call,
                agent_import=config.agent_import,
            ),
            "reused": True,
        }
        append_progress(progress, seed)
        if seed["status"] == "scored" and seed.get("reward") is not None:
            completed["regex-log"] = seed
    return completed


def task_args(
    root: Path,
    config: RunConfig,
    shared: argparse.Namespace,
) -> argparse.Namespace:
    return argparse.Namespace(
        jobs_dir=config.jobs_dir,
        work_dir=config.work_dir,
        run_prefix=config.run_prefix,
        harbor=root / "bin/harbor",
        agent_import=config.agent_import,
        model_label=config.model_label,
    )


def main() -> int:
    args = parse_args()
    if args.concurrency < 1:
        raise SystemExit("concurrency must be positive")
    if args.image_batch_size < 1:
        raise SystemExit("image-batch-size must be positive")
    root = args.root
    dataset = root / "dataset"
    adapter_root = root / "adapter"
    binary = root / "build/release/timem-native-rs"
    harbor = root / "bin/harbor"
    missing = [key for key in MODEL_ENV_KEYS if not os.environ.get(key)]
    if missing:
        raise SystemExit("missing model environment: " + ", ".join(missing))
    if not binary.is_file() or not harbor.is_file():
        raise SystemExit("Timem binary or Harbor executable is missing")

    run_configs = configs(
        root,
        model_label=args.model_label,
        run_tag=args.run_tag,
    )
    for config in run_configs:
        config.jobs_dir.mkdir(parents=True, exist_ok=True)
        config.work_dir.mkdir(parents=True, exist_ok=True)
    matrix_work = root / (
        f"matrix-{args.run_tag}"
        if args.run_tag
        else "full-matrix-shared-20260815"
    )
    matrix_work.mkdir(parents=True, exist_ok=True)
    lock_handle = (matrix_work / "runner.lock").open("w")
    try:
        fcntl.flock(lock_handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit("another comparison matrix runner is already active")
    lock_handle.write(f"{os.getpid()}\n")
    lock_handle.flush()
    env = os.environ.copy()
    env["PYTHONPATH"] = str(adapter_root)
    env["TIMEM_BENCH_BINARY"] = str(binary)
    env.update(
        {
            "PI_BENCH_ARCHIVE": str(root / "pi-agent-0.84.2.tar.gz"),
            "PI_BENCH_NODE": str(root / "node/node-v22.22.0-linux-x64/bin/node"),
            "PYTHON_BENCH_ARCHIVE": str(root / "tools/archives/python-3.12.12.tar.gz"),
            "AIDER_BENCH_ARCHIVE": str(root / "tools/archives/aider-0.86.0-site.tar.gz"),
            "OPENHANDS_BENCH_ARCHIVE": str(root / "tools/archives/openhands-1.13.0-site.tar.gz"),
            "OPENHARNESS_BENCH_ARCHIVE": str(root / "tools/archives/openharness-0.1.9-site.tar.gz"),
            "SWEAGENT_BENCH_ARCHIVE": str(root / "tools/archives/swe-agent-1.1.0-site.tar.gz"),
            "SWEAGENT_SOURCE_ARCHIVE": str(root / "tools/archives/swe-agent-1.1.0-source.tar.gz"),
            "GOOSE_BENCH_ARCHIVE": str(root / "tools/goose-1.46.0.tar.gz"),
        }
    )
    task_dirs = sorted(
        path
        for path in dataset.iterdir()
        if path.is_dir() and (path / "task.toml").is_file()
    )
    targets = {config.label: {path.name for path in task_dirs} for config in run_configs}

    for pass_number in range(1, args.max_passes + 1):
        completed = {
            config.label: load_completed(config, recover_jobs=True)
            for config in run_configs
        }
        counts = {
            label: len(set(records) & targets[label])
            for label, records in completed.items()
        }
        print(
            json.dumps(
                {
                    "pass": pass_number,
                    "model": args.model_label,
                    "run_tag": args.run_tag or "legacy-gpt56",
                    "counts": counts,
                }
            ),
            flush=True,
        )
        if all(
            counts[config.label] == len(targets[config.label])
            for config in run_configs
        ):
            return 0
        pending = [
            task_dir
            for task_dir in task_dirs
            if any(
                task_dir.name in targets[config.label]
                and task_dir.name not in completed[config.label]
                for config in run_configs
            )
        ]

        for offset in range(0, len(pending), args.image_batch_size):
            batch = pending[offset : offset + args.image_batch_size]
            runnable: list[tuple[Path, str]] = []
            for task_dir in batch:
                image = task_image(task_dir)
                if not image_exists(image):
                    dockerfile = mirrored_dockerfile(task_dir, matrix_work)
                    return_code = run_logged(
                        [
                            "docker", "buildx", "build",
                            "--builder", args.builder,
                            "--allow", "network.host",
                            "--network", "host",
                            "--load",
                            "--tag", image,
                            "--file", str(dockerfile),
                            str(task_dir / "environment"),
                        ],
                        matrix_work / "build-logs" / f"{task_dir.name}.log",
                        env,
                    )
                    if return_code != 0:
                        record = {
                            "task": task_dir.name,
                            "status": "infra_error",
                            "stage": "build",
                            "return_code": return_code,
                        }
                        for config in run_configs:
                            if (
                                task_dir.name in targets[config.label]
                                and task_dir.name not in completed[config.label]
                            ):
                                append_progress(config.work_dir / "progress.jsonl", record)
                        continue
                runnable.append((task_dir, image))

            # Share one executor across every agent/task combination in the batch.
            # A slow trial must not leave the remaining slots idle while the runner
            # waits before moving to the next agent.
            agent_jobs = [
                (
                    config,
                    task_dir,
                    image,
                    task_args(root, config, args),
                )
                for config in run_configs
                for task_dir, image in runnable
                if (
                    task_dir.name in targets[config.label]
                    and task_dir.name not in completed[config.label]
                )
            ]
            with concurrent.futures.ThreadPoolExecutor(
                max_workers=args.concurrency
            ) as executor:
                futures = {
                    executor.submit(
                        run_task,
                        task_dir,
                        image,
                        per_agent_args,
                        env,
                    ): (config, task_dir)
                    for config, task_dir, image, per_agent_args in agent_jobs
                }
                for future in concurrent.futures.as_completed(futures):
                    config, task_dir = futures[future]
                    try:
                        record = future.result()
                    except Exception as error:
                        record = {
                            "task": task_dir.name,
                            "status": "infra_error",
                            "stage": "runner",
                            "error": type(error).__name__,
                        }
                    append_progress(config.work_dir / "progress.jsonl", record)
                    print(
                        json.dumps(
                            {"agent": config.label, **record},
                            sort_keys=True,
                        ),
                        flush=True,
                    )

            for _, image in runnable:
                subprocess.run(
                    ["docker", "image", "rm", image],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            if shutil.disk_usage(matrix_work).free < 20 * 1024**3:
                raise SystemExit("disk guard stopped matrix run")

    counts = {
        config.label: len(
            set(load_completed(config, recover_jobs=True)) & targets[config.label]
        )
        for config in run_configs
    }
    print(json.dumps({"status": "incomplete", "counts": counts}), flush=True)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
