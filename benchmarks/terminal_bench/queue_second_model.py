"""Queue a second model matrix after the legacy GPT matrix is complete.

The second-model credential is read once from stdin, retained only in process
memory, and inherited by the eventual matrix process. Status files never
contain endpoint, credential, prompts, or response bodies.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

from benchmarks.terminal_bench.run_matrix_server import configs, load_completed


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--gpt-runner-pid", type=int, required=True)
    parser.add_argument("--run-tag", required=True)
    parser.add_argument("--status", type=Path, required=True)
    parser.add_argument("--poll-seconds", type=int, default=60)
    return parser.parse_args()


def read_environment(pid: int) -> dict[str, str]:
    output: dict[str, str] = {}
    for item in Path(f"/proc/{pid}/environ").read_bytes().split(b"\0"):
        if not item or b"=" not in item:
            continue
        key, value = item.split(b"=", 1)
        output[key.decode()] = value.decode()
    return output


def write_status(path: Path, phase: str, **details: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(
            {"phase": phase, "updated_at": time.time(), **details},
            sort_keys=True,
        )
        + "\n"
    )


def daemonize() -> None:
    if os.fork() > 0:
        return
    os.setsid()
    if os.fork() > 0:
        os._exit(0)
    null_fd = os.open(os.devnull, os.O_RDWR)
    for fd in (0, 1, 2):
        os.dup2(null_fd, fd)
    if null_fd > 2:
        os.close(null_fd)
    supervise()
    os._exit(0)


def matrix_complete(root: Path) -> tuple[bool, dict[str, int]]:
    dataset_tasks = {
        path.name
        for path in (root / "dataset").iterdir()
        if path.is_dir() and (path / "task.toml").is_file()
    }
    completed = {
        config.label: set(load_completed(config)) & dataset_tasks
        for config in configs(root)
    }
    counts = {label: len(tasks) for label, tasks in completed.items()}
    return (
        len(dataset_tasks) == 89
        and all(tasks == dataset_tasks for tasks in completed.values()),
        counts,
    )


def process_commands() -> list[list[str]]:
    commands = []
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            raw = (proc / "cmdline").read_bytes()
        except OSError:
            continue
        command = [item.decode(errors="replace") for item in raw.split(b"\0") if item]
        if command:
            commands.append(command)
    return commands


def legacy_runner_active() -> bool:
    for command in process_commands():
        joined = " ".join(command)
        if "benchmarks.terminal_bench.run_matrix_server" not in joined:
            continue
        if "--run-tag" not in command:
            return True
    return False


def probe(config: dict[str, str]) -> tuple[bool, str | None]:
    url = config["base_url"].rstrip("/") + "/chat/completions"
    request = urllib.request.Request(
        url,
        data=json.dumps(
            {
                "model": config["model_label"],
                "messages": [{"role": "user", "content": "Reply only OK."}],
                "max_tokens": 8,
            }
        ).encode(),
        headers={
            "Authorization": f'Bearer {config["api_key"]}',
            "Content-Type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            payload = json.loads(response.read())
        choices = payload.get("choices") or []
        if not choices:
            return False, "response_without_choices"
        return True, None
    except urllib.error.HTTPError as error:
        return False, f"http_{error.code}"
    except (OSError, ValueError, json.JSONDecodeError) as error:
        return False, type(error).__name__


def start_runner(
    root: Path,
    environment: dict[str, str],
    log_path: Path,
    *,
    model_label: str | None = None,
    run_tag: str | None = None,
) -> subprocess.Popen[bytes]:
    command = [
        str(root / "tools/python/cpython-3.12.12-linux-x86_64-gnu/bin/python3"),
        "-m",
        "benchmarks.terminal_bench.run_matrix_server",
        "--root",
        str(root),
        "--concurrency",
        "2",
        "--max-passes",
        "10",
    ]
    if model_label is not None:
        command.extend(["--model-label", model_label])
    if run_tag is not None:
        command.extend(["--run-tag", run_tag])
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("ab", buffering=0) as log_handle:
        return subprocess.Popen(
            command,
            cwd=root / "adapter",
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )


def supervise() -> None:
    args = GLOBALS["args"]
    root: Path = args.root
    gpt_environment: dict[str, str] = GLOBALS["gpt_environment"]
    second: dict[str, str] = GLOBALS["second"]
    second_environment = gpt_environment.copy()
    second_environment.update(
        {
            "TIMEM_API_KEY": second["api_key"],
            "TIMEM_BASE_URL": second["base_url"],
            "TIMEM_MODEL": second["model_label"],
        }
    )
    write_status(args.status, "waiting_for_gpt")
    while True:
        complete, counts = matrix_complete(root)
        if complete and not legacy_runner_active():
            break
        if not complete and not legacy_runner_active():
            restarted = start_runner(
                root,
                gpt_environment,
                root / "full-matrix-shared-20260815/runner.log",
            )
            write_status(
                args.status,
                "gpt_restarted",
                pid=restarted.pid,
                counts=counts,
            )
        else:
            write_status(args.status, "waiting_for_gpt", counts=counts)
        time.sleep(args.poll_seconds)

    while True:
        ok, error = probe(second)
        if ok:
            break
        write_status(args.status, "second_model_probe_failed", error=error)
        time.sleep(max(args.poll_seconds, 300))

    log_path = root / f"matrix-{args.run_tag}/runner.log"
    while True:
        runner = start_runner(
            root,
            second_environment,
            log_path,
            model_label=second["model_label"],
            run_tag=args.run_tag,
        )
        write_status(args.status, "second_model_running", pid=runner.pid)
        return_code = runner.wait()
        if return_code == 0:
            write_status(args.status, "second_model_complete", pid=runner.pid)
            return
        write_status(
            args.status,
            "second_model_runner_retry",
            pid=runner.pid,
            return_code=return_code,
        )
        time.sleep(args.poll_seconds)


GLOBALS: dict[str, object] = {}


def main() -> int:
    args = parse_args()
    second = json.load(sys.stdin)
    required = {"base_url", "api_key", "model_label"}
    missing = sorted(required - set(second))
    if missing:
        raise SystemExit("missing second-model fields: " + ", ".join(missing))
    if args.poll_seconds < 10:
        raise SystemExit("poll-seconds must be at least 10")
    gpt_environment = read_environment(args.gpt_runner_pid)
    GLOBALS.update(
        {
            "args": args,
            "second": {key: str(second[key]) for key in required},
            "gpt_environment": gpt_environment,
        }
    )
    daemonize()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
