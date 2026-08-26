"""Portable Python runtime helpers for installed-agent benchmark adapters."""

from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import Any

from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

from benchmarks.terminal_bench.benchmark_assets import (
    install_verifier_assets,
    model_no_proxy,
)


PYTHON = "/opt/bench-python/bin/python3.12"


async def install_python_agent(
    agent: Any,
    environment: BaseEnvironment,
    *,
    archive_env: str,
    destination: str,
    extra_archives: tuple[tuple[str, str], ...] = (),
) -> None:
    """Upload and extract a fixed Python runtime plus an agent package tree."""
    await install_verifier_assets(agent, environment)
    python_archive = Path(os.environ["PYTHON_BENCH_ARCHIVE"])
    agent_archive = Path(os.environ[archive_env])
    archives = [(python_archive, "/tmp/bench-python.tar.gz", "/opt/bench-python")]
    archives.append((agent_archive, "/tmp/bench-agent.tar.gz", destination))
    for env_key, remote_destination in extra_archives:
        archives.append(
            (Path(os.environ[env_key]), f"/tmp/{env_key.lower()}.tar.gz", remote_destination)
        )
    missing = [str(source) for source, _, _ in archives if not source.is_file()]
    if missing:
        raise FileNotFoundError("portable agent asset missing: " + ", ".join(missing))
    await agent.exec_as_root(
        environment,
        command="mkdir -p " + " ".join(destination for _, _, destination in archives),
    )
    for source, remote_archive, remote_destination in archives:
        await environment.upload_file(source, remote_archive)
        await agent.exec_as_root(
            environment,
            command=f"tar -xzf {remote_archive} -C {remote_destination}",
        )
    await agent.exec_as_root(environment, command=f"chmod 755 {PYTHON}")


def python_env(site: str, home: str) -> dict[str, str]:
    no_proxy = model_no_proxy(os.environ.get("TIMEM_BASE_URL", ""))
    return {
        "HOME": home,
        "PYTHONPATH": site,
        "PYTHONNOUSERSITE": "1",
        "PYTHONUNBUFFERED": "1",
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "NO_PROXY": no_proxy,
        "no_proxy": no_proxy,
    }


def populate_json_usage(path: Path, context: AgentContext) -> None:
    """Collect per-call token usage dictionaries from JSON or JSONL output."""
    if not path.is_file():
        return
    candidates: list[dict[str, Any]] = []

    def visit(value: Any) -> None:
        if isinstance(value, dict):
            lowered = {str(key).lower() for key in value}
            if lowered & {
                "input_tokens", "prompt_tokens", "output_tokens",
                "completion_tokens", "cache_read_input_tokens",
                "cached_input_tokens",
            }:
                candidates.append(value)
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)

    for line in path.read_text(errors="replace").splitlines():
        try:
            visit(json.loads(line))
        except json.JSONDecodeError:
            continue
    unique: list[dict[str, Any]] = []
    seen: set[str] = set()
    for candidate in candidates:
        marker = json.dumps(candidate, sort_keys=True, default=str)
        if marker not in seen:
            seen.add(marker)
            unique.append(candidate)
    input_tokens = output_tokens = cache_tokens = 0
    for usage in unique:
        input_tokens += int(
            usage.get("input_tokens", usage.get("prompt_tokens", 0)) or 0
        )
        output_tokens += int(
            usage.get("output_tokens", usage.get("completion_tokens", 0)) or 0
        )
        cache_tokens += int(
            usage.get(
                "cache_read_input_tokens",
                usage.get("cached_input_tokens", usage.get("cache_tokens", 0)),
            )
            or 0
        )
    if unique:
        context.n_input_tokens = input_tokens
        context.n_output_tokens = output_tokens
        context.n_cache_tokens = cache_tokens


def populate_aider_usage(path: Path, context: AgentContext) -> None:
    if not path.is_file():
        return

    def number(text: str) -> int:
        text = text.replace(",", "").strip().lower()
        multiplier = 1
        if text.endswith("k"):
            text, multiplier = text[:-1], 1000
        elif text.endswith("m"):
            text, multiplier = text[:-1], 1_000_000
        return int(float(text) * multiplier)

    sent = received = 0
    pattern = re.compile(
        r"Tokens:\s*([\d,.]+[kKmM]?)\s+sent,\s*([\d,.]+[kKmM]?)\s+received"
    )
    for match in pattern.finditer(path.read_text(errors="replace")):
        sent += number(match.group(1))
        received += number(match.group(2))
    if sent or received:
        context.n_input_tokens = sent
        context.n_output_tokens = received
