#!/usr/bin/env python3
"""Enforce Timem's semantic project layout and dependency direction."""

from __future__ import annotations

import argparse
import tempfile
from pathlib import Path

REQUIRED = (
    "Cargo.toml",
    "docs/semantic-project-layout.md",
    "core/agent/Cargo.toml",
    "core/agent/src/lib.rs",
    "core/session/Cargo.toml",
    "core/session/module_boundary.md",
    "core/session/src/lib.rs",
    "core/session/tests/unit/session_worker_tests.rs",
    "core/platform/Cargo.toml",
    "core/platform/module_boundary.md",
    "core/platform/src/lib.rs",
    "core/platform/src/api.rs",
    "core/platform/src/shared.rs",
    "core/platform/src/macos.rs",
    "core/platform/src/linux.rs",
    "core/platform/src/windows/command.rs",
    "core/platform/src/windows/mod.rs",
    "core/platform/src/windows/process.rs",
    "core/platform/src/windows/system.rs",
    "docs/windows-support-matrix.md",
    "core/platform/tests/unit/platform_tests.rs",
    "core/ui_contract/Cargo.toml",
    "core/ui_contract/module_boundary.md",
    "core/ui_contract/src/lib.rs",
    "core/ui_contract/src/commands/mod.rs",
    "core/ui_contract/src/projections/mod.rs",
    "core/ui_contract/tests/command_contract_tests.rs",
    "core/ui_contract/tests/projection_contract_tests.rs",
    "bridges/in_process/Cargo.toml",
    "bridges/in_process/module_boundary.md",
    "bridges/in_process/src/lib.rs",
    "bridges/in_process/tests/turn_bridge_tests.rs",
    "bridges/http_websocket/Cargo.toml",
    "bridges/http_websocket/module_boundary.md",
    "bridges/http_websocket/src/lib.rs",
    "interfaces/shell/Cargo.toml",
    "interfaces/shell/module_boundary.md",
    "interfaces/shell/src/os/mod.rs",
    "interfaces/shell/src/os/unix.rs",
    "interfaces/shell/src/os/windows/mod.rs",
    "interfaces/shell/src/os/windows/console.rs",
    "interfaces/web/package.json",
    "interfaces/web/module_boundary.md",
    "applications/timem/Cargo.toml",
    "applications/timem/module_boundary.md",
    "applications/timem/src/lib.rs",
    "applications/timem/src/main.rs",
    "applications/timem/src/os/mod.rs",
    "applications/timem/src/os/unix.rs",
    "applications/timem/src/os/windows/mod.rs",
    "applications/timem/src/os/windows/lifecycle.rs",
)
FORBIDDEN_DIRS = (
    "agent_core",
    "timem_shell",
    "web_ui",
    "core/application",
    "core/agent/src/os",
    "host_projection",
    "timem_web",
)
PROCESS_PRIMITIVES = ("libc::getpgid", "libc::getpgrp", "libc::waitpid", ".process_group(0)")
TARGET_DIRECTORIES = (
    "core/agent",
    "core/session",
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
    "  session/",
    "  ui_contract/",
    "bridges/",
    "  in_process/",
    "  http_websocket/",
    "  ipc/",
    "interfaces/",
    "  shell/",
    "  web/",
    "applications/",
    "  timem/",
    "Migration complete",
    "all same-process Rust Interfaces",
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
    for member in (
        '"bridges/in_process"',
        '"bridges/http_websocket"',
        '"core/platform"',
        '"core/session"',
        '"core/ui_contract"',
        '"interfaces/shell"',
        '"applications/timem"',
    ):
        if member not in workspace:
            errors.append(f"workspace must include {member}")
    for legacy in ('"timem_shell"', '"web_ui/timem-web"'):
        if legacy in workspace:
            errors.append(f"workspace uses legacy member {legacy}")

    agent_manifest = text(root, "core/agent/Cargo.toml")
    if 'timem_platform = { path = "../platform" }' not in agent_manifest:
        errors.append("agent_core must depend on core/platform through timem_platform")
    if 'timem_ui_contract = { path = "../ui_contract" }' not in agent_manifest:
        errors.append("agent_core must depend inward on core/ui_contract")
    for forbidden in ("timem_session", "timem_in_process", "../session", "../../bridges/"):
        if forbidden in agent_manifest:
            errors.append(f"agent_core must not depend outward on {forbidden}")
    agent_lib = text(root, "core/agent/src/lib.rs")
    if "pub use timem_platform as os;" not in agent_lib:
        errors.append("agent_core must preserve its public os facade through timem_platform")
    if "pub mod os;" in agent_lib:
        errors.append("agent_core must not restore its legacy embedded os module")

    session_manifest = text(root, "core/session/Cargo.toml")
    if 'name = "timem_session"' not in session_manifest:
        errors.append("core/session must expose the timem_session crate")
    if 'agent_core = { path = "../agent" }' not in session_manifest:
        errors.append("core/session must depend inward on agent_core")
    if 'timem_ui_contract = { path = "../ui_contract" }' not in session_manifest:
        errors.append("core/session must depend inward on core/ui_contract")
    for forbidden in ("timem_shell", "timem_web", "host_projection", "bridges/", "interfaces/"):
        if forbidden in session_manifest:
            errors.append(f"core/session must not depend outward on {forbidden}")
    for forbidden in ("reedline", "crossterm", "termimad", "axum", "tungstenite", "websocket"):
        if forbidden in session_manifest:
            errors.append(f"core/session must not absorb Interface or transport dependency: {forbidden}")

    in_process_manifest = text(root, "bridges/in_process/Cargo.toml")
    if 'name = "timem_in_process"' not in in_process_manifest:
        errors.append("bridges/in_process must expose the timem_in_process crate")
    if 'timem_session = { path = "../../core/session" }' not in in_process_manifest:
        errors.append("bridges/in_process must depend inward on core/session")
    if 'timem_ui_contract = { path = "../../core/ui_contract" }' not in in_process_manifest:
        errors.append("bridges/in_process must depend inward on core/ui_contract")
    for forbidden in ("agent_core", "timem_shell", "timem_web", "host_projection", "interfaces/"):
        if forbidden in in_process_manifest:
            errors.append(f"bridges/in_process must not depend outward on {forbidden}")
    for forbidden in ("serde", "serde_json", "tokio", "reqwest", "axum", "tungstenite", "websocket"):
        if forbidden in in_process_manifest:
            errors.append(
                f"in-process Bridge must not add serialization, networking, or async runtime dependency: {forbidden}"
            )
    in_process_source = text(root, "bridges/in_process/src/lib.rs")
    for forbidden in ("serde::", "serde_json::", "tokio::", "reqwest::", "axum::", "tungstenite", "TcpStream", "UdpSocket"):
        if forbidden in in_process_source:
            errors.append(f"in-process Bridge must remain direct-call and zero-transport: {forbidden}")

    http_manifest = text(root, "bridges/http_websocket/Cargo.toml")
    if 'name = "timem_http_websocket"' not in http_manifest:
        errors.append("bridges/http_websocket must expose the timem_http_websocket crate")
    if 'timem_ui_contract = { path = "../../core/ui_contract" }' not in http_manifest:
        errors.append("HTTP/WebSocket Bridge must depend inward on core/ui_contract")
    for forbidden in ("timem_shell", "timem_web", "host_projection", "interfaces/"):
        if forbidden in http_manifest:
            errors.append(f"HTTP/WebSocket Bridge must not depend outward on an Interface: {forbidden}")
    http_source = text(root, "bridges/http_websocket/src/lib.rs")
    if "timem_ui_contract::projections" not in http_source:
        errors.append("HTTP/WebSocket delivery must consume projection semantics from core/ui_contract")
    http_source_root = root / "bridges/http_websocket/src"
    http_sources = "\n".join(
        path.read_text(errors="replace") for path in http_source_root.rglob("*.rs")
    ) if http_source_root.is_dir() else ""
    for forbidden in ("timem_shell::", "timem_web::", "interfaces/"):
        if forbidden in http_sources:
            errors.append(
                f"HTTP/WebSocket Bridge source must not depend outward on an Interface/Application: {forbidden}"
            )

    interfaces_root = root / "interfaces"
    if interfaces_root.is_dir():
        for manifest_path in interfaces_root.glob("*/Cargo.toml"):
            manifest = manifest_path.read_text(errors="replace")
            for other in interfaces_root.iterdir():
                if other.is_dir() and other != manifest_path.parent:
                    relative_reference = f"../{other.name}"
                    if relative_reference in manifest:
                        errors.append(
                            f"Interface must not depend on another Interface: {manifest_path.relative_to(root)}"
                        )

    shell_manifest = text(root, "interfaces/shell/Cargo.toml")
    if 'name = "timem_shell"' not in shell_manifest:
        errors.append("interfaces/shell must preserve the timem_shell package name")
    for forbidden in ('agent_core', 'timem_session'):
        if forbidden in shell_manifest:
            errors.append(f"interfaces/shell must not bypass the in-process Bridge through {forbidden}")
    if 'timem_ui_contract = { path = "../../core/ui_contract" }' not in shell_manifest:
        errors.append("interfaces/shell must depend on core/ui_contract")
    if 'timem_in_process = { path = "../../bridges/in_process" }' not in shell_manifest:
        errors.append("interfaces/shell must use the in-process Bridge")
    shell_main = text(root, "interfaces/shell/src/app.rs")
    if "run_in_process_turn(" not in shell_main:
        errors.append("interfaces/shell must enter synchronous Turns through the in-process Bridge")
    if "run_session_turn(" in shell_main:
        errors.append("interfaces/shell must not bypass the in-process Bridge for synchronous Turns")
    for primitive in ("libc::", "/dev/tty", "termios", "tcsetattr", "fcntl(", "AsRawFd", "FromRawFd"):
        if primitive in shell_main:
            errors.append(
                f"Shell OS primitive escaped interfaces/shell/src/os: "
                f"interfaces/shell/src/app.rs contains {primitive}"
            )

    platform_manifest = text(root, "core/platform/Cargo.toml")
    if 'name = "timem_platform"' not in platform_manifest:
        errors.append("core/platform must expose the timem_platform crate")
    if '[target.\'cfg(windows)\'.dependencies]' not in platform_manifest or 'windows-sys' not in platform_manifest:
        errors.append("core/platform must declare its target-scoped Windows backend dependency")
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
        "core/session",
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
        '#[cfg(windows)]\nmod windows;',
    )
    for declaration in required_cfg_modules:
        if declaration not in platform_lib:
            errors.append(f"platform target selection missing: {declaration.splitlines()[-1]}")

    for path in (root / "core/agent/src").rglob("*.rs") if (root / "core/agent/src").is_dir() else ():
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
        "Cargo.toml": '[workspace]\nmembers = ["bridges/in_process", "bridges/http_websocket", "core/agent", "core/platform", "core/session", "core/ui_contract", "interfaces/shell", "applications/timem"]\n',
        "docs/semantic-project-layout.md": "\n".join(ARCHITECTURE_CONTRACT_MARKERS),
        "docs/windows-support-matrix.md": "platform implemented; upper layers not yet supported\n",
        "core/agent/Cargo.toml": '[dependencies]\ntimem_platform = { path = "../platform" }\ntimem_ui_contract = { path = "../ui_contract" }\n',
        "core/agent/src/lib.rs": "pub use timem_platform as os;\n",
        "core/session/Cargo.toml": '[package]\nname = "timem_session"\n[dependencies]\nagent_core = { path = "../agent" }\ntimem_ui_contract = { path = "../ui_contract" }\n',
        "core/session/module_boundary.md": "session boundary\n",
        "core/session/src/lib.rs": "pub struct CoreSessionWorker;\n",
        "core/session/tests/unit/session_worker_tests.rs": "#[test] fn worker() {}\n",
        "core/platform/Cargo.toml": '[package]\nname = "timem_platform"\n[target.\'cfg(windows)\'.dependencies]\nwindows-sys = "0.61"\n',
        "core/platform/module_boundary.md": "platform boundary\n",
        "core/platform/src/lib.rs": 'mod api;\n#[cfg(target_os = "linux")]\nmod linux;\n#[cfg(target_os = "macos")]\nmod macos;\n#[cfg(unix)]\nmod shared;\n#[cfg(windows)]\nmod windows;\n',
        "core/platform/src/api.rs": "pub fn api() {}\n",
        "core/platform/src/shared.rs": "pub fn shared() {}\n",
        "core/platform/src/macos.rs": "pub fn macos() {}\n",
        "core/platform/src/linux.rs": "pub fn linux() {}\n",
        "core/platform/src/windows/command.rs": "pub fn command() {}\n",
        "core/platform/src/windows/mod.rs": "mod command; mod process; mod system;\n",
        "core/platform/src/windows/process.rs": "pub fn process() {}\n",
        "core/platform/src/windows/system.rs": "pub fn system() {}\n",
        "core/platform/tests/unit/platform_tests.rs": "#[test] fn platform() {}\n",
        "core/ui_contract/Cargo.toml": '[package]\nname = "timem_ui_contract"\n',
        "core/ui_contract/module_boundary.md": "ui contract boundary\n",
        "core/ui_contract/src/lib.rs": "pub mod commands;\npub mod projections;\n",
        "core/ui_contract/src/commands/mod.rs": "pub struct ToolGenRequest;\n",
        "core/ui_contract/src/projections/mod.rs": "pub struct TurnProjection;\n",
        "core/ui_contract/tests/command_contract_tests.rs": "#[test] fn command() {}\n",
        "core/ui_contract/tests/projection_contract_tests.rs": "#[test] fn projection() {}\n",
        "bridges/in_process/Cargo.toml": '[package]\nname = "timem_in_process"\n[dependencies]\ntimem_session = { path = "../../core/session" }\ntimem_ui_contract = { path = "../../core/ui_contract" }\n',
        "bridges/in_process/module_boundary.md": "in-process boundary\n",
        "bridges/in_process/src/lib.rs": "pub fn run_turn() {}\n",
        "bridges/in_process/tests/turn_bridge_tests.rs": "#[test] fn bridge() {}\n",
        "bridges/http_websocket/Cargo.toml": '[package]\nname = "timem_http_websocket"\n[dependencies]\ntimem_ui_contract = { path = "../../core/ui_contract" }\n',
        "bridges/http_websocket/module_boundary.md": "http websocket boundary\n",
        "bridges/http_websocket/src/lib.rs": "use timem_ui_contract::projections::TurnProjection;\npub struct DeliveryRevision;\n",
        "interfaces/shell/Cargo.toml": '[package]\nname = "timem_shell"\n[dependencies]\ntimem_in_process = { path = "../../bridges/in_process" }\ntimem_ui_contract = { path = "../../core/ui_contract" }\n',
        "interfaces/shell/src/app.rs": "fn main() { run_in_process_turn(); }\n",
        "interfaces/shell/module_boundary.md": "shell boundary\n",
        "interfaces/shell/src/os/mod.rs": "#[cfg(unix)] mod unix; #[cfg(windows)] mod windows;\n",
        "interfaces/shell/src/os/unix.rs": "pub fn terminal() {}\n",
        "interfaces/shell/src/os/windows/mod.rs": "mod console;\n",
        "interfaces/shell/src/os/windows/console.rs": "pub fn terminal() {}\n",
        "interfaces/web/package.json": "{}\n",
        "interfaces/web/module_boundary.md": "web boundary\n",
        "applications/timem/Cargo.toml": '[package]\nname = "timem_web"\n',
        "applications/timem/module_boundary.md": "application boundary\n",
        "applications/timem/src/lib.rs": "pub fn run() {}\n",
        "applications/timem/src/main.rs": "fn main() {}\n",
        "applications/timem/src/os/mod.rs": "#[cfg(unix)] mod unix; #[cfg(windows)] mod windows;\n",
        "applications/timem/src/os/unix.rs": "pub fn lifecycle() {}\n",
        "applications/timem/src/os/windows/mod.rs": "mod lifecycle;\n",
        "applications/timem/src/os/windows/lifecycle.rs": "pub fn lifecycle() {}\n",
    }
    for relative, content in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="timem-architecture-extension-") as directory:
        root = Path(directory)
        write_fixture(root)
        extension = root / "interfaces/desktop"
        extension.mkdir(parents=True)
        (extension / "Cargo.toml").write_text(
            '[package]\nname = "timem_desktop"\n[dependencies]\n'
            'timem_in_process = { path = "../../bridges/in_process" }\n'
        )
        extension_errors = violations(root)
        if extension_errors:
            raise SystemExit(
                "self-test rejected a new Interface that depends only on the in-process Bridge: "
                f"{extension_errors}"
            )

    cases = (
        ("legacy directory", lambda root: (root / "timem_shell").mkdir()),
        (
            "legacy generic application directory",
            lambda root: (root / "core/application").mkdir(parents=True),
        ),
        ("Agent to Bridge reverse dependency", lambda root: (root / "core/agent/Cargo.toml").write_text('[dependencies]\ntimem_platform = { path = "../platform" }\ntimem_ui_contract = { path = "../ui_contract" }\ntimem_in_process = { path = "../../bridges/in_process" }\n')),
        ("Bridge to Interface reverse dependency", lambda root: (root / "bridges/in_process/Cargo.toml").write_text('[package]\nname = "timem_in_process"\n[dependencies]\ntimem_session = { path = "../../core/session" }\ntimem_ui_contract = { path = "../../core/ui_contract" }\ntimem_shell = { path = "../../interfaces/shell" }\n')),
        ("HTTP Bridge to Web Interface reverse dependency", lambda root: (root / "bridges/http_websocket/Cargo.toml").write_text('[package]\nname = "timem_http_websocket"\n[dependencies]\ntimem_ui_contract = { path = "../../core/ui_contract" }\ntimem_web = { path = "../../timem_web" }\n')),
        ("HTTP Bridge bypasses UI contract semantic owner", lambda root: ((root / "bridges/http_websocket/Cargo.toml").write_text('[package]\nname = "timem_http_websocket"\n[dependencies]\nagent_core = { path = "../../core/agent" }\n'), (root / "bridges/http_websocket/src/lib.rs").write_text("use agent_core::TurnProjection;\n"))),
        ("in-process Bridge adds serialization", lambda root: (root / "bridges/in_process/Cargo.toml").write_text('[package]\nname = "timem_in_process"\n[dependencies]\ntimem_session = { path = "../../core/session" }\ntimem_ui_contract = { path = "../../core/ui_contract" }\nserde_json = "1"\n')),
        ("in-process Bridge adds network transport", lambda root: (root / "bridges/in_process/src/lib.rs").write_text("fn run_turn() { let _transport: Option<TcpStream> = None; }\n")),
        ("Session absorbs terminal UI", lambda root: (root / "core/session/Cargo.toml").write_text('[package]\nname = "timem_session"\n[dependencies]\nagent_core = { path = "../agent" }\ntimem_ui_contract = { path = "../ui_contract" }\ncrossterm = "0.29"\n')),
        ("Interface depends on another Interface", lambda root: (root / "interfaces/shell/Cargo.toml").write_text('[package]\nname = "timem_shell"\n[dependencies]\ntimem_in_process = { path = "../../bridges/in_process" }\ntimem_ui_contract = { path = "../../core/ui_contract" }\ntimem_web_ui = { path = "../web" }\n')),
        ("Shell bypasses in-process Bridge", lambda root: (root / "interfaces/shell/src/app.rs").write_text("fn main() { run_session_turn(); }\n")),
        ("Shell Unix primitive outside OS adapter", lambda root: (root / "interfaces/shell/src/app.rs").write_text("fn main() { run_in_process_turn(); libc::fcntl(0, 0); }\n")),
        ("missing Windows Shell console adapter", lambda root: (root / "interfaces/shell/src/os/windows/console.rs").unlink()),
        ("missing Windows Web lifecycle adapter", lambda root: (root / "applications/timem/src/os/windows/lifecycle.rs").unlink()),
        ("Agent to Session reverse dependency", lambda root: (root / "core/agent/Cargo.toml").write_text('[dependencies]\ntimem_platform = { path = "../platform" }\ntimem_ui_contract = { path = "../ui_contract" }\ntimem_session = { path = "../session" }\n')),
        ("reverse dependency", lambda root: (root / "core/platform/Cargo.toml").write_text('[package]\nname = "timem_platform"\n[dependencies]\ntimem_shell = { path = "../../interfaces/shell" }\n')),
        ("UI contract reverse dependency", lambda root: (root / "core/ui_contract/Cargo.toml").write_text('[package]\nname = "timem_ui_contract"\n[dependencies]\nagent_core = { path = "../agent" }\n')),
        ("escaped process primitive", lambda root: (root / "core/agent/src/leak.rs").write_text("fn leak() { libc::waitpid(0, std::ptr::null_mut(), 0); }\n")),
        ("missing Windows Platform backend", lambda root: (root / "core/platform/src/windows/system.rs").unlink()),
        ("missing target-scoped Windows dependency", lambda root: (root / "core/platform/Cargo.toml").write_text('[package]\nname = "timem_platform"\n')),
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
