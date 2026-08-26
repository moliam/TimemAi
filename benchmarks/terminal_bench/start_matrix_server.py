"""Start the comparison matrix detached from an SSH control connection."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

from benchmarks.terminal_bench.run_full_server import MODEL_ENV_KEYS


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--concurrency", type=int, default=2)
    parser.add_argument("--max-passes", type=int, default=10)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    missing = [key for key in MODEL_ENV_KEYS if not os.environ.get(key)]
    if missing:
        raise SystemExit("missing model environment: " + ", ".join(missing))

    adapter_root = args.root / "adapter"
    work_dir = args.root / "full-matrix-shared-20260815"
    work_dir.mkdir(parents=True, exist_ok=True)
    log_path = work_dir / "runner.log"
    env = os.environ.copy()
    env["PYTHONPATH"] = str(adapter_root)
    command = [
        sys.executable,
        "-m",
        "benchmarks.terminal_bench.run_matrix_server",
        "--root",
        str(args.root),
        "--concurrency",
        str(args.concurrency),
        "--max-passes",
        str(args.max_passes),
    ]
    with log_path.open("a") as log:
        process = subprocess.Popen(
            command,
            cwd=adapter_root,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    print(process.pid)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
