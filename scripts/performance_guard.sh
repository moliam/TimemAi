#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export TIMEM_PERF_GUARD=1

echo "performance_guard: agent_core"
cargo test -p agent_core performance_guard --quiet

echo "performance_guard: timem_shell"
cargo test -p timem_shell performance_guard --quiet

echo "performance_guard: ok"
