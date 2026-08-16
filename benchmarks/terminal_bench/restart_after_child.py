"""Restart a paused matrix runner after its active Harbor child exits.

This helper preserves the paused runner's environment in memory, including model
configuration, without writing it to disk or logs.  It is intended for a safe
runner-code upgrade at a trial boundary.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import time
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runner-pid", type=int, required=True)
    parser.add_argument("--child-pid", type=int, action="append", required=True)
    parser.add_argument("--job-dir", type=Path, action="append", required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--status", type=Path, required=True)
    return parser.parse_args()


def proc_bytes(pid: int, name: str) -> bytes:
    return Path(f"/proc/{pid}/{name}").read_bytes()


def child_finished(pid: int) -> bool:
    stat = Path(f"/proc/{pid}/stat")
    if not stat.is_file():
        return True
    # The process name is parenthesized and can contain spaces.  The state is
    # the first field after the final closing parenthesis.
    state = stat.read_text().rsplit(")", 1)[1].strip().split()[0]
    return state == "Z"


def result_complete(job_dir: Path) -> bool:
    result = job_dir / "result.json"
    try:
        stats = json.loads(result.read_text()).get("stats") or {}
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        return False
    return stats.get("n_completed_trials") == 1


def main() -> int:
    args = parse_args()
    if len(args.child_pid) != len(args.job_dir):
        raise SystemExit("each child-pid requires one corresponding job-dir")
    runner = args.runner_pid
    command = [
        item.decode()
        for item in proc_bytes(runner, "cmdline").split(b"\0")
        if item
    ]
    environment = {}
    for item in proc_bytes(runner, "environ").split(b"\0"):
        if not item or b"=" not in item:
            continue
        key, value = item.split(b"=", 1)
        environment[key.decode()] = value.decode()
    cwd = Path(f"/proc/{runner}/cwd").resolve()

    while not all(child_finished(pid) for pid in args.child_pid):
        time.sleep(2)
    # Harbor writes the aggregate result before it exits.  Allow filesystem
    # propagation, but never discard a completed trial just to restart faster.
    for _ in range(30):
        if all(result_complete(job_dir) for job_dir in args.job_dir):
            break
        time.sleep(1)
    else:
        incomplete = [
            str(job_dir)
            for job_dir in args.job_dir
            if not result_complete(job_dir)
        ]
        args.status.write_text(
            json.dumps({"status": "result_incomplete", "job_dirs": incomplete})
            + "\n"
        )
        os.kill(runner, signal.SIGCONT)
        return 2

    os.kill(runner, signal.SIGKILL)
    time.sleep(1)
    args.log.parent.mkdir(parents=True, exist_ok=True)
    with args.log.open("ab", buffering=0) as log_handle:
        restarted = subprocess.Popen(
            command,
            cwd=cwd,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    args.status.write_text(
        json.dumps(
            {
                "status": "restarted",
                "old_pid": runner,
                "new_pid": restarted.pid,
            },
            sort_keys=True,
        )
        + "\n"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
