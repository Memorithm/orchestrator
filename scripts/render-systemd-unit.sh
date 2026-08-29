#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:?repository root is required}"
RUNTIME_DATA="${2:-/root/.local/share/memorithm-orchestrator}"
ENV_PATH="${3:-/etc/memorithm-orchestrator.env}"
RUNTIME_HOME="$RUNTIME_DATA/runtime-home"
CARGO_HOME_PATH="$RUNTIME_DATA/cargo-home"

cat <<EOF
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

# Give the daemon a private logical home while retaining read-only access to
# the operator's existing GitHub/Git/Rust authentication and toolchain state.
Environment=HOME=$RUNTIME_HOME
Environment=XDG_CONFIG_HOME=$RUNTIME_HOME/.config
Environment=XDG_DATA_HOME=$RUNTIME_HOME/.local/share
Environment=XDG_CACHE_HOME=$RUNTIME_HOME/.cache
Environment=GH_CONFIG_DIR=/root/.config/gh
Environment=GIT_CONFIG_GLOBAL=/root/.gitconfig
Environment=CARGO_HOME=$CARGO_HOME_PATH
Environment=RUSTUP_HOME=/root/.rustup
Environment=PATH=/root/.cargo/bin:/root/.local/bin:/root/.opencode/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
EnvironmentFile=-$ENV_PATH
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

# Host filesystem containment. Source/toolchain/auth state is readable, while
# autonomous writes are restricted to Orchestrator data, build output and the
# unit's private /tmp. GPU/device access is intentionally left available.
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=-$RUNTIME_DATA
ReadWritePaths=-$ROOT/target
PrivateTmp=true
NoNewPrivileges=true
ProtectControlGroups=true
ProtectKernelModules=true
ProtectKernelTunables=true
ProtectClock=true
RestrictSUIDSGID=true
LockPersonality=true

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
