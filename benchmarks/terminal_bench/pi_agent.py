"""Harbor installed-agent adapter for the Pi coding agent."""

from __future__ import annotations

import json
import os
import shlex
import tempfile
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


class PiAgent(BaseInstalledAgent):
    """Run Pi with the same OpenAI-compatible model used by Timem."""

    _NODE = "/usr/local/bin/node"
    _PI_ROOT = "/opt/pi"
    _PI_CLI = "/opt/pi/node_modules/@earendil-works/pi-coding-agent/dist/cli.js"
    _LOG = "pi.jsonl"

    @staticmethod
    @override
    def name() -> str:
        return "pi"

    @override
    def get_version_command(self) -> str | None:
        return f"{self._NODE} {self._PI_CLI} --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await install_verifier_assets(self, environment)
        archive = Path(os.environ["PI_BENCH_ARCHIVE"])
        node = Path(os.environ["PI_BENCH_NODE"])
        if not archive.is_file() or not node.is_file():
            raise FileNotFoundError("Pi archive or Node binary is missing")

        await environment.upload_file(node, self._NODE)
        await environment.upload_file(archive, "/tmp/pi-agent.tar.gz")
        await self.exec_as_root(
            environment,
            command=(
                f"chmod 755 {self._NODE} && mkdir -p {self._PI_ROOT} "
                "/tmp/pi-agent-config && "
                f"tar -xzf /tmp/pi-agent.tar.gz -C {self._PI_ROOT}"
            ),
        )

        model_config = {
            "providers": {
                "timem": {
                    "baseUrl": os.environ["TIMEM_BASE_URL"],
                    "api": "openai-completions",
                    "apiKey": "$TIMEM_API_KEY",
                    "authHeader": True,
                    "models": [{
                        "id": os.environ["TIMEM_MODEL"],
                        "name": os.environ["TIMEM_MODEL"],
                        "reasoning": True,
                        "contextWindow": int(os.environ.get("TIMEM_MAX_LLM_INPUT", "200000")),
                        "maxTokens": int(os.environ.get("TIMEM_MAX_LLM_OUTPUT", "20000")),
                        "compat": {
                            "supportsDeveloperRole": False,
                            "supportsReasoningEffort": False,
                        },
                    }],
                }
            }
        }
        with tempfile.NamedTemporaryFile("w", suffix=".json") as config_file:
            json.dump(model_config, config_file)
            config_file.flush()
            await environment.upload_file(
                Path(config_file.name),
                "/tmp/pi-agent-config/models.json",
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
        no_proxy = model_no_proxy(os.environ["TIMEM_BASE_URL"])
        env = {
            "TIMEM_API_KEY": self._get_env("TIMEM_API_KEY") or "",
            "PI_CODING_AGENT_DIR": "/tmp/pi-agent-config",
            "PI_CODING_AGENT_SESSION_DIR": "/tmp/pi-sessions",
            "PI_OFFLINE": "1",
            "PI_SKIP_VERSION_CHECK": "1",
            "PI_TELEMETRY": "0",
            "NO_PROXY": no_proxy,
            "no_proxy": no_proxy,
        }
        command = (
            f"{self._NODE} {self._PI_CLI} "
            "--print --mode json --no-session --approve --offline "
            "--no-context-files --no-skills --no-extensions "
            "--no-prompt-templates --no-themes "
            f"--provider timem --model {shlex.quote(os.environ['TIMEM_MODEL'])} "
            f"{shlex.quote(instruction)} | tee {shlex.quote(output_path)}"
        )
        await exec_agent_with_cleanup(self, environment, command=command, env=env)

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        path = self.logs_dir / self._LOG
        if not path.is_file():
            return
        input_tokens = output_tokens = cache_tokens = 0
        for line in path.read_text(errors="replace").splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            usage = event.get("usage") or (event.get("message") or {}).get("usage") or {}
            input_tokens += usage.get("input", usage.get("inputTokens", 0)) or 0
            output_tokens += usage.get("output", usage.get("outputTokens", 0)) or 0
            cache_tokens += usage.get("cacheRead", usage.get("cacheReadTokens", 0)) or 0
        context.n_input_tokens = input_tokens
        context.n_output_tokens = output_tokens
        context.n_cache_tokens = cache_tokens
