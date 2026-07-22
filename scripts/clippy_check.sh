#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

cargo clippy --workspace --all-targets -- \
  -D warnings \
  -A clippy::too_many_arguments \
  -A clippy::type_complexity \
  -A clippy::large_enum_variant \
  -A clippy::result_large_err
