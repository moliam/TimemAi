"""Run an installed agent with timeout-safe in-container process cleanup."""

from __future__ import annotations

import json
import shlex
from collections.abc import Mapping
from pathlib import Path
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from harbor.environments.base import BaseEnvironment


PID_FILE = "/logs/agent/benchmark-agent-root.pid"
CLEANUP_FILE = "/logs/agent/benchmark-process-cleanup.json"


def wrap_agent_command(command: str) -> str:
    """Wrap an agent command and persist the root PID of its process tree."""
    return f"""
set -o pipefail
rm -f {shlex.quote(PID_FILE)} {shlex.quote(CLEANUP_FILE)}
(
{command}
) &
benchmark_agent_root_pid=$!
printf '%s\\n' "$benchmark_agent_root_pid" > {shlex.quote(PID_FILE)}
wait "$benchmark_agent_root_pid"
""".strip()


def process_cleanup_command() -> str:
    """Build the in-container command that terminates the recorded tree."""
    return f"""
set +e
pid_file={shlex.quote(PID_FILE)}
cleanup_file={shlex.quote(CLEANUP_FILE)}
cleanup_ok=false
pid_file_found=false
collect_tree() {{
    current="$1"
    test -d "/proc/$current" || return 0
    children=$(cat "/proc/$current/task/$current/children" 2>/dev/null)
    for child in $children; do collect_tree "$child"; done
    printf '%s\\n' "$current"
}}
if test -s "$pid_file"; then
    pid_file_found=true
    root_pid=$(head -n 1 "$pid_file")
    case "$root_pid" in
        ''|*[!0-9]*) cleanup_ok=false ;;
        *)
            pids=$(collect_tree "$root_pid")
            if test -n "$pids"; then
                kill -TERM $pids 2>/dev/null
                sleep 1
                kill -KILL $pids 2>/dev/null
                sleep 0.2
            fi
            remaining=false
            for pid in $pids; do
                if test -d "/proc/$pid"; then remaining=true; fi
            done
            if test "$remaining" = false; then cleanup_ok=true; fi
            ;;
    esac
fi
printf '{{"version":1,"pid_file_found":%s,"cleanup_completed":%s}}\\n' \
    "$pid_file_found" "$cleanup_ok" > "$cleanup_file"
""".strip()


async def exec_agent_with_cleanup(
    agent: Any,
    environment: BaseEnvironment,
    *,
    command: str,
    env: Mapping[str, str],
) -> None:
    """Ensure a cancelled Docker exec cannot keep mutating during verification."""
    wrapped = wrap_agent_command(command)
    try:
        await agent.exec_as_agent(environment, command=wrapped, env=dict(env))
    finally:
        # asyncio.wait_for cancels the adapter coroutine at the official task
        # deadline. Harbor's Docker exec transport can leave the in-container
        # process alive after that cancellation, so terminate the recorded tree
        # before the adapter is allowed to return and verification can begin.
        await agent.exec_as_root(environment, command=process_cleanup_command())


def cleanup_completed(job_dir: Path | str) -> bool:
    """Return true when an adapter persisted successful process cleanup."""
    root = Path(job_dir)
    for marker in root.glob(f"*/agent/{CLEANUP_FILE.rsplit('/', 1)[-1]}"):
        try:
            payload = json.loads(marker.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        if payload.get("cleanup_completed") is True:
            return True
    return False
