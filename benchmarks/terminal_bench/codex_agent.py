"""Harbor installed-agent adapter for Codex CLI."""

from __future__ import annotations

import json
import os
import shlex
from pathlib import Path
from typing import override

from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

from benchmarks.terminal_bench.benchmark_assets import install_verifier_assets


class CodexAgent(BaseInstalledAgent):
    """Run Codex CLI against the same GPT-5.6-sol endpoint as Timem."""

    _BINARY = "/usr/local/bin/codex"
    _LOG = "codex.jsonl"

    @staticmethod
    @override
    def name() -> str:
        return "codex-cli"

    @override
    def get_version_command(self) -> str | None:
        return f"{self._BINARY} --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await install_verifier_assets(self, environment)
        binary = Path(os.environ["CODEX_BENCH_BINARY"])
        if not binary.is_file():
            raise FileNotFoundError("Codex binary is missing")
        await environment.upload_file(binary, self._BINARY)
        await self.exec_as_root(
            environment,
            command=f"chmod 755 {self._BINARY}",
        )

    @override
    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        output_path = f"/logs/agent/{self._LOG}"
        env = {
            "TIMEM_API_KEY": self._get_env("TIMEM_API_KEY") or "",
            "HOME": "/tmp/codex-home",
            "NO_PROXY": "10.125.112.83,localhost,127.0.0.1",
            "no_proxy": "10.125.112.83,localhost,127.0.0.1",
        }
        base_url = os.environ["TIMEM_BASE_URL"]
        model = os.environ["TIMEM_MODEL"]
        command = (
            "mkdir -p /tmp/codex-home && "
            f"{self._BINARY} exec --json --skip-git-repo-check "
            "--dangerously-bypass-approvals-and-sandbox "
            f"--model {shlex.quote(model)} "
            "-c model_provider=\"timem\" "
            "-c model_providers.timem.name=\"Timem endpoint\" "
            f"-c model_providers.timem.base_url={shlex.quote(json.dumps(base_url))} "
            "-c model_providers.timem.env_key=\"TIMEM_API_KEY\" "
            "-c model_providers.timem.wire_api=\"responses\" "
            f"{shlex.quote(instruction)} | tee {shlex.quote(output_path)}"
        )
        await self.exec_as_agent(environment, command=command, env=env)

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        path = self.logs_dir / self._LOG
        if not path.is_file():
            return
        for line in reversed(path.read_text(errors="replace").splitlines()):
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            usage = event.get("usage") or {}
            if usage:
                context.n_input_tokens = usage.get("input_tokens", 0)
                context.n_cache_tokens = usage.get("cached_input_tokens", 0)
                context.n_output_tokens = usage.get("output_tokens", 0)
                break
