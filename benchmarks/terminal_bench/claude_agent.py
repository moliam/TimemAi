"""Harbor installed-agent adapter for EPT-launched Claude Code."""

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


class ClaudeAgent(BaseInstalledAgent):
    """Run the server's EPT Claude profile inside the task container."""

    _EPT = "/usr/local/bin/ept"
    _CLAUDE = "/usr/local/bin/claude"
    _HOME = "/tmp/ept-claude-home"
    _LOG = "claude.json"

    @staticmethod
    @override
    def name() -> str:
        return "claude-code-ept"

    @override
    def get_version_command(self) -> str | None:
        return f"{self._CLAUDE} --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await install_verifier_assets(self, environment)
        assets = {
            self._EPT: Path(os.environ["CLAUDE_BENCH_EPT_BINARY"]),
            self._CLAUDE: Path(os.environ["CLAUDE_BENCH_BINARY"]),
            "/etc/ssl/certs/ca-certificates.crt": Path(
                os.environ["CLAUDE_BENCH_CA_BUNDLE"]
            ),
            f"{self._HOME}/.config/ept/auth_session.json": Path(
                os.environ["CLAUDE_BENCH_EPT_AUTH"]
            ),
            f"{self._HOME}/.config/ept/config.yaml": Path(
                os.environ["CLAUDE_BENCH_EPT_CONFIG"]
            ),
        }
        if any(not path.is_file() for path in assets.values()):
            raise FileNotFoundError("Claude/EPT binary or configuration is missing")
        await self.exec_as_root(
            environment,
            command=(
                "id -u claudeagent >/dev/null 2>&1 || "
                f"useradd -M -d {self._HOME} -s /bin/bash claudeagent; "
                f"mkdir -p {self._HOME}/.config/ept {self._HOME}/.ept/bin "
                "/etc/ssl/certs"
            ),
        )
        for remote_path, local_path in assets.items():
            await environment.upload_file(local_path, remote_path)
        await self.exec_as_root(
            environment,
            command=(
                f"chmod 755 {self._EPT} {self._CLAUDE} && "
                f"ln -sf {self._EPT} {self._HOME}/.ept/bin/ept && "
                f"chown -R claudeagent:claudeagent {self._HOME} /app /logs/agent"
            ),
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
            "HOME": self._HOME,
            "PATH": "/usr/local/bin:/usr/bin:/bin",
            "DISABLE_AUTOUPDATER": "1",
            "DISABLE_TELEMETRY": "1",
            "DISABLE_ERROR_REPORTING": "1",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
            "SSL_CERT_FILE": "/etc/ssl/certs/ca-certificates.crt",
        }
        command = (
            "set -o pipefail; "
            f"timeout --signal=TERM --kill-after=10s 1800s {self._EPT} "
            "claude --bare --print --output-format json "
            "--no-session-persistence --dangerously-skip-permissions "
            "--model andes-glm-5.2 "
            f"{shlex.quote(instruction)} > {shlex.quote(output_path)}; "
            "agent_status=$?; "
            "if [ \"$agent_status\" -eq 124 ] || [ \"$agent_status\" -eq 137 ]; then "
            "pkill -TERM -f '^/usr/local/bin/claude ' 2>/dev/null || true; "
            "pkill -TERM -f '^/usr/local/bin/ept daemon serve' 2>/dev/null || true; "
            "sleep 2; "
            "pkill -KILL -f '^/usr/local/bin/claude ' 2>/dev/null || true; "
            "pkill -KILL -f '^/usr/local/bin/ept daemon serve' 2>/dev/null || true; "
            "fi; "
            f"cat {shlex.quote(output_path)}; exit \"$agent_status\""
        )
        await self._exec(
            environment,
            command=command,
            user="claudeagent",
            env=env,
            cwd="/app",
        )

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        path = self.logs_dir / self._LOG
        if not path.is_file():
            return
        try:
            payload = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            return
        usage = payload.get("usage") or {}
        context.n_input_tokens = usage.get("input_tokens", 0)
        context.n_cache_tokens = usage.get("cache_read_input_tokens", 0)
        context.n_output_tokens = usage.get("output_tokens", 0)
