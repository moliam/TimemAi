"""Run the complete Terminal-Bench dataset with Timem on a Docker host.

The script builds official task Dockerfiles in small batches because some Docker
registries do not reliably serve the published prebuilt task images. Model
credentials are inherited from the process environment and are never written to
the progress file or child command lines.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import re
import shutil
import subprocess
import time
from pathlib import Path
from urllib.parse import urlsplit

from benchmarks.terminal_bench.process_cleanup import cleanup_completed


MODEL_ENV_KEYS = (
    "TIMEM_API_KEY",
    "TIMEM_API_PROTOCOL",
    "TIMEM_BASE_URL",
    "TIMEM_MODEL",
    "TIMEM_RESPONSE_PROTOCOL",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", type=Path, required=True)
    parser.add_argument("--jobs-dir", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--adapter-root", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--harbor", type=Path, required=True)
    parser.add_argument("--concurrency", type=int, default=3)
    parser.add_argument("--seed-job", type=Path)
    parser.add_argument("--seed-task", default="regex-log")
    parser.add_argument(
        "--agent-import",
        default="benchmarks.terminal_bench.timem_agent:TimemShellAgent",
    )
    parser.add_argument("--model-label", default="gpt-5.6-sol")
    parser.add_argument("--run-prefix", default="timem-full")
    parser.add_argument("--builder", default="timem-bench-builder")
    return parser.parse_args()


def run_logged(argv: list[str], log_path: Path, env: dict[str, str]) -> int:
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("w") as log:
        completed = subprocess.run(
            argv,
            stdout=log,
            stderr=subprocess.STDOUT,
            text=True,
            env=env,
        )
    return completed.returncode


def task_image(task_dir: Path) -> str:
    text = (task_dir / "task.toml").read_text()
    match = re.search(r'^docker_image\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        raise ValueError(f"docker_image missing from {task_dir / 'task.toml'}")
    return match.group(1)


def image_exists(image: str) -> bool:
    return subprocess.run(
        ["docker", "image", "inspect", image],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ).returncode == 0


def mirrored_dockerfile(task_dir: Path, work_dir: Path) -> Path:
    """Generate a Dockerfile that only swaps Debian/Ubuntu package mirrors."""
    source = (task_dir / "environment" / "Dockerfile").read_text()
    # GitHub occasionally resets HTTP/2 clone streams on this host. Pinning git's
    # transport to HTTP/1.1 changes no source content and makes image builds
    # reproducible (notably crack-7z-hash).
    source = re.sub(r"\bgit\s+clone\b", "git -c http.version=HTTP/1.1 clone", source)
    injection = (
        "RUN set -eux; for f in /etc/apt/sources.list "
        "/etc/apt/sources.list.d/debian.sources "
        "/etc/apt/sources.list.d/ubuntu.sources; do "
        "[ ! -f \"$f\" ] || sed -i "
        "-e 's|http://archive.ubuntu.com/ubuntu|http://mirrors.ustc.edu.cn/ubuntu|g' "
        "-e 's|http://security.ubuntu.com/ubuntu|http://mirrors.ustc.edu.cn/ubuntu|g' "
        "-e 's|http://deb.debian.org/debian|http://mirrors.ustc.edu.cn/debian|g' "
        "-e 's|http://security.debian.org/debian-security|http://mirrors.ustc.edu.cn/debian-security|g' "
        "\"$f\"; done\n"
    )
    lines: list[str] = []
    for line in source.splitlines(keepends=True):
        line = re.sub(
            r"^(\s*FROM\s+)(python|ubuntu|debian):",
            r"\1public.ecr.aws/docker/library/\2:",
            line,
            flags=re.IGNORECASE,
        )
        lines.append(line)
        if line.lstrip().upper().startswith("FROM "):
            lines.append(injection)
    output = work_dir / "generated-dockerfiles" / f"{task_dir.name}.Dockerfile"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("".join(lines))
    return output


def timem_never_called_model(job_dir: Path) -> bool:
    """Identify a completed Timem transport/setup failure, not an agent timeout."""
    for output in job_dir.glob("*/agent/timem.json"):
        try:
            payload = json.loads(output.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        stats = payload.get("stats") or {}
        return int(stats.get("llm_calls") or 0) == 0
    return False


def agent_called_model(job_dir: Path, agent_import: str) -> bool:
    """Return true only when the installed agent left model-response evidence."""
    marker_files: tuple[str, tuple[str, ...]]
    if "timem_agent" in agent_import:
        for output in job_dir.glob("*/agent/timem.json"):
            try:
                stats = (json.loads(output.read_text()).get("stats") or {})
                return int(stats.get("llm_calls") or 0) > 0
            except (OSError, json.JSONDecodeError, TypeError, ValueError):
                continue
        return False
    if "pi_agent" in agent_import:
        marker_files = ("*/agent/pi.jsonl", ('"role":"assistant"',))
    elif "openhands_agent" in agent_import:
        marker_files = ("*/agent/openhands.jsonl", ('"tool_call_id"',))
    elif "goose_agent" in agent_import:
        marker_files = (
            "*/agent/goose.jsonl",
            ('"inference":{"provider":"openai"',),
        )
    elif "aider_agent" in agent_import:
        marker_files = ("*/agent/aider.log", ("Tokens:",))
    elif "sweagent_agent" in agent_import:
        # A Harbor timeout can interrupt SWE-agent before it finalizes the
        # trajectory file. Its streamed log records each LiteLLM response as it
        # happens and therefore remains authoritative model-call evidence.
        for output in job_dir.glob("*/agent/sweagent.log"):
            try:
                if "ModelResponse" in output.read_text(errors="replace"):
                    return True
            except OSError:
                continue
        marker_files = ("*/agent/swe-output/**/*.traj", ('"role": "assistant"',))
    elif "openharness_agent" in agent_import:
        marker_files = (
            "*/agent/openharness.jsonl",
            (
                '"type": "assistant_delta"',
                '"type":"assistant_delta"',
                '"type": "assistant_complete"',
                '"type":"assistant_complete"',
            ),
        )
    else:
        return False
    pattern, markers = marker_files
    for output in job_dir.glob(pattern):
        try:
            text = output.read_text(errors="replace")
        except OSError:
            continue
        if any(marker in text for marker in markers):
            return True
    return False


def agent_timed_out(job_dir: Path) -> bool:
    """Return true when Harbor ended a launched trial at its official timeout."""
    for result_path in job_dir.glob("*/result.json"):
        try:
            result = json.loads(result_path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        exception = result.get("exception_info") or {}
        if exception.get("exception_type") == "AgentTimeoutError":
            return True
    return False


def agent_safety_refused(job_dir: Path) -> bool:
    """Return true for a deterministic provider safety refusal after a request."""
    for result_path in job_dir.glob("*/result.json"):
        try:
            result = json.loads(result_path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        exception = result.get("exception_info") or {}
        if exception.get("exception_type") == "AgentSafetyRefusalError":
            return True
    markers = (
        "flagged for possible cybersecurity risk",
        "agentsafetyrefusalerror",
        "agent safety refusal",
    )
    for pattern in (
        "*/agent/timem.json",
        "*/agent/pi.jsonl",
        "*/agent/openhands.jsonl",
        "*/agent/goose.jsonl",
        "*/agent/aider.log",
        "*/agent/sweagent.log",
        "*/agent/openharness.jsonl",
    ):
        for output in job_dir.glob(pattern):
            try:
                text = output.read_text(errors="replace").lower()
            except OSError:
                continue
            if any(marker in text for marker in markers):
                return True
    return False


def result_summary(
    job_dir: Path,
    *,
    require_model_call: bool = False,
    agent_import: str | None = None,
) -> dict[str, object]:
    result = json.loads((job_dir / "result.json").read_text())
    stats = result.get("stats") or {}
    evals = stats.get("evals") or {}
    eval_data = next(iter(evals.values()), {})
    metrics = eval_data.get("metrics") or []
    mean = metrics[0].get("mean") if metrics else None
    input_tokens = stats.get("n_input_tokens", 0)
    cache_tokens = stats.get("n_cache_tokens", 0)
    output_tokens = stats.get("n_output_tokens", 0)
    completed = stats.get("n_completed_trials") == 1
    errored = int(stats.get("n_errored_trials") or 0) != 0
    # A process that consumes the official agent timeout is a legitimate
    # Pass@1 attempt even when its CLI only flushes model telemetry on graceful
    # exit, but only if the adapter proves that the cancelled process tree was
    # terminated before verification. Older Docker-exec timeouts could leave an
    # agent alive concurrently with the verifier and are invalid infrastructure
    # trials rather than extra attempts. Fast setup/transport exits still
    # require positive model-response evidence.
    timed_out = agent_timed_out(job_dir)
    safety_refused = agent_safety_refused(job_dir)
    timeout_cleanup_ok = cleanup_completed(job_dir)
    unclean_timeout = timed_out and not timeout_cleanup_ok
    no_model_call = not timed_out and not safety_refused and (
        not agent_called_model(job_dir, agent_import)
        if agent_import
        else require_model_call and timem_never_called_model(job_dir)
    )
    status = (
        "scored"
        if completed and mean is not None and not no_model_call and not unclean_timeout
        else "infra_error"
    )
    summary = {
        "status": status,
        "reward": mean,
        "input_tokens": input_tokens,
        "cache_tokens": cache_tokens,
        "output_tokens": output_tokens,
        "job_dir": str(job_dir),
    }
    if completed and no_model_call:
        summary["stage"] = "agent_no_model_call"
    if completed and unclean_timeout:
        summary["stage"] = "agent_timeout_process_cleanup_missing"
    if timed_out:
        summary["timeout_process_cleanup_completed"] = timeout_cleanup_ok
    if safety_refused:
        summary["model_safety_refusal"] = True
    if completed and errored and not no_model_call:
        summary["agent_exception"] = True
    return summary


def run_task(
    task_dir: Path,
    image: str,
    args: argparse.Namespace,
    env: dict[str, str],
) -> dict[str, object]:
    name = task_dir.name
    require_model_call = "timem_agent" in args.agent_import
    job_name = f"{args.run_prefix}-{name}"
    job_dir = args.jobs_dir / job_name
    if (job_dir / "result.json").is_file():
        summary = result_summary(
            job_dir,
            require_model_call=require_model_call,
            agent_import=args.agent_import,
        )
        if summary["status"] == "scored":
            return {"task": name, **summary, "reused": True}
        job_name = f"{job_name}-retry-{int(time.time())}"
        job_dir = args.jobs_dir / job_name

    model_host = urlsplit(env["TIMEM_BASE_URL"]).hostname
    if not model_host:
        return {
            "task": name,
            "status": "infra_error",
            "stage": "invalid_model_base_url",
            "job_dir": str(job_dir),
        }
    argv = [
        str(args.harbor),
        "run",
        "--path", str(task_dir),
        "--agent", args.agent_import,
        "--model", args.model_label,
        "--allow-agent-host", model_host,
        "--n-concurrent", "1",
        "--yes",
        "--jobs-dir", str(args.jobs_dir),
        "--job-name", job_name,
    ]
    return_code = run_logged(
        argv,
        args.work_dir / "run-logs" / f"{name}.log",
        env,
    )
    if not (job_dir / "result.json").is_file():
        return {
            "task": name,
            "status": "infra_error",
            "stage": "harbor",
            "return_code": return_code,
            "job_dir": str(job_dir),
        }
    return {
        "task": name,
        **result_summary(
            job_dir,
            require_model_call=require_model_call,
            agent_import=args.agent_import,
        ),
        "return_code": return_code,
    }


def append_progress(path: Path, record: dict[str, object]) -> None:
    with path.open("a") as output:
        output.write(json.dumps(record, sort_keys=True) + "\n")
        output.flush()


def main() -> int:
    args = parse_args()
    missing = [key for key in MODEL_ENV_KEYS if not os.environ.get(key)]
    if missing:
        raise SystemExit("missing model environment: " + ", ".join(missing))
    if not args.binary.is_file() or not args.harbor.is_file():
        raise SystemExit("Timem binary or Harbor executable is missing")

    args.work_dir.mkdir(parents=True, exist_ok=True)
    args.jobs_dir.mkdir(parents=True, exist_ok=True)
    progress_path = args.work_dir / "progress.jsonl"
    require_model_call = "timem_agent" in args.agent_import
    completed = {}
    if progress_path.is_file():
        for line in progress_path.read_text().splitlines():
            if line.strip():
                record = json.loads(line)
                if record.get("status") != "scored" or record.get("reward") is None:
                    continue
                if record.get("job_dir"):
                    verified = result_summary(
                        Path(str(record["job_dir"])),
                        require_model_call=require_model_call,
                        agent_import=args.agent_import,
                    )
                    if verified["status"] != "scored":
                        continue
                completed.setdefault(str(record["task"]), record)

    if args.seed_job and args.seed_task not in completed:
        seed = {
            "task": args.seed_task,
            **result_summary(
                args.seed_job,
                require_model_call=require_model_call,
                agent_import=args.agent_import,
            ),
            "reused": True,
        }
        append_progress(progress_path, seed)
        completed[args.seed_task] = seed

    task_dirs = sorted(
        path
        for path in args.dataset.iterdir()
        if path.is_dir() and (path / "task.toml").is_file()
    )
    pending = [path for path in task_dirs if path.name not in completed]
    env = os.environ.copy()
    env["PYTHONPATH"] = str(args.adapter_root)
    env["TIMEM_BENCH_BINARY"] = str(args.binary)

    for offset in range(0, len(pending), args.concurrency):
        batch = pending[offset : offset + args.concurrency]
        runnable: list[tuple[Path, str, bool]] = []
        for task_dir in batch:
            name = task_dir.name
            image = task_image(task_dir)
            preexisting = image_exists(image)
            if not preexisting:
                dockerfile = mirrored_dockerfile(task_dir, args.work_dir)
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
                    args.work_dir / "build-logs" / f"{name}.log",
                    env,
                )
                if return_code != 0:
                    append_progress(progress_path, {
                        "task": name,
                        "status": "infra_error",
                        "stage": "build",
                        "return_code": return_code,
                    })
                    continue
            runnable.append((task_dir, image, preexisting))

        with concurrent.futures.ThreadPoolExecutor(
            max_workers=args.concurrency
        ) as executor:
            futures = {
                executor.submit(run_task, task_dir, image, args, env): (
                    task_dir,
                    image,
                    preexisting,
                )
                for task_dir, image, preexisting in runnable
            }
            for future in concurrent.futures.as_completed(futures):
                task_dir, image, preexisting = futures[future]
                try:
                    record = future.result()
                except Exception as error:
                    record = {
                        "task": task_dir.name,
                        "status": "infra_error",
                        "stage": "runner",
                        "error": type(error).__name__,
                    }
                append_progress(progress_path, record)
                print(json.dumps(record, sort_keys=True), flush=True)
                if not preexisting:
                    subprocess.run(
                        ["docker", "image", "rm", image],
                        stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL,
                    )

        free_bytes = shutil.disk_usage(args.work_dir).free
        if free_bytes < 35 * 1024**3:
            subprocess.run(
                [
                    "docker", "buildx", "prune",
                    "--builder", args.builder,
                    "--all", "--force",
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            free_bytes = shutil.disk_usage(args.work_dir).free
        if free_bytes < 20 * 1024**3:
            append_progress(progress_path, {
                "task": "__run__",
                "status": "infra_error",
                "stage": "disk_guard",
                "free_bytes": free_bytes,
            })
            print("disk guard stopped run", flush=True)
            return 2

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
