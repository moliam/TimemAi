"""SWE-agent v1.1.0 package initialization with no-git compatibility.

This is the upstream module with its two commit-hash probes widened so the
documented ``unavailable`` fallback also covers a missing git executable.
"""

from __future__ import annotations

import os
import sys
from functools import partial
from logging import WARNING, getLogger
from pathlib import Path

import swerex.utils.log as log_swerex
from git import Repo
from packaging import version

from sweagent.utils.log import get_logger

__version__ = "1.1.0"
PYTHON_MINIMUM_VERSION = (3, 11)
SWEREX_MINIMUM_VERSION = "1.2.0"
SWEREX_RECOMMENDED_VERSION = "1.2.1"

log_swerex.get_logger = partial(get_logger, emoji="🦖")
getLogger("datasets").setLevel(WARNING)
getLogger("numexpr.utils").setLevel(WARNING)
getLogger("LiteLLM").setLevel(WARNING)

PACKAGE_DIR = Path(__file__).resolve().parent
if sys.version_info < PYTHON_MINIMUM_VERSION:
    raise RuntimeError("SWE-agent requires Python 3.11 or higher.")

assert PACKAGE_DIR.is_dir(), PACKAGE_DIR
REPO_ROOT = PACKAGE_DIR.parent
assert REPO_ROOT.is_dir(), REPO_ROOT
CONFIG_DIR = Path(os.getenv("SWE_AGENT_CONFIG_DIR", REPO_ROOT / "config"))
assert CONFIG_DIR.is_dir(), CONFIG_DIR
TOOLS_DIR = Path(os.getenv("SWE_AGENT_TOOLS_DIR", REPO_ROOT / "tools"))
assert TOOLS_DIR.is_dir(), TOOLS_DIR
TRAJECTORY_DIR = Path(os.getenv("SWE_AGENT_TRAJECTORY_DIR", REPO_ROOT / "trajectories"))
assert TRAJECTORY_DIR.is_dir(), TRAJECTORY_DIR


def _repo_hash(path: Path) -> str:
    try:
        return Repo(path, search_parent_directories=False).head.object.hexsha
    except Exception:
        return "unavailable"


def get_agent_commit_hash() -> str:
    return _repo_hash(REPO_ROOT)


def get_rex_commit_hash() -> str:
    import swerex

    return _repo_hash(Path(swerex.__file__).resolve().parent.parent.parent)


def get_rex_version() -> str:
    from swerex import __version__ as rex_version

    return rex_version


def get_agent_version_info() -> str:
    return (
        f"This is SWE-agent version {__version__} (hash={get_agent_commit_hash()}) "
        f"with SWE-ReX version {get_rex_version()} (hash={get_rex_commit_hash()})."
    )


def impose_rex_lower_bound() -> None:
    rex_version = get_rex_version()
    if version.parse(rex_version) < version.parse(SWEREX_MINIMUM_VERSION):
        raise RuntimeError(
            "SWE-ReX is too old; install at least " + SWEREX_MINIMUM_VERSION
        )
    if version.parse(rex_version) < version.parse(SWEREX_RECOMMENDED_VERSION):
        get_logger("swe-agent", emoji="👋").warning(
            "SWE-ReX %s is below the recommended version %s.",
            rex_version,
            SWEREX_RECOMMENDED_VERSION,
        )


impose_rex_lower_bound()
get_logger("swe-agent", emoji="👋").info(get_agent_version_info())

__all__ = [
    "PACKAGE_DIR",
    "CONFIG_DIR",
    "get_agent_commit_hash",
    "get_agent_version_info",
    "__version__",
]
