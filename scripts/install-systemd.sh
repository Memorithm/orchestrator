#!/usr/bin/env bash
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
  printf 'ERROR: this installer must run as root\n' >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVICE_NAME="memorithm-orchestrator"
UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}.service"
ENV_PATH="/etc/${SERVICE_NAME}.env"
RUNTIME_DATA="/root/.local/share/memorithm-orchestrator"
RUNTIME_HOME="$RUNTIME_DATA/runtime-home"
CARGO_HOME_PATH="$RUNTIME_DATA/cargo-home"

for command_name in git gh ollama opencode cargo rustc systemctl; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf 'ERROR: required command not found: %s\n' "$command_name" >&2
    exit 1
  fi
done

mkdir -p "$RUNTIME_HOME/.config" "$RUNTIME_HOME/.local/share" "$RUNTIME_HOME/.cache" "$CARGO_HOME_PATH"
chmod 700 "$RUNTIME_HOME" "$CARGO_HOME_PATH"

# Persist only non-secret runtime policy. GitHub authentication remains in the
# root account's existing gh credential/config store; no token is copied here.
# Qwen 3.8 is the sole autonomous model for primary, surgical, and follow-up
# work. Auto-merge remains disabled until unattended validation is proven.
umask 077
cat >"$ENV_PATH" <<'EOF'
ORCHESTRATOR_DATA_ROOT=/root/.local/share/memorithm-orchestrator
ORCHESTRATOR_MODEL=ollama/qwen3.8:latest
ORCHESTRATOR_SURGICAL_MODEL=ollama/qwen3.8:latest
ORCHESTRATOR_INTERVAL_SECS=180
ORCHESTRATOR_AUTO_MERGE=0
ORCHESTRATOR_AUTO_MERGE_SCOPE=orchestrator-validated
ORCHESTRATOR_FULL_VALIDATION=1
ORCHESTRATOR_BACKEND_ERROR_MAX=3
ORCHESTRATOR_PRIMARY_EDIT_MAX_TOOLS=24
ORCHESTRATOR_PRIMARY_EDIT_IDLE_SECS=420
ORCHESTRATOR_SURGICAL_EDIT_MAX_TOOLS=16
ORCHESTRATOR_SURGICAL_EDIT_IDLE_SECS=300
ORCHESTRATOR_GENERAL_MAX_TOOLS=96
ORCHESTRATOR_GENERAL_IDLE_SECS=600
ORCHESTRATOR_GENERAL_MAX_SECS=5400
ORCHESTRATOR_SUCCESS_COOLDOWN_SECS=900
ORCHESTRATOR_FAILURE_BASE_COOLDOWN_SECS=300
ORCHESTRATOR_FAILURE_MAX_COOLDOWN_SECS=7200
ORCHESTRATOR_TRANSIENT_FAILURE_COOLDOWN_SECS=180
ORCHESTRATOR_QUARANTINE_AFTER_FAILURES=4
ORCHESTRATOR_QUARANTINE_SECS=21600
EOF
chmod 600 "$ENV_PATH"

bash "$ROOT/scripts/render-systemd-unit.sh" "$ROOT" "$RUNTIME_DATA" "$ENV_PATH" >"$UNIT_PATH"
chmod 644 "$UNIT_PATH"

# Verify the exact installed unit before reloading systemd. A bad hardening
# directive must fail installation instead of leaving a partially active unit.
if command -v systemd-analyze >/dev/null 2>&1; then
  systemd-analyze verify "$UNIT_PATH"
fi

systemctl daemon-reload
systemctl enable --now "$SERVICE_NAME.service"

printf '\nMemorithm Orchestrator systemd service installed.\n'
printf 'Service : %s.service\n' "$SERVICE_NAME"
printf 'Unit    : %s\n' "$UNIT_PATH"
printf 'Policy  : %s\n' "$ENV_PATH"
printf 'Data    : %s\n' "$RUNTIME_DATA"
printf '\nUseful commands:\n'
printf '  systemctl status %s --no-pager\n' "$SERVICE_NAME"
printf '  journalctl -u %s -f\n' "$SERVICE_NAME"
printf '  systemctl restart %s\n' "$SERVICE_NAME"
printf '  systemctl stop %s\n' "$SERVICE_NAME"
