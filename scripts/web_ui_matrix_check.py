#!/usr/bin/env python3
"""Validate that the Web UI feature matrix points at real evidence."""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
MATRIX = ROOT / "docs" / "web-ui-feature-test-matrix.md"
TEST_ROOTS = [
    ROOT / "timem_web" / "tests",
    ROOT / "web_ui" / "timem-web" / "tests",
]
CI_SCRIPT = ROOT / "scripts" / "ci.sh"
MANUAL_SMOKE = ROOT / "docs" / "manual-release-smoke.md"


def read(path: pathlib.Path) -> str:
    return path.read_text(encoding="utf-8", errors="ignore")


def iter_files(root: pathlib.Path):
    if not root.exists():
        return
    for path in root.rglob("*"):
        if path.is_file() and path.suffix in {".rs", ".ts", ".tsx", ".sh", ".md"}:
            yield path


def search_roots(needle: str, roots: list[pathlib.Path]) -> bool:
    for root in roots:
        for path in iter_files(root):
            if needle in read(path):
                return True
    return False


def file_exists_token(token: str) -> bool:
    if "/" in token:
        return (ROOT / token).exists()
    if token.endswith((".ts", ".tsx", ".rs", ".sh", ".md")):
        for base in [
            ROOT,
            ROOT / "web_ui" / "timem-web" / "tests",
            ROOT / "timem_web" / "tests",
            ROOT / "scripts",
            ROOT / "docs",
        ]:
            if (base / token).exists():
                return True
    return False


def command_is_ci_evidence(token: str) -> bool:
    if " " not in token:
        return False
    return token in read(CI_SCRIPT)


def evidence_in_roots(token: str, roots: list[pathlib.Path]) -> bool:
    if token in {"SessionN"}:
        return search_roots(token, roots)
    if token == "self_tool chg_cwd":
        return search_roots("chg_cwd", roots)
    if token.endswith(" tests"):
        return search_roots(token.removesuffix(" tests"), roots)
    if token == "host_error handling in frontend contract tests":
        return search_roots("host_error", roots)
    if "/" not in token and token.endswith((".ts", ".tsx", ".rs", ".sh", ".md")):
        for root in roots:
            for path in iter_files(root):
                if path.name == token:
                    return True
    return search_roots(token, roots)


def evidence_exists(token: str) -> bool:
    if token in {"SessionN"}:
        return search_roots(token, TEST_ROOTS)
    if token == "self_tool chg_cwd":
        return search_roots("chg_cwd", TEST_ROOTS)
    if token.endswith(" tests"):
        return search_roots(token.removesuffix(" tests"), TEST_ROOTS)
    if token == "host_error handling in frontend contract tests":
        return search_roots("host_error", TEST_ROOTS)
    if token == "browser smoke rows in docs/manual-release-smoke.md":
        return MANUAL_SMOKE.exists() and "Browser Matrix" in read(MANUAL_SMOKE)
    if file_exists_token(token):
        return True
    if command_is_ci_evidence(token):
        return True
    return search_roots(token, TEST_ROOTS)


HOST_RUNTIME_ROWS = {
    "Authenticated local host",
    "Session creation and naming",
    "Per-session runtime profile",
    "Multi-session topic isolation",
    "Worker hierarchy and state",
    "Stop/cancel under human pressure",
    "Send during active work",
    "Stale supplement recovery",
    "Attachments",
    "Inline decisions",
    "Work instructions",
    "Current cwd display",
    "Final answer rendering",
    "Usage and context status",
    "History and resume",
    "Mem switching",
    "Scroll and bounded rendering",
}


VAGUE_EVIDENCE_TOKENS = {
    "composerSendDecision",
    "turnLiveUsage",
    "sessionContextUsage",
    "tailPath",
    "self_tool chg_cwd",
}


def is_vague_evidence(token: str) -> bool:
    return token in VAGUE_EVIDENCE_TOKENS or token.endswith(" tests")


def main() -> int:
    if not MATRIX.exists():
        print(f"missing Web UI feature matrix: {MATRIX.relative_to(ROOT)}", file=sys.stderr)
        return 1
    failures: list[str] = []
    rows = [
        line
        for line in read(MATRIX).splitlines()
        if line.startswith("| ") and not line.startswith("| Requirement ") and not line.startswith("|---")
    ]
    if not rows:
        print("Web UI feature matrix has no requirement rows", file=sys.stderr)
        return 1
    for line in rows:
        cells = [cell.strip() for cell in line.strip("|").split("|")]
        if len(cells) != 3:
            failures.append(f"malformed matrix row: {line}")
            continue
        requirement, _behavior, evidence = cells
        tokens = re.findall(r"`([^`]+)`", evidence)
        if not tokens:
            failures.append(f"{requirement}: missing backticked test evidence")
            continue
        vague = [token for token in tokens if is_vague_evidence(token)]
        if vague:
            failures.append(f"{requirement}: vague evidence token must name a concrete test or file: {', '.join(vague)}")
        missing = [token for token in tokens if not evidence_exists(token)]
        if missing:
            failures.append(f"{requirement}: evidence not found: {', '.join(missing)}")
        if requirement in HOST_RUNTIME_ROWS:
            has_host = any(evidence_in_roots(token, [ROOT / "timem_web" / "tests"]) for token in tokens)
            has_frontend = any(evidence_in_roots(token, [ROOT / "web_ui" / "timem-web" / "tests"]) for token in tokens)
            if not has_host or not has_frontend:
                missing_sides = []
                if not has_host:
                    missing_sides.append("timem_web host/runtime test evidence")
                if not has_frontend:
                    missing_sides.append("web_ui frontend test evidence")
                failures.append(f"{requirement}: missing cross-boundary evidence: {', '.join(missing_sides)}")
    if failures:
        print("web_ui_matrix_check failed:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print(f"web_ui_matrix_check: ok ({len(rows)} requirement rows)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
