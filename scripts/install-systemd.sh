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

for command_name in git gh ollama opencode cargo rustc systemctl; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf 'ERROR: required command not found: %s\n' "$command_name" >&2
    exit 1
  fi
done

# Persist only non-secret runtime policy. GitHub authentication remains in the
# root account's existing gh credential/config store; no token is copied here.
# Qwen 3.8 is the sole autonomous model for primary, surgical, and follow-up
# work. Auto-merge remains disabled until unattended validation is proven.
umask 077
cat >"$ENV_PATH" <<'EOF'
ORCHESTRATOR_MODEL=ollama/qwen3.8:latest
ORCHESTRATOR_SURGICAL_MODEL=ollama/qwen3.8:latest
ORCHESTRATOR_INTERVAL_SECS=180
ORCHESTRATOR_AUTO_MERGE=0
ORCHESTRATOR_FULL_VALIDATION=1
ORCHESTRATOR_BACKEND_ERROR_MAX=3
ORCHESTRATOR_PRIMARY_EDIT_MAX_TOOLS=24
ORCHESTRATOR_PRIMARY_EDIT_IDLE_SECS=420
ORCHESTRATOR_SURGICAL_EDIT_MAX_TOOLS=16
ORCHESTRATOR_SURGICAL_EDIT_IDLE_SECS=300
EOF
chmod 600 "$ENV_PATH"

cat >"$UNIT_PATH" <<EOF
[Unit]
Description=Memorithm autonomous repository orchestrator
Wants=network-online.target
After=network-online.target
StartLimitIntervalSec=300
StartLimitBurst=10

[Service]
Type=simple
User=root
WorkingDirectory=$ROOT
Environment=HOME=/root
Environment=XDG_CONFIG_HOME=/root/.config
Environment=XDG_DATA_HOME=/root/.local/share
Environment=PATH=/root/.cargo/bin:/root/.local/bin:/root/.opencode/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
EnvironmentFile=$ENV_PATH
ExecStart=/usr/bin/env bash $ROOT/scripts/start.sh Memorithm
Restart=always
RestartSec=15

# The Rust parent receives SIGINT first. Any descendant still alive after the
# grace period is killed as part of this unit's cgroup, never by a global pkill.
KillMode=mixed
KillSignal=SIGINT
FinalKillSignal=SIGKILL
SendSIGKILL=yes
TimeoutStopSec=30
TimeoutStopFailureMode=kill

# Keep runaway agent trees bounded without constraining normal Rust/CUDA work.
TasksMax=4096
LimitNOFILE=65536
OOMPolicy=stop
UMask=0077

StandardOutput=journal
StandardError=journal
SyslogIdentifier=memorithm-orchestrator

[Install]
WantedBy=multi-user.target
EOF
chmod 644 "$UNIT_PATH"

systemctl daemon-reload
systemctl enable --now "$SERVICE_NAME.service"

printf '\nMemorithm Orchestrator systemd service installed.\n'
printf 'Service : %s.service\n' "$SERVICE_NAME"
printf 'Unit    : %s\n' "$UNIT_PATH"
printf 'Policy  : %s\n' "$ENV_PATH"
printf '\nUseful commands:\n'
printf '  systemctl status %s --no-pager\n' "$SERVICE_NAME"
printf '  journalctl -u %s -f\n' "$SERVICE_NAME"
printf '  systemctl restart %s\n' "$SERVICE_NAME"
printf '  systemctl stop %s\n' "$SERVICE_NAME"
