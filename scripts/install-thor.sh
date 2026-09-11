#!/usr/bin/env bash
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
  printf 'ERROR: Thor deployment must run as root\n' >&2
  exit 1
fi

ARCH="$(uname -m)"
if [[ "$ARCH" != "aarch64" && "$ARCH" != "arm64" ]]; then
  printf 'ERROR: Thor deployment profile expects ARM64/aarch64, got %s\n' "$ARCH" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Thor is intended to run Memorithm research continuously. Auto-merge here is
# not a bypass: Orchestrator still requires repository policy, exact-head CI,
# validated base identity, authorship and evidence gates before every merge.
export ORCHESTRATOR_INSTALL_AUTO_MERGE="${ORCHESTRATOR_INSTALL_AUTO_MERGE:-1}"
export ORCHESTRATOR_INSTALL_INTERVAL_SECS="${ORCHESTRATOR_INSTALL_INTERVAL_SECS:-60}"

exec bash "$ROOT/scripts/install-systemd.sh"
