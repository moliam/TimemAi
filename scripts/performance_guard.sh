#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export TIMEM_PERF_GUARD=1

assert_exact_performance_tests() {
  local package="$1"
  shift
  local listed
  listed="$(cargo test --release -p "$package" -- --list 2>/dev/null | awk '/performance_guard.*: test$/ { sub(/: test$/, ""); print }' | sort)"
  local expected
  expected="$(printf '%s\n' "$@" | sort)"
  if [[ "$listed" != "$expected" ]]; then
    echo "error: $package performance test discovery drift" >&2
    diff -u <(printf '%s\n' "$expected") <(printf '%s\n' "$listed") >&2 || true
    exit 1
  fi
  echo "performance_guard: $package discovered $# expected tests"
}

assert_exact_performance_tests agent_core \
  performance_guard_large_context_prompt_render_is_bounded \
  performance_guard_many_overlay_capabilities_render_is_bounded \
  performance_guard_topic_generation_for_many_actions_is_bounded

echo "performance_guard: agent_core"
cargo test --release -p agent_core performance_guard --quiet

assert_exact_performance_tests timem_shell \
  observation::tests::performance_guard_many_observation_events_render_bounded \
  observation::tests::performance_guard_topic_interface_rate_mix_render_bounded

echo "performance_guard: timem_shell"
cargo test --release -p timem_shell performance_guard --quiet

echo "performance_guard: timem browser hot paths"
pnpm --dir interfaces/web exec vitest run tests/performance_guard.test.ts

echo "performance_guard: ok"
