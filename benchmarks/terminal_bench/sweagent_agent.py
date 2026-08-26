"""Harbor installed-agent adapter for SWE-agent v1.1.0."""

from __future__ import annotations

import os
import shlex
from pathlib import Path
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


class SWEAgent(BaseInstalledAgent):
    _SITE = "/opt/agents/sweagent"
    _SOURCE = "/opt/agents/swe-source"
    _CLI = f"{_SITE}/bin/sweagent"
    _CONFIG = f"{_SOURCE}/config/terminal-bench.yaml"
    _LOG = "sweagent.log"

    @staticmethod
    @override
    def name() -> str:
        return "swe-agent"

    @override
    def get_version_command(self) -> str | None:
        return (
            f"PYTHONPATH={self._SOURCE}:{self._SITE} {PYTHON} -c "
            "'import sweagent; print(sweagent.__version__)'"
        )

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await install_python_agent(
            self,
            environment,
            archive_env="SWEAGENT_BENCH_ARCHIVE",
            destination=self._SITE,
            extra_archives=(("SWEAGENT_SOURCE_ARCHIVE", self._SOURCE),),
        )
        config = Path(__file__).with_name("sweagent_terminal_bench.yaml")
        await environment.upload_file(config, self._CONFIG)
        compat_init = Path(__file__).with_name("sweagent_init_compat.py")
        await environment.upload_file(
            compat_init, f"{self._SOURCE}/sweagent/__init__.py"
        )

    @override
    @with_prompt_template
    async def run(
        self, instruction: str, environment: BaseEnvironment, context: AgentContext
    ) -> None:
        output = f"/logs/agent/{self._LOG}"
        env = python_env(f"{self._SOURCE}:{self._SITE}", "/tmp/sweagent-home")
        env["OPENAI_API_KEY"] = self._get_env("TIMEM_API_KEY") or ""
        env["GIT_PYTHON_REFRESH"] = "quiet"
        command = (
            "mkdir -p /tmp/sweagent-home /logs/agent/swe-output; set -o pipefail; "
            f"{PYTHON} {self._CLI} run --config {self._CONFIG} "
            f"--agent.model.name {shlex.quote('openai/' + os.environ['TIMEM_MODEL'])} "
            f"--agent.model.api_base {shlex.quote(os.environ['TIMEM_BASE_URL'])} "
            "--agent.model.api_key '$OPENAI_API_KEY' "
            f"--problem_statement.type text --problem_statement.text {shlex.quote(instruction)} "
            "--output_dir /logs/agent/swe-output "
            f"2>&1 | tee {shlex.quote(output)}"
        )
        await exec_agent_with_cleanup(self, environment, command=command, env=env)

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        for path in (self.logs_dir / "swe-output").rglob("*.traj"):
            populate_json_usage(path, context)
            return
