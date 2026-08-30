#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BWRAP="$(command -v bwrap || true)"
if [[ -z "$BWRAP" ]]; then
  printf 'agent sandbox selftest: bwrap is required\n' >&2
  exit 1
fi

TMP_ROOT="$(mktemp -d /var/tmp/orchestrator-agent-sandbox-test.XXXXXX)"
cleanup() {
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

DATA_ROOT="$TMP_ROOT/data"
WORKSPACE="$DATA_ROOT/workspaces/Memorithm__sandbox-test"
HOST_HOME="$TMP_ROOT/host-account-home"
RUNTIME_HOME="$TMP_ROOT/runtime-home"
AGENT_HOME="$TMP_ROOT/agent-home"
AGENT_BIN="$TMP_ROOT/agent-bin"
CARGO_HOME_HOST="$TMP_ROOT/cargo-home"
RUSTUP_HOME_HOST="$TMP_ROOT/rustup"
CARGO_BIN_HOST="$TMP_ROOT/cargo-bin"
OPENCODE_BIN_HOST="$TMP_ROOT/opencode-bin"
FAKE_OLLAMA="$TMP_ROOT/fake-ollama"
mkdir -p \
  "$WORKSPACE" "$HOST_HOME" "$RUNTIME_HOME" "$AGENT_HOME/.config/gh-empty" \
  "$AGENT_BIN" "$CARGO_HOME_HOST" "$RUSTUP_HOME_HOST" "$CARGO_BIN_HOST" \
  "$OPENCODE_BIN_HOST"
printf 'host-secret\n' > "$HOST_HOME/secret.txt"
printf 'runtime-secret\n' > "$RUNTIME_HOME/secret.txt"
printf 'base\n' > "$WORKSPACE/tracked.txt"
git -C "$WORKSPACE" init -q

cat > "$CARGO_BIN_HOST/cargo" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat > "$OPENCODE_BIN_HOST/opencode" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat > "$FAKE_OLLAMA" <<'EOF'
#!/usr/bin/env bash
set -eu
host_secret="$1"
runtime_secret="$2"
test ! -e "$host_secret"
test ! -e "$runtime_secret"
test -z "${GH_TOKEN:-}"
test -z "${GITHUB_TOKEN:-}"
test -z "${SSH_AUTH_SOCK:-}"
test "$HOME" = /tmp/orchestrator/agent-home
test "$GH_CONFIG_DIR" = /tmp/orchestrator/agent-home/.config/gh-empty
test "$PWD" = /tmp/orchestrator/workspace
test -c /dev/null
for path in /dev/sda /dev/vda /dev/nvme0n1 /dev/mmcblk0 /dev/mem; do
  test ! -e "$path"
done
test ! -s /etc/shadow
if touch .git/sandbox-must-not-write 2>/dev/null; then
  echo 'sandbox unexpectedly wrote .git' >&2
  exit 31
fi
printf 'sandbox-write\n' >> tracked.txt
EOF
chmod 700 "$CARGO_BIN_HOST/cargo" "$OPENCODE_BIN_HOST/opencode" "$FAKE_OLLAMA"

export ORCHESTRATOR_REAL_BWRAP="$BWRAP"
export ORCHESTRATOR_DATA_ROOT="$DATA_ROOT"
export ORCHESTRATOR_HOST_ACCOUNT_HOME="$HOST_HOME"
export ORCHESTRATOR_RUNTIME_HOME="$RUNTIME_HOME"
export ORCHESTRATOR_AGENT_HOME="$AGENT_HOME"
export ORCHESTRATOR_AGENT_WRAPPER_DIR="$AGENT_BIN"
export ORCHESTRATOR_REAL_CARGO="$CARGO_BIN_HOST/cargo"
export ORCHESTRATOR_REAL_OPENCODE="$OPENCODE_BIN_HOST/opencode"
export ORCHESTRATOR_REAL_GIT="$(command -v git)"
export ORCHESTRATOR_REAL_GH="$(command -v gh || command -v true)"
export ORCHESTRATOR_REAL_OLLAMA="$FAKE_OLLAMA"
export ORCHESTRATOR_AGENT_SANDBOX="$ROOT/scripts/agent-sandbox"
export CARGO_HOME="$CARGO_HOME_HOST"
export RUSTUP_HOME="$RUSTUP_HOME_HOST"
export GH_TOKEN='must-not-cross-sandbox'
export GITHUB_TOKEN='must-not-cross-sandbox-either'
export SSH_AUTH_SOCK='/tmp/must-not-cross.sock'

(
  cd "$WORKSPACE"
  bash "$ROOT/scripts/agent-ollama" "$HOST_HOME/secret.txt" "$RUNTIME_HOME/secret.txt"
)

if ! grep -Fxq 'sandbox-write' "$WORKSPACE/tracked.txt"; then
  printf 'agent sandbox selftest: workspace write did not propagate\n' >&2
  exit 1
fi
if [[ -e "$WORKSPACE/.git/sandbox-must-not-write" ]]; then
  printf 'agent sandbox selftest: .git mutation propagated\n' >&2
  exit 1
fi

printf 'agent process sandbox selftest: PASS\n'
