#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RUNTIME_HOME="${HOME:?HOME is not set}"
HOST_ACCOUNT_HOME="$(getent passwd "$(id -u)" | awk -F: 'NR == 1 { print $6 }')"
if [[ -z "$HOST_ACCOUNT_HOME" ]]; then
  HOST_ACCOUNT_HOME="$RUNTIME_HOME"
fi
export ORCHESTRATOR_DATA_ROOT="${ORCHESTRATOR_DATA_ROOT:-$HOST_ACCOUNT_HOME/.local/share/memorithm-orchestrator}"
export CARGO_HOME="${CARGO_HOME:-$ORCHESTRATOR_DATA_ROOT/cargo-home}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOST_ACCOUNT_HOME/.rustup}"
mkdir -p "$ORCHESTRATOR_DATA_ROOT" "$CARGO_HOME"

# Orchestrator intentionally runs one local model only. Keep the primary,
# surgical, and follow-up repair paths on the same deterministic local agent
# so stale service environment cannot silently re-enable another model.
export ORCHESTRATOR_MODEL="ollama/qwen3.8:latest"
export ORCHESTRATOR_SURGICAL_MODEL="ollama/qwen3.8:latest"
export ORCHESTRATOR_INTERVAL_SECS="${ORCHESTRATOR_INTERVAL_SECS:-180}"
export ORCHESTRATOR_AUTO_MERGE="${ORCHESTRATOR_AUTO_MERGE:-0}"
export ORCHESTRATOR_FULL_VALIDATION="${ORCHESTRATOR_FULL_VALIDATION:-0}"
export ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB="${ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB:-4096}"
export ORCHESTRATOR_MIN_FREE_DISK_MB="${ORCHESTRATOR_MIN_FREE_DISK_MB:-8192}"
export ORCHESTRATOR_MAX_LOAD_PER_CPU="${ORCHESTRATOR_MAX_LOAD_PER_CPU:-2.0}"
export ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS="${ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS:-4}"
export ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES="${ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES:-1}"
export ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS="${ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS:-604800}"
export ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM="${ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM:-50}"

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

REAL_OLLAMA="$(command -v ollama)"
if [[ -z "$REAL_OLLAMA" ]]; then
  printf 'ERROR: ollama is not installed or not on PATH\n' >&2
  exit 1
fi

REAL_BWRAP="$(command -v bwrap)"
if [[ -z "$REAL_BWRAP" ]]; then
  printf 'ERROR: bubblewrap (bwrap) is required for credential-isolated agent execution\n' >&2
  exit 1
fi

# Preserve authenticated GitHub access for the trusted Orchestrator parent.
# The coding process itself is launched through agent-ollama -> agent-sandbox,
# which clears credentials and masks the real account home.
REAL_GH_CONFIG_DIR="${GH_CONFIG_DIR:-${XDG_CONFIG_HOME:-$RUNTIME_HOME/.config}/gh}"

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
install -m 700 "$ROOT/scripts/validation-sandbox" "$WRAPPER_DIR/validation-sandbox"

# The coding worker receives managed read-only Git/GitHub bridges and a separate
# launch path. Only the launch path contains the ollama interceptor; it is not
# mounted inside the worker, preventing recursive sandbox entry.
AGENT_WRAPPER_DIR="$ROOT/target/orchestrator-agent-bin"
AGENT_LAUNCHER_DIR="$ROOT/target/orchestrator-agent-launcher-bin"
AGENT_HOME="${ORCHESTRATOR_AGENT_HOME:-$ROOT/target/orchestrator-agent-home}"
AGENT_CONFIG_DIR="$AGENT_HOME/.config"
AGENT_DATA_DIR="$AGENT_HOME/.local/share"
AGENT_CACHE_DIR="$AGENT_HOME/.cache"
mkdir -p \
  "$AGENT_WRAPPER_DIR" \
  "$AGENT_LAUNCHER_DIR" \
  "$AGENT_HOME" \
  "$AGENT_CONFIG_DIR" \
  "$AGENT_DATA_DIR" \
  "$AGENT_CACHE_DIR" \
  "$AGENT_CONFIG_DIR/gh-empty"
install -m 700 "$ROOT/scripts/cargo" "$AGENT_WRAPPER_DIR/cargo"
install -m 700 "$ROOT/scripts/agent-git" "$AGENT_WRAPPER_DIR/git"
install -m 700 "$ROOT/scripts/agent-gh" "$AGENT_WRAPPER_DIR/gh"
install -m 700 "$ROOT/scripts/agent-ollama" "$AGENT_LAUNCHER_DIR/ollama"

export ORCHESTRATOR_REAL_OPENCODE="$REAL_OPENCODE"
export ORCHESTRATOR_REAL_CARGO="$REAL_CARGO"
export ORCHESTRATOR_REAL_GIT="$REAL_GIT"
export ORCHESTRATOR_REAL_GH="$REAL_GH"
export ORCHESTRATOR_REAL_OLLAMA="$REAL_OLLAMA"
export ORCHESTRATOR_REAL_BWRAP="$REAL_BWRAP"
export ORCHESTRATOR_GH_CONFIG_DIR="$REAL_GH_CONFIG_DIR"
export ORCHESTRATOR_HOST_ACCOUNT_HOME="$HOST_ACCOUNT_HOME"
export ORCHESTRATOR_RUNTIME_HOME="$RUNTIME_HOME"
export ORCHESTRATOR_OPENCODE_CORE="$WRAPPER_DIR/opencode-core"
export ORCHESTRATOR_AGENT_SANDBOX="$WRAPPER_DIR/agent-sandbox"
export ORCHESTRATOR_VALIDATION_SANDBOX="$WRAPPER_DIR/validation-sandbox"
export ORCHESTRATOR_AGENT_WRAPPER_DIR="$AGENT_WRAPPER_DIR"
export ORCHESTRATOR_AGENT_HOME="$AGENT_HOME"
export ORCHESTRATOR_AGENT_CONFIG_DIR="$AGENT_CONFIG_DIR"
export ORCHESTRATOR_AGENT_DATA_DIR="$AGENT_DATA_DIR"
export ORCHESTRATOR_AGENT_CACHE_DIR="$AGENT_CACHE_DIR"
export ORCHESTRATOR_ORIGINAL_PATH="$PATH"
export ORCHESTRATOR_AGENT_PATH="$AGENT_LAUNCHER_DIR:$AGENT_WRAPPER_DIR:$PATH"
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
printf 'min_available_memory_mb=%s\n' "$ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB"
printf 'min_free_disk_mb=%s\n' "$ORCHESTRATOR_MIN_FREE_DISK_MB"
printf 'max_load_per_cpu=%s\n' "$ORCHESTRATOR_MAX_LOAD_PER_CPU"
printf 'low_disk_reclaim_max_targets=%s\n' "$ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS"
printf 'low_disk_reclaim_max_workspaces=%s\n' "$ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES"
printf 'workspace_min_idle_secs=%s\n' "$ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS"
printf 'trajectory_max_per_item=%s\n' "$ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM"
printf 'opencode_bridge=isolated-ollama-launch+runtime-permissions\n'
printf 'cargo_bridge=ci-pinned-rustfmt\n'
printf 'git_bridge=push-race-recovery\n'
printf 'gh_bridge=pr-base-sync\n'
printf 'agent_cargo_bridge=enabled\n'
printf 'agent_git_bridge=read-only\n'
printf 'agent_gh_bridge=read-only-no-credentials\n'
printf 'agent_config=isolate-home-xdg\n'
printf 'agent_process_sandbox=bubblewrap+private-dev+readonly-git+masked-host-state\n'
printf 'validation_sandbox=bubblewrap+private-net+readonly-git+masked-host-state\n\n'

exec ./target/release/orchestrator run "$ORGANIZATION"
