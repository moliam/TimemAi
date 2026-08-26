"""Harbor installed-agent adapter for Aider."""

from __future__ import annotations

import os
import shlex
from typing import override

from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

from benchmarks.terminal_bench.portable_python import (
    PYTHON,
    install_python_agent,
    populate_aider_usage,
    python_env,
)
from benchmarks.terminal_bench.process_cleanup import exec_agent_with_cleanup


class AiderAgent(BaseInstalledAgent):
    _SITE = "/opt/agents/aider"
    _CLI = f"{_SITE}/bin/aider"
    _LOG = "aider.log"

    @staticmethod
    @override
    def name() -> str:
        return "aider"

    @override
    def get_version_command(self) -> str | None:
        return f"PYTHONPATH={self._SITE} {PYTHON} {self._CLI} --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await install_python_agent(
            self, environment, archive_env="AIDER_BENCH_ARCHIVE", destination=self._SITE
        )

    @override
    @with_prompt_template
    async def run(
        self, instruction: str, environment: BaseEnvironment, context: AgentContext
    ) -> None:
        output = f"/logs/agent/{self._LOG}"
        env = python_env(self._SITE, "/tmp/aider-home")
        env["OPENAI_API_KEY"] = self._get_env("TIMEM_API_KEY") or ""
        command = (
            "mkdir -p /tmp/aider-home; set -o pipefail; "
            f"{PYTHON} {self._CLI} "
            f"--model {shlex.quote('openai/' + os.environ['TIMEM_MODEL'])} "
            f"--openai-api-base {shlex.quote(os.environ['TIMEM_BASE_URL'])} "
            "--yes-always --no-git --no-auto-commits --no-pretty --no-stream "
            "--no-check-update --no-show-model-warnings --disable-playwright "
            f"--message {shlex.quote(instruction)} 2>&1 | tee {shlex.quote(output)}"
        )
        await exec_agent_with_cleanup(self, environment, command=command, env=env)

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        populate_aider_usage(self.logs_dir / self._LOG, context)
