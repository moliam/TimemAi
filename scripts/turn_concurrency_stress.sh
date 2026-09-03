#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

ITERATIONS="${TIMEM_TURN_STRESS_ITERATIONS:-300}"
SEED="${TIMEM_TURN_STRESS_SEED:-6840335614483443998}"
case "$ITERATIONS" in
  ''|*[!0-9]*) echo "error: TIMEM_TURN_STRESS_ITERATIONS must be a positive integer" >&2; exit 2 ;;
esac
case "$SEED" in
  ''|*[!0-9]*) echo "error: TIMEM_TURN_STRESS_SEED must be an unsigned integer" >&2; exit 2 ;;
esac
if [ "$ITERATIONS" -lt 1 ]; then
  echo "error: TIMEM_TURN_STRESS_ITERATIONS must be >= 1" >&2
  exit 2
fi

export TIMEM_TURN_STRESS_ITERATIONS="$ITERATIONS"
export TIMEM_TURN_STRESS_SEED="$SEED"
echo "== Turn concurrency stress: PromptCut / terminal ownership (Core/Worker) =="
echo "seed=$SEED iterations=$ITERATIONS"
cargo test -p timem_session --lib --locked \
  tests::prompt_cut_terminal_ownership_stress_is_seeded_and_bounded -- \
  --exact --ignored --nocapture --test-threads=1

echo "turn_concurrency_stress: implemented=prompt_cut_core_worker pending=host_attachment_fifo,stop_start,websocket_fifo,chrome_latency"
