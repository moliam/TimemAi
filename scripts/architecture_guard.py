#!/usr/bin/env python3
"""Enforce Timem's semantic project layout and dependency direction."""

from __future__ import annotations

import argparse
import tempfile
from pathlib import Path

REQUIRED = (
    "Cargo.toml",
    "docs/semantic-project-layout.md",
    "agent_core/Cargo.toml",
    "agent_core/src/lib.rs",
    "core/platform/Cargo.toml",
    "core/platform/module_boundary.md",
    "core/platform/src/lib.rs",
    "core/platform/src/api.rs",
    "core/platform/src/shared.rs",
    "core/platform/src/macos.rs",
    "core/platform/src/linux.rs",
    "core/platform/tests/unit/platform_tests.rs",
    "core/ui_contract/Cargo.toml",
    "core/ui_contract/module_boundary.md",
    "core/ui_contract/src/lib.rs",
    "core/ui_contract/src/projections/mod.rs",
    "core/ui_contract/tests/projection_contract_tests.rs",
    "interfaces/shell/Cargo.toml",
    "interfaces/shell/module_boundary.md",
    "interfaces/web/package.json",
    "interfaces/web/module_boundary.md",
)
FORBIDDEN_DIRS = ("timem_shell", "web_ui", "agent_core/src/os", "core/platform/src/windows")
PROCESS_PRIMITIVES = ("libc::getpgid", "libc::getpgrp", "libc::waitpid", ".process_group(0)")
TARGET_DIRECTORIES = (
    "core/agent",
    "core/application",
    "core/ui_contract",
    "bridges/in_process",
    "bridges/http_websocket",
    "bridges/ipc",
    "interfaces/macos",
    "interfaces/windows",
    "interfaces/linux",
)
ARCHITECTURE_CONTRACT_MARKERS = (
    "## Target physical layout",
    "core/",
    "  agent/",
    "  application/",
    "  ui_contract/",
    "bridges/",
    "  in_process/",
    "  http_websocket/",
    "  ipc/",
    "interfaces/",
    "  shell/",
    "  web/",
    "`agent_core/` | `core/agent/`, `core/application/`, `core/ui_contract/`",
    "`host_projection/` | `bridges/http_websocket/`",
    "`timem_web/` | `bridges/http_websocket/`",
)


def text(root: Path, relative: str) -> str:
    path = root / relative
    return path.read_text(errors="replace") if path.is_file() else ""


def violations(root: Path) -> list[str]:
    errors: list[str] = []
    for relative in REQUIRED:
        if not (root / relative).is_file():
            errors.append(f"missing required architecture file: {relative}")
    for relative in FORBIDDEN_DIRS:
        if (root / relative).exists():
            errors.append(f"legacy or unsupported architecture path exists: {relative}")

    architecture_contract = text(root, "docs/semantic-project-layout.md")
    for marker in ARCHITECTURE_CONTRACT_MARKERS:
        if marker not in architecture_contract:
            errors.append(f"architecture contract is missing target or migration marker: {marker}")

    for relative in TARGET_DIRECTORIES:
        directory = root / relative
        if directory.is_dir() and not any(path.is_file() for path in directory.rglob("*")):
            errors.append(f"target architecture directory must not be an empty placeholder: {relative}")

    workspace = text(root, "Cargo.toml")
    for member in ('"core/platform"', '"core/ui_contract"', '"interfaces/shell"'):
        if member not in workspace:
            errors.append(f"workspace must include {member}")
    for legacy in ('"timem_shell"', '"web_ui/timem-web"'):
        if legacy in workspace:
            errors.append(f"workspace uses legacy member {legacy}")

    agent_manifest = text(root, "agent_core/Cargo.toml")
    if 'timem_platform = { path = "../core/platform" }' not in agent_manifest:
        errors.append("agent_core must depend on core/platform through timem_platform")
    if 'timem_ui_contract = { path = "../core/ui_contract" }' not in agent_manifest:
        errors.append("agent_core must depend inward on core/ui_contract")
    agent_lib = text(root, "agent_core/src/lib.rs")
    if "pub use timem_platform as os;" not in agent_lib:
        errors.append("agent_core must preserve its public os facade through timem_platform")
    if "pub mod os;" in agent_lib:
        errors.append("agent_core must not restore its legacy embedded os module")

    shell_manifest = text(root, "interfaces/shell/Cargo.toml")
    if 'name = "timem_shell"' not in shell_manifest:
        errors.append("interfaces/shell must preserve the timem_shell package name")
    if 'agent_core = { path = "../../agent_core" }' not in shell_manifest:
        errors.append("interfaces/shell must depend inward on agent_core")

    platform_manifest = text(root, "core/platform/Cargo.toml")
    if 'name = "timem_platform"' not in platform_manifest:
        errors.append("core/platform must expose the timem_platform crate")
    for forbidden in ("agent_core", "timem_shell", "timem_web", "host_projection", "interfaces/"):
        if forbidden in platform_manifest:
            errors.append(f"core/platform must not depend outward on {forbidden}")

    ui_contract_manifest = text(root, "core/ui_contract/Cargo.toml")
    if 'name = "timem_ui_contract"' not in ui_contract_manifest:
        errors.append("core/ui_contract must expose the timem_ui_contract crate")
    for forbidden in (
        "agent_core",
        "timem_platform",
        "timem_shell",
        "timem_web",
        "host_projection",
        "core/application",
        "bridges/",
        "interfaces/",
    ):
        if forbidden in ui_contract_manifest:
            errors.append(f"core/ui_contract must not depend outward on {forbidden}")

    platform_lib = text(root, "core/platform/src/lib.rs")
    required_cfg_modules = (
        '#[cfg(target_os = "linux")]\nmod linux;',
        '#[cfg(target_os = "macos")]\nmod macos;',
        '#[cfg(unix)]\nmod shared;',
    )
    for declaration in required_cfg_modules:
        if declaration not in platform_lib:
            errors.append(f"platform target selection missing: {declaration.splitlines()[-1]}")

    for path in (root / "agent_core/src").rglob("*.rs") if (root / "agent_core/src").is_dir() else ():
        source = path.read_text(errors="replace")
        for primitive in PROCESS_PRIMITIVES:
            if primitive in source:
                errors.append(
                    f"Core process primitive escaped core/platform/src/shared.rs: "
                    f"{path.relative_to(root)} contains {primitive}"
                )

    return errors


def write_fixture(root: Path) -> None:
    files = {
        "Cargo.toml": '[workspace]\nmembers = ["agent_core", "core/platform", "core/ui_contract", "interfaces/shell"]\n',
        "docs/semantic-project-layout.md": "\n".join(ARCHITECTURE_CONTRACT_MARKERS),
        "agent_core/Cargo.toml": '[dependencies]\ntimem_platform = { path = "../core/platform" }\ntimem_ui_contract = { path = "../core/ui_contract" }\n',
        "agent_core/src/lib.rs": "pub use timem_platform as os;\n",
        "core/platform/Cargo.toml": '[package]\nname = "timem_platform"\n',
        "core/platform/module_boundary.md": "platform boundary\n",
        "core/platform/src/lib.rs": 'mod api;\n#[cfg(target_os = "linux")]\nmod linux;\n#[cfg(target_os = "macos")]\nmod macos;\n#[cfg(unix)]\nmod shared;\n',
        "core/platform/src/api.rs": "pub fn api() {}\n",
        "core/platform/src/shared.rs": "pub fn shared() {}\n",
        "core/platform/src/macos.rs": "pub fn macos() {}\n",
        "core/platform/src/linux.rs": "pub fn linux() {}\n",
        "core/platform/tests/unit/platform_tests.rs": "#[test] fn platform() {}\n",
        "core/ui_contract/Cargo.toml": '[package]\nname = "timem_ui_contract"\n',
        "core/ui_contract/module_boundary.md": "ui contract boundary\n",
        "core/ui_contract/src/lib.rs": "pub mod projections;\n",
        "core/ui_contract/src/projections/mod.rs": "pub struct TurnProjection;\n",
        "core/ui_contract/tests/projection_contract_tests.rs": "#[test] fn projection() {}\n",
        "interfaces/shell/Cargo.toml": '[package]\nname = "timem_shell"\n[dependencies]\nagent_core = { path = "../../agent_core" }\n',
        "interfaces/shell/module_boundary.md": "shell boundary\n",
        "interfaces/web/package.json": "{}\n",
        "interfaces/web/module_boundary.md": "web boundary\n",
    }
    for relative, content in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)


def self_test() -> None:
    cases = (
        ("legacy directory", lambda root: (root / "timem_shell").mkdir()),
        ("reverse dependency", lambda root: (root / "core/platform/Cargo.toml").write_text('[package]\nname = "timem_platform"\n[dependencies]\ntimem_shell = { path = "../../interfaces/shell" }\n')),
        ("UI contract reverse dependency", lambda root: (root / "core/ui_contract/Cargo.toml").write_text('[package]\nname = "timem_ui_contract"\n[dependencies]\nagent_core = { path = "../../agent_core" }\n')),
        ("escaped process primitive", lambda root: (root / "agent_core/src/leak.rs").write_text("fn leak() { libc::waitpid(0, std::ptr::null_mut(), 0); }\n")),
        ("empty target placeholder", lambda root: (root / "bridges/ipc").mkdir(parents=True)),
        ("target contract drift", lambda root: (root / "docs/semantic-project-layout.md").write_text("incomplete target\n")),
    )
    for label, mutate in cases:
        with tempfile.TemporaryDirectory(prefix="timem-architecture-guard-") as directory:
            root = Path(directory)
            write_fixture(root)
            baseline = violations(root)
            if baseline:
                raise SystemExit(f"self-test fixture invalid for {label}: {baseline}")
            mutate(root)
            if not violations(root):
                raise SystemExit(f"self-test failed to reject {label}")
    print("architecture_guard self-test: ok")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    errors = violations(args.root.resolve())
    if errors:
        raise SystemExit("\n".join(errors))
    print("architecture_guard: ok")


if __name__ == "__main__":
    main()
