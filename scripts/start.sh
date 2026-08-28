#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export ORCHESTRATOR_MODEL="${ORCHESTRATOR_MODEL:-ollama/muse-glimmer:latest}"
export ORCHESTRATOR_INTERVAL_SECS="${ORCHESTRATOR_INTERVAL_SECS:-180}"
export ORCHESTRATOR_AUTO_MERGE="${ORCHESTRATOR_AUTO_MERGE:-0}"
export ORCHESTRATOR_FULL_VALIDATION="${ORCHESTRATOR_FULL_VALIDATION:-0}"

ORGANIZATION="${1:-Memorithm}"

REAL_OPENCODE="$(command -v opencode)"
if [[ -z "$REAL_OPENCODE" ]]; then
  printf 'ERROR: opencode is not installed or not on PATH\n' >&2
  exit 1
fi

REAL_CARGO="$(command -v cargo)"
if [[ -z "$REAL_CARGO" ]]; then
  printf 'ERROR: cargo is not installed or not on PATH\n' >&2
  exit 1
fi

# Put private runtime wrappers first in PATH. OpenCode is bridged through
# Ollama with runtime-enforced permissions. Cargo mirrors an explicitly
# pinned GitHub CI formatter toolchain when one is declared by the target
# repository. Generated executables live under target/ only.
WRAPPER_DIR="$ROOT/target/orchestrator-bin"
mkdir -p "$WRAPPER_DIR"
install -m 700 "$ROOT/scripts/opencode" "$WRAPPER_DIR/opencode"
install -m 700 "$ROOT/scripts/cargo" "$WRAPPER_DIR/cargo"

export ORCHESTRATOR_REAL_OPENCODE="$REAL_OPENCODE"
export ORCHESTRATOR_REAL_CARGO="$REAL_CARGO"
export ORCHESTRATOR_ORIGINAL_PATH="$PATH"
export PATH="$WRAPPER_DIR:$PATH"

printf '\n===== BUILD ORCHESTRATOR =====\n'
cargo build --release

printf '\n===== PREFLIGHT =====\n'
./target/release/orchestrator doctor

printf '\n===== START AUTONOMOUS LOOP =====\n'
printf 'organization=%s\n' "$ORGANIZATION"
printf 'model=%s\n' "$ORCHESTRATOR_MODEL"
printf 'interval=%ss\n' "$ORCHESTRATOR_INTERVAL_SECS"
printf 'auto_merge=%s\n' "$ORCHESTRATOR_AUTO_MERGE"
printf 'full_validation=%s\n' "$ORCHESTRATOR_FULL_VALIDATION"
printf 'opencode_bridge=ollama-launch+runtime-permissions\n'
printf 'cargo_bridge=ci-pinned-rustfmt\n\n'

exec ./target/release/orchestrator run "$ORGANIZATION"
