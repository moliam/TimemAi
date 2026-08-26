"""Harbor installed-agent adapter for HKUDS OpenHarness."""

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


class OpenHarnessAgent(BaseInstalledAgent):
    _SITE = "/opt/agents/openharness"
    _CLI = f"{_SITE}/bin/oh"
    _LOG = "openharness.jsonl"

    @staticmethod
    @override
    def name() -> str:
        return "openharness"

    @override
    def get_version_command(self) -> str | None:
        return f"PYTHONPATH={self._SITE} {PYTHON} {self._CLI} --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await install_python_agent(
            self,
            environment,
            archive_env="OPENHARNESS_BENCH_ARCHIVE",
            destination=self._SITE,
        )

    @override
    @with_prompt_template
    async def run(
        self, instruction: str, environment: BaseEnvironment, context: AgentContext
    ) -> None:
        output = f"/logs/agent/{self._LOG}"
        env = python_env(self._SITE, "/tmp/openharness-home")
        env["OPENAI_API_KEY"] = self._get_env("TIMEM_API_KEY") or ""
        env["OPENAI_BASE_URL"] = os.environ["TIMEM_BASE_URL"]
        env["OPENHARNESS_API_FORMAT"] = "openai"
        command = (
            "mkdir -p /tmp/openharness-home; set -o pipefail; "
            f"{PYTHON} {self._CLI} --bare --dangerously-skip-permissions "
            "--api-format openai "
            f"--base-url {shlex.quote(os.environ['TIMEM_BASE_URL'])} "
            f"--model {shlex.quote(os.environ['TIMEM_MODEL'])} --max-turns 100 "
            f"--print {shlex.quote(instruction)} --output-format stream-json "
            f"2>&1 | tee {shlex.quote(output)}"
        )
        await exec_agent_with_cleanup(self, environment, command=command, env=env)

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        populate_json_usage(self.logs_dir / self._LOG, context)
