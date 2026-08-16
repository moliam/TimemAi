"""Harbor installed-agent adapter for the Timem shell agent."""

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
from benchmarks.terminal_bench.process_cleanup import exec_agent_with_cleanup


class TimemShellAgent(BaseInstalledAgent):
    """Install a prebuilt Timem binary and run one autonomous benchmark turn."""

    _OUTPUT_FILENAME = "timem.json"
    _REMOTE_BINARY = "/usr/local/bin/timem-native-rs"

    @staticmethod
    @override
    def name() -> str:
        return "timem-shell"

    @override
    def get_version_command(self) -> str | None:
        return None

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        repository_root = Path(__file__).resolve().parents[2]
        binary = Path(
            os.environ.get(
                "TIMEM_BENCH_BINARY",
                str(
                    repository_root
                    / ".benchmark-cache"
                    / "linux-target"
                    / "release"
                    / "timem-native-rs"
                ),
            )
        )
        if not binary.is_file():
            raise FileNotFoundError(
                f"Linux Timem binary not found at {binary}. "
                "Build it before starting Harbor."
            )

        await environment.upload_file(binary, self._REMOTE_BINARY)
        await install_verifier_assets(self, environment)
        await self.exec_as_root(
            environment,
            command=f"chmod 755 {shlex.quote(self._REMOTE_BINARY)}",
        )

    @override
    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        output_path = f"/logs/agent/{self._OUTPUT_FILENAME}"
        required_model_env = (
            "TIMEM_API_KEY",
            "TIMEM_API_PROTOCOL",
            "TIMEM_BASE_URL",
            "TIMEM_MODEL",
            "TIMEM_RESPONSE_PROTOCOL",
        )
        missing = [name for name in required_model_env if not self._get_env(name)]
        if missing:
            raise ValueError(
                "Missing Timem model environment: " + ", ".join(missing)
            )

        fixed_env = {
            name: self._get_env(name) or "" for name in required_model_env
        }
        for optional_name in (
            "TIMEM_MAX_LLM_INPUT",
            "TIMEM_MAX_LLM_OUTPUT",
            "TIMEM_TIMEOUT",
            "TIMEM_ENABLE_THINKING",
            "TIMEM_REASONING_EFFORT",
        ):
            if value := self._get_env(optional_name):
                fixed_env[optional_name] = value

        no_proxy = model_no_proxy(fixed_env["TIMEM_BASE_URL"])
        fixed_env.update({
            "TIMEM_BASH_APPROVAL": "approve",
            "TIMEM_WORK_INSTRUCTIONS": "off",
            "TIMEM_MAX_ROUNDS": "300",
            "TIMEM_DATA_DIR": "/tmp/timem-benchmark-data",
            "TIMEM_SPACE": "terminal-bench",
            "TIMEM_STREAM": "false",
            "NO_PROXY": no_proxy,
            "no_proxy": no_proxy,
        })
        await exec_agent_with_cleanup(
            self,
            environment,
            command=(
                f"{shlex.quote(self._REMOTE_BINARY)} "
                f"--once-json {shlex.quote(instruction)} "
                "--work-instructions off "
                f"| tee {shlex.quote(output_path)}"
            ),
            env=fixed_env,
        )

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        output_path = self.logs_dir / self._OUTPUT_FILENAME
        if not output_path.is_file():
            return

        try:
            payload = json.loads(output_path.read_text())
        except (OSError, json.JSONDecodeError):
            return

        stats = payload.get("stats") or {}
        context.n_input_tokens = stats.get("prompt_tokens", 0)
        context.n_cache_tokens = stats.get("cached_tokens", 0)
        context.n_output_tokens = stats.get("completion_tokens", 0)
        context.metadata = {
            "timem_status": payload.get("status"),
            "timem_elapsed_ms": payload.get("elapsed_ms"),
            "timem_tool_calls": stats.get("tool_calls", 0),
            "timem_llm_calls": stats.get("llm_calls", 0),
        }
