#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BWRAP="$(command -v bwrap || true)"
if [[ -z "$BWRAP" ]]; then
  printf 'agent sandbox selftest: bwrap is required\n' >&2
  exit 1
fi

TMP_ROOT="$(mktemp -d /var/tmp/orchestrator-agent-sandbox-test.XXXXXX)"
SECRET="$HOME/.orchestrator-agent-sandbox-secret-$$"
cleanup() {
  rm -rf "$TMP_ROOT"
  rm -f "$SECRET"
}
trap cleanup EXIT

DATA_ROOT="$TMP_ROOT/data"
WORKSPACE="$DATA_ROOT/workspaces/Memorithm__sandbox-test"
AGENT_HOME="$TMP_ROOT/agent-home"
AGENT_BIN="$TMP_ROOT/agent-bin"
CARGO_HOME_HOST="$TMP_ROOT/cargo-home"
RUSTUP_HOME_HOST="$TMP_ROOT/rustup"
CARGO_BIN_HOST="$TMP_ROOT/cargo-bin"
OPENCODE_BIN_HOST="$TMP_ROOT/opencode-bin"
mkdir -p \
  "$WORKSPACE" "$AGENT_HOME/.config" "$AGENT_BIN" "$CARGO_HOME_HOST" \
  "$RUSTUP_HOME_HOST" "$CARGO_BIN_HOST" "$OPENCODE_BIN_HOST"
printf 'base\n' > "$WORKSPACE/tracked.txt"
printf 'parent-secret\n' > "$SECRET"

cat > "$CARGO_BIN_HOST/cargo" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat > "$OPENCODE_BIN_HOST/opencode" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod 700 "$CARGO_BIN_HOST/cargo" "$OPENCODE_BIN_HOST/opencode"

export ORCHESTRATOR_REAL_BWRAP="$BWRAP"
export ORCHESTRATOR_DATA_ROOT="$DATA_ROOT"
export ORCHESTRATOR_PARENT_HOME="$HOME"
export ORCHESTRATOR_AGENT_HOME="$AGENT_HOME"
export ORCHESTRATOR_AGENT_WRAPPER_DIR="$AGENT_BIN"
export ORCHESTRATOR_REAL_CARGO="$CARGO_BIN_HOST/cargo"
export ORCHESTRATOR_REAL_OPENCODE="$OPENCODE_BIN_HOST/opencode"
export ORCHESTRATOR_REAL_GIT="$(command -v git)"
export ORCHESTRATOR_REAL_GH="$(command -v gh || command -v true)"
export CARGO_HOME="$CARGO_HOME_HOST"
export RUSTUP_HOME="$RUSTUP_HOME_HOST"
export GH_TOKEN='must-not-cross-sandbox'
export GITHUB_TOKEN='must-not-cross-sandbox-either'
export SSH_AUTH_SOCK='/tmp/must-not-cross.sock'

(
  cd "$WORKSPACE"
  bash "$ROOT/scripts/agent-sandbox" -- bash -c '
    set -eu
    secret_path="$1"
    test ! -e "$secret_path"
    test -z "${GH_TOKEN:-}"
    test -z "${GITHUB_TOKEN:-}"
    test -z "${SSH_AUTH_SOCK:-}"
    test "$HOME" = /tmp/orchestrator/agent-home
    test "$GH_CONFIG_DIR" = /tmp/orchestrator/agent-home/.config/gh-empty
    test "$PWD" = /tmp/orchestrator/workspace
    printf "sandbox-write\n" >> tracked.txt
  ' sandbox-test "$SECRET"
)

if ! grep -Fxq 'sandbox-write' "$WORKSPACE/tracked.txt"; then
  printf 'agent sandbox selftest: workspace write did not propagate\n' >&2
  exit 1
fi

printf 'agent process sandbox selftest: PASS\n'
