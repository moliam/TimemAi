"""Harbor installed-agent adapter for OpenHands CLI."""

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
    populate_json_usage,
    python_env,
)
from benchmarks.terminal_bench.process_cleanup import exec_agent_with_cleanup


class OpenHandsAgent(BaseInstalledAgent):
    _SITE = "/opt/agents/openhands"
    _CLI = f"{_SITE}/bin/openhands"
    _LOG = "openhands.jsonl"

    @staticmethod
    @override
    def name() -> str:
        return "openhands"

    @override
    def get_version_command(self) -> str | None:
        return f"PYTHONPATH={self._SITE} {PYTHON} {self._CLI} --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await install_python_agent(
            self, environment, archive_env="OPENHANDS_BENCH_ARCHIVE", destination=self._SITE
        )

    @override
    @with_prompt_template
    async def run(
        self, instruction: str, environment: BaseEnvironment, context: AgentContext
    ) -> None:
        output = f"/logs/agent/{self._LOG}"
        env = python_env(self._SITE, "/tmp/openhands-home")
        env.update(
            {
                "LLM_API_KEY": self._get_env("TIMEM_API_KEY") or "",
                "LLM_MODEL": "openai/" + os.environ["TIMEM_MODEL"],
                "LLM_BASE_URL": os.environ["TIMEM_BASE_URL"],
                "DISABLE_TELEMETRY": "1",
            }
        )
        command = (
            "mkdir -p /tmp/openhands-home; set -o pipefail; "
            f"{PYTHON} {self._CLI} --headless --json --override-with-envs "
            f"--task {shlex.quote(instruction)} 2>&1 | tee {shlex.quote(output)}"
        )
        await exec_agent_with_cleanup(self, environment, command=command, env=env)

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        populate_json_usage(self.logs_dir / self._LOG, context)
