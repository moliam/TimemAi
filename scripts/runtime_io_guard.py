#!/usr/bin/env python3
"""Measure Timem's average physical disk I/O during a real work interval."""

from __future__ import annotations

import argparse
import ctypes
import dataclasses
import json
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import time

DEFAULT_LIMIT_BPS = 500_000
DEFAULT_SAMPLE_SECONDS = 0.10


@dataclasses.dataclass(frozen=True)
class Counters:
    read_bytes: int
    write_bytes: int


class ProcessTreeSampler:
    def __init__(self, root_pid: int):
        self.root_pid = root_pid
        self.last: dict[tuple[int, str], Counters] = {}
        self.read_bytes = 0
        self.write_bytes = 0

    def establish_baseline(self) -> None:
        self.last = self._current()

    def sample(self) -> None:
        current = self._current()
        for key, counters in current.items():
            previous = self.last.get(key)
            if previous is None:
                # A child launched by Timem is part of the measured workload.
                self.read_bytes += counters.read_bytes
                self.write_bytes += counters.write_bytes
            else:
                self.read_bytes += max(0, counters.read_bytes - previous.read_bytes)
                self.write_bytes += max(0, counters.write_bytes - previous.write_bytes)
        self.last = current

    def _current(self) -> dict[tuple[int, str], Counters]:
        result: dict[tuple[int, str], Counters] = {}
        for pid in process_tree(self.root_pid):
            sampled = process_io(pid)
            if sampled is not None:
                identity, counters = sampled
                result[(pid, identity)] = counters
        return result


class RusageInfoV2(ctypes.Structure):
    _fields_ = [
        ("ri_uuid", ctypes.c_uint8 * 16),
        ("ri_user_time", ctypes.c_uint64),
        ("ri_system_time", ctypes.c_uint64),
        ("ri_pkg_idle_wkups", ctypes.c_uint64),
        ("ri_interrupt_wkups", ctypes.c_uint64),
        ("ri_pageins", ctypes.c_uint64),
        ("ri_wired_size", ctypes.c_uint64),
        ("ri_resident_size", ctypes.c_uint64),
        ("ri_phys_footprint", ctypes.c_uint64),
        ("ri_proc_start_abstime", ctypes.c_uint64),
        ("ri_proc_exit_abstime", ctypes.c_uint64),
        ("ri_child_user_time", ctypes.c_uint64),
        ("ri_child_system_time", ctypes.c_uint64),
        ("ri_child_pkg_idle_wkups", ctypes.c_uint64),
        ("ri_child_interrupt_wkups", ctypes.c_uint64),
        ("ri_child_pageins", ctypes.c_uint64),
        ("ri_child_elapsed_abstime", ctypes.c_uint64),
        ("ri_diskio_bytesread", ctypes.c_uint64),
        ("ri_diskio_byteswritten", ctypes.c_uint64),
    ]


def process_tree(root_pid: int) -> list[int]:
    if sys.platform.startswith("linux"):
        parent_by_pid: dict[int, int] = {}
        try:
            entries = pathlib.Path("/proc").iterdir()
        except OSError:
            return [root_pid]
        for entry in entries:
            if not entry.name.isdigit():
                continue
            try:
                fields = (entry / "stat").read_text().split()
                parent_by_pid[int(entry.name)] = int(fields[3])
            except (OSError, ValueError, IndexError):
                continue
    elif sys.platform == "darwin":
        try:
            output = subprocess.check_output(
                ["ps", "-axo", "pid=,ppid="], text=True, stderr=subprocess.DEVNULL
            )
        except (OSError, subprocess.SubprocessError):
            return [root_pid]
        parent_by_pid = {}
        for line in output.splitlines():
            fields = line.split()
            if len(fields) == 2:
                try:
                    parent_by_pid[int(fields[0])] = int(fields[1])
                except ValueError:
                    pass
    else:
        raise RuntimeError(f"unsupported platform: {sys.platform}")

    result = [root_pid]
    known = {root_pid}
    changed = True
    while changed:
        changed = False
        for pid, ppid in parent_by_pid.items():
            if pid not in known and ppid in known:
                known.add(pid)
                result.append(pid)
                changed = True
    return result


def process_io(pid: int) -> tuple[str, Counters] | None:
    if sys.platform.startswith("linux"):
        try:
            stat = pathlib.Path(f"/proc/{pid}/stat").read_text().split()
            values: dict[str, int] = {}
            for line in pathlib.Path(f"/proc/{pid}/io").read_text().splitlines():
                key, value = line.split(":", 1)
                values[key] = int(value.strip())
            return stat[21], Counters(values["read_bytes"], values["write_bytes"])
        except (OSError, ValueError, IndexError, KeyError):
            return None
    if sys.platform == "darwin":
        libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        function = libproc.proc_pid_rusage
        function.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        function.restype = ctypes.c_int
        info = RusageInfoV2()
        if function(pid, 2, ctypes.byref(info)) != 0:
            return None
        return str(info.ri_proc_start_abstime), Counters(
            info.ri_diskio_bytesread, info.ri_diskio_byteswritten
        )
    raise RuntimeError(f"unsupported platform: {sys.platform}")


def wait_for_file(path: pathlib.Path, process: subprocess.Popen[bytes], timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            return
        if process.poll() is not None:
            raise RuntimeError(f"workload exited before creating {path.name}")
        time.sleep(0.05)
    raise RuntimeError(f"timed out waiting for {path.name}")


def stop_process(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


def run(args: argparse.Namespace) -> int:
    root = pathlib.Path(__file__).resolve().parents[1]
    report_path = pathlib.Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="timem-runtime-io-") as tmp:
        markers = pathlib.Path(tmp)
        pid_file = markers / "pid"
        start_file = markers / "start"
        end_file = markers / "end"
        env = os.environ.copy()
        env.update(
            TIMEM_RUNTIME_IO_PID_FILE=str(pid_file),
            TIMEM_RUNTIME_IO_START_FILE=str(start_file),
            TIMEM_RUNTIME_IO_END_FILE=str(end_file),
        )
        process = subprocess.Popen(
            [str(root / "scripts/real_tty_stress.expect")],
            cwd=root,
            env=env,
            start_new_session=True,
        )
        try:
            wait_for_file(pid_file, process, args.start_timeout)
            timem_pid = int(pid_file.read_text().strip())
            wait_for_file(start_file, process, args.start_timeout)
            sampler = ProcessTreeSampler(timem_pid)
            sampler.establish_baseline()
            started = time.monotonic()
            while not end_file.exists():
                if process.poll() is not None:
                    raise RuntimeError("workload exited before the measured interval ended")
                time.sleep(args.sample_seconds)
                sampler.sample()
            sampler.sample()
            duration = max(time.monotonic() - started, 0.001)
            return_code = process.wait(timeout=args.finish_timeout)
            if return_code != 0:
                raise RuntimeError(f"real TTY workload failed with exit code {return_code}")
        except Exception as error:
            stop_process(process)
            report_path.write_text(
                json.dumps(
                    {
                        "workload": "real_tty_stress",
                        "limit_bps": args.limit_bps,
                        "status": "error",
                        "error": str(error),
                    },
                    indent=2,
                    sort_keys=True,
                )
                + "\n"
            )
            print(f"runtime_io_guard: error: {error}; report: {report_path}", file=sys.stderr)
            return 2

    total = sampler.read_bytes + sampler.write_bytes
    average_bps = total / duration
    report = {
        "workload": "real_tty_stress",
        "limit_bps": args.limit_bps,
        "duration_seconds": round(duration, 3),
        "read_bytes": sampler.read_bytes,
        "write_bytes": sampler.write_bytes,
        "total_bytes": total,
        "average_bps": round(average_bps),
        "status": "passed" if average_bps <= args.limit_bps else "failed",
    }
    report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(
        "runtime_io_guard: "
        f"average={average_bps:.0f} B/s, limit={args.limit_bps} B/s, "
        f"read={sampler.read_bytes}, write={sampler.write_bytes}, duration={duration:.2f}s"
    )
    if average_bps > args.limit_bps:
        print(f"runtime_io_guard: failed; report: {report_path}", file=sys.stderr)
        return 1
    print(f"runtime_io_guard: ok; report: {report_path}")
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--limit-bps", type=int, default=DEFAULT_LIMIT_BPS)
    parser.add_argument("--sample-seconds", type=float, default=DEFAULT_SAMPLE_SECONDS)
    parser.add_argument("--start-timeout", type=float, default=60.0)
    parser.add_argument("--finish-timeout", type=float, default=10.0)
    parser.add_argument("--report", default="target/runtime-io-guard/report.json")
    args = parser.parse_args()
    if args.limit_bps <= 0 or args.sample_seconds <= 0:
        parser.error("limit and sample interval must be positive")
    return args


if __name__ == "__main__":
    raise SystemExit(run(parse_args()))
