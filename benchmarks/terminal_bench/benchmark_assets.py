"""Common, answer-neutral runtime assets for Terminal-Bench containers."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from harbor.environments.base import BaseEnvironment


def model_no_proxy(base_url: str) -> str:
    """Return a minimal proxy bypass list containing the configured model host."""
    hostname = urlsplit(base_url).hostname
    hosts = [hostname] if hostname else []
    hosts.extend(["localhost", "127.0.0.1"])
    return ",".join(dict.fromkeys(hosts))


async def install_verifier_assets(
    agent: Any,
    environment: BaseEnvironment,
) -> None:
    """Install pinned curl/uv assets to avoid flaky verifier downloads."""
    repository_root = Path(__file__).resolve().parents[2]
    benchmark_root = Path(__file__).resolve().parents[3]
    curl_bundle = Path(
        os.environ.get(
            "BENCH_CURL_BUNDLE",
            str(benchmark_root / "curl-8.5-bundle.tar.gz"),
        )
    )
    uv_binary = Path(
        os.environ.get(
            "BENCH_UV_BINARY",
            str(benchmark_root / "uv-0.9.5" / "uv"),
        )
    )
    curl_wrapper = repository_root / "benchmarks" / "terminal_bench" / "curl-wrapper"
    missing = [path for path in (curl_bundle, uv_binary, curl_wrapper) if not path.is_file()]
    if missing:
        raise FileNotFoundError("benchmark runtime asset missing: " + ", ".join(map(str, missing)))

    await agent.exec_as_root(
        environment,
        command="mkdir -p /opt/benchmark-assets /opt/timem-curl /root/.local/bin",
    )
    await environment.upload_file(curl_bundle, "/tmp/benchmark-curl.tar.gz")
    await environment.upload_file(uv_binary, "/opt/benchmark-assets/uv")
    await environment.upload_file(curl_wrapper, "/usr/local/bin/curl")
    await agent.exec_as_root(
        environment,
        command=(
            "tar -xzf /tmp/benchmark-curl.tar.gz -C /opt/timem-curl && "
            "chmod 755 /opt/benchmark-assets/uv /usr/local/bin/curl && "
            "ln -sf /opt/benchmark-assets/uv /root/.local/bin/uv && "
            "printf '%s\\n' 'export PATH=\"$HOME/.local/bin:$PATH\"' "
            "> /root/.local/bin/env"
        ),
    )
