#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PARENT_HOME="${HOME:?HOME is not set}"
export ORCHESTRATOR_DATA_ROOT="${ORCHESTRATOR_DATA_ROOT:-$PARENT_HOME/.local/share/memorithm-orchestrator}"
export CARGO_HOME="${CARGO_HOME:-$ORCHESTRATOR_DATA_ROOT/cargo-home}"
export RUSTUP_HOME="${RUSTUP_HOME:-$PARENT_HOME/.rustup}"
mkdir -p "$ORCHESTRATOR_DATA_ROOT" "$CARGO_HOME"

# Orchestrator intentionally runs one local model only. Keep the primary,
# surgical, and follow-up repair paths on the same deterministic local agent
# so stale service environment cannot silently re-enable another model.
export ORCHESTRATOR_MODEL="ollama/qwen3.8:latest"
export ORCHESTRATOR_SURGICAL_MODEL="ollama/qwen3.8:latest"
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

REAL_GIT="$(command -v git)"
if [[ -z "$REAL_GIT" ]]; then
  printf 'ERROR: git is not installed or not on PATH\n' >&2
  exit 1
fi

REAL_GH="$(command -v gh)"
if [[ -z "$REAL_GH" ]]; then
  printf 'ERROR: gh is not installed or not on PATH\n' >&2
  exit 1
fi

REAL_BWRAP="$(command -v bwrap)"
if [[ -z "$REAL_BWRAP" ]]; then
  printf 'ERROR: bubblewrap (bwrap) is required for credential-isolated agent execution\n' >&2
  exit 1
fi

# Runtime wrappers used by Orchestrator itself. Git protects validated pushes
# from non-fast-forward races. gh stages the current PR base before validation
# so a stale branch cannot make Qwen repair code already fixed on the base.
WRAPPER_DIR="$ROOT/target/orchestrator-bin"
mkdir -p "$WRAPPER_DIR"
install -m 700 "$ROOT/scripts/opencode-env" "$WRAPPER_DIR/opencode"
install -m 700 "$ROOT/scripts/opencode" "$WRAPPER_DIR/opencode-core"
install -m 700 "$ROOT/scripts/cargo" "$WRAPPER_DIR/cargo"
install -m 700 "$ROOT/scripts/git" "$WRAPPER_DIR/git"
install -m 700 "$ROOT/scripts/gh" "$WRAPPER_DIR/gh"
install -m 700 "$ROOT/scripts/agent-sandbox" "$WRAPPER_DIR/agent-sandbox"

# The coding worker receives only explicitly managed command bridges. Cargo
# mirrors repository CI formatting policy; Git and GitHub are technically
# read-only even if an agent prompt is ignored or a plugin attempts mutation.
AGENT_WRAPPER_DIR="$ROOT/target/orchestrator-agent-bin"
AGENT_HOME="${ORCHESTRATOR_AGENT_HOME:-$ROOT/target/orchestrator-agent-home}"
AGENT_CONFIG_DIR="$AGENT_HOME/.config"
AGENT_DATA_DIR="$AGENT_HOME/.local/share"
AGENT_CACHE_DIR="$AGENT_HOME/.cache"
mkdir -p \
  "$AGENT_WRAPPER_DIR" \
  "$AGENT_HOME" \
  "$AGENT_CONFIG_DIR" \
  "$AGENT_DATA_DIR" \
  "$AGENT_CACHE_DIR"
install -m 700 "$ROOT/scripts/cargo" "$AGENT_WRAPPER_DIR/cargo"
install -m 700 "$ROOT/scripts/agent-git" "$AGENT_WRAPPER_DIR/git"
install -m 700 "$ROOT/scripts/agent-gh" "$AGENT_WRAPPER_DIR/gh"

export ORCHESTRATOR_REAL_OPENCODE="$REAL_OPENCODE"
export ORCHESTRATOR_REAL_CARGO="$REAL_CARGO"
export ORCHESTRATOR_REAL_GIT="$REAL_GIT"
export ORCHESTRATOR_REAL_GH="$REAL_GH"
export ORCHESTRATOR_REAL_BWRAP="$REAL_BWRAP"
export ORCHESTRATOR_PARENT_HOME="$PARENT_HOME"
export ORCHESTRATOR_OPENCODE_CORE="$WRAPPER_DIR/opencode-core"
export ORCHESTRATOR_AGENT_SANDBOX="$WRAPPER_DIR/agent-sandbox"
export ORCHESTRATOR_AGENT_WRAPPER_DIR="$AGENT_WRAPPER_DIR"
export ORCHESTRATOR_AGENT_HOME="$AGENT_HOME"
export ORCHESTRATOR_AGENT_CONFIG_DIR="$AGENT_CONFIG_DIR"
export ORCHESTRATOR_AGENT_DATA_DIR="$AGENT_DATA_DIR"
export ORCHESTRATOR_AGENT_CACHE_DIR="$AGENT_CACHE_DIR"
export ORCHESTRATOR_ORIGINAL_PATH="$PATH"
export ORCHESTRATOR_AGENT_PATH="$AGENT_WRAPPER_DIR:$PATH"
export PATH="$WRAPPER_DIR:$PATH"

printf '\n===== BUILD ORCHESTRATOR =====\n'
cargo build --release

printf '\n===== PREFLIGHT =====\n'
./target/release/orchestrator doctor

printf '\n===== START AUTONOMOUS LOOP =====\n'
printf 'organization=%s\n' "$ORGANIZATION"
printf 'model=%s\n' "$ORCHESTRATOR_MODEL"
printf 'surgical_model=%s\n' "$ORCHESTRATOR_SURGICAL_MODEL"
printf 'interval=%ss\n' "$ORCHESTRATOR_INTERVAL_SECS"
printf 'auto_merge=%s\n' "$ORCHESTRATOR_AUTO_MERGE"
printf 'full_validation=%s\n' "$ORCHESTRATOR_FULL_VALIDATION"
printf 'opencode_bridge=isolated-ollama-launch+runtime-permissions\n'
printf 'cargo_bridge=ci-pinned-rustfmt\n'
printf 'git_bridge=push-race-recovery\n'
printf 'gh_bridge=pr-base-sync\n'
printf 'agent_cargo_bridge=enabled\n'
printf 'agent_git_bridge=read-only\n'
printf 'agent_gh_bridge=read-only-no-credentials\n'
printf 'agent_config=isolate-home-xdg\n'
printf 'agent_process_sandbox=bubblewrap-hidden-parent-home\n\n'

exec ./target/release/orchestrator run "$ORGANIZATION"
