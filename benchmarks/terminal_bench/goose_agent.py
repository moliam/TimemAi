"""Harbor installed-agent adapter for Goose CLI."""

from __future__ import annotations

import json
import os
import shlex
from pathlib import Path
from typing import override

from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

from benchmarks.terminal_bench.benchmark_assets import (
    install_verifier_assets,
    model_no_proxy,
)
from benchmarks.terminal_bench.portable_python import populate_json_usage
from benchmarks.terminal_bench.process_cleanup import exec_agent_with_cleanup


class GooseAgent(BaseInstalledAgent):
    _BINARY = "/usr/local/bin/goose"
    _LOG = "goose.jsonl"

    @staticmethod
    @override
    def name() -> str:
        return "goose"

    @override
    def get_version_command(self) -> str | None:
        return f"{self._BINARY} --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await install_verifier_assets(self, environment)
        archive = Path(os.environ["GOOSE_BENCH_ARCHIVE"])
        if not archive.is_file():
            raise FileNotFoundError("Goose archive is missing")
        await environment.upload_file(archive, "/tmp/goose.tar.gz")
        await self.exec_as_root(
            environment,
            command=(
                "mkdir -p /opt/goose && tar -xzf /tmp/goose.tar.gz -C /opt/goose && "
                f"install -m 755 /opt/goose/goose {self._BINARY}"
            ),
        )
        ca_bundle = Path("/etc/ssl/certs/ca-certificates.crt")
        if not ca_bundle.is_file():
            raise FileNotFoundError("Host CA bundle is missing")
        await environment.upload_file(ca_bundle, "/opt/goose/ca-certificates.crt")

    @override
    @with_prompt_template
    async def run(
        self, instruction: str, environment: BaseEnvironment, context: AgentContext
    ) -> None:
        output = f"/logs/agent/{self._LOG}"
        base_url = os.environ["TIMEM_BASE_URL"].rstrip("/")
        host = base_url[:-3] if base_url.endswith("/v1") else base_url
        no_proxy = model_no_proxy(base_url)
        env = {
            "HOME": "/tmp/goose-home",
            "GOOSE_PATH_ROOT": "/tmp/goose-path",
            "GOOSE_PROVIDER": "openai",
            "GOOSE_MODEL": os.environ["TIMEM_MODEL"],
            "OPENAI_API_KEY": self._get_env("TIMEM_API_KEY") or "",
            "OPENAI_HOST": host,
            "OPENAI_BASE_URL": base_url,
            "OPENAI_BASE_PATH": "v1/chat/completions",
            "GOOSE_DISABLE_KEYRING": "1",
            "GOOSE_MODE": "auto",
            "SSL_CERT_FILE": "/opt/goose/ca-certificates.crt",
            "GOOSE_CA_CERT_PATH": "/opt/goose/ca-certificates.crt",
            "GOOSE_TELEMETRY_ENABLED": "false",
            "NO_PROXY": no_proxy,
            "no_proxy": no_proxy,
        }
        command = (
            "mkdir -p /tmp/goose-home /tmp/goose-path; "
            "test -n \"$OPENAI_API_KEY\" || { echo 'SETUP_ERROR: missing OPENAI_API_KEY'; exit 78; }; "
            "test -n \"$GOOSE_DISABLE_KEYRING\" || { echo 'SETUP_ERROR: missing GOOSE_DISABLE_KEYRING'; exit 78; }; "
            "set -o pipefail; "
            f"{self._BINARY} run --no-session --stats "
            "--provider openai "
            f"--model {shlex.quote(os.environ['TIMEM_MODEL'])} "
            "--with-builtin developer --max-turns 100 --output-format stream-json "
            f"--text {shlex.quote(instruction)} 2>&1 | tee {shlex.quote(output)}"
        )
        await exec_agent_with_cleanup(self, environment, command=command, env=env)

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        populate_json_usage(self.logs_dir / self._LOG, context)
