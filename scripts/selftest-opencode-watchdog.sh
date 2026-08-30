#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REAL_GIT_BIN="$(command -v git)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/orchestrator-watchdog-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

BIN="$TMP/bin"
REPO="$TMP/repo"
mkdir -p "$BIN" "$REPO"

cat >"$BIN/ollama" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$*" == *"WATCHDOG_STDIN_SECRET"* ]]; then
  printf 'prompt secret leaked into fake ollama argv\n' >&2
  exit 91
fi
stdin_payload="$(cat)"
if [[ "$stdin_payload" != *"WATCHDOG_STDIN_SECRET"* ]]; then
  printf 'prompt secret missing from fake ollama stdin\n' >&2
  exit 92
fi

if [[ "${FAKE_AGENT_EDIT:-0}" == "1" ]]; then
  printf 'changed\n' >> "${FAKE_AGENT_EDIT_FILE:?}"
fi

for _ in 1 2 3 4 5 6; do
  printf 'INFO message=evaluated permission=read pattern=*\n'
done
sleep 30
EOF
chmod 700 "$BIN/ollama"

cat >"$BIN/cargo" <<'EOF'
#!/usr/bin/env bash
exec /bin/true
EOF
chmod 700 "$BIN/cargo"

cat >"$BIN/git" <<'EOF'
#!/usr/bin/env bash
exec "${REAL_GIT:?}" "$@"
EOF
chmod 700 "$BIN/git"

cd "$REPO"
"$REAL_GIT_BIN" init -q
"$REAL_GIT_BIN" config user.name test
"$REAL_GIT_BIN" config user.email test@example.invalid
printf 'base\n' > tracked.txt
"$REAL_GIT_BIN" add tracked.txt
"$REAL_GIT_BIN" commit -qm base

export REAL_GIT="$REAL_GIT_BIN"
export PATH="$BIN:$PATH"
export ORCHESTRATOR_AGENT_PATH="$BIN:$PATH"
export ORCHESTRATOR_REAL_OPENCODE=/bin/true
export ORCHESTRATOR_GENERAL_MAX_TOOLS=3
export ORCHESTRATOR_GENERAL_IDLE_SECS=30
export ORCHESTRATOR_GENERAL_MAX_SECS=30
export ORCHESTRATOR_BACKEND_ERROR_MAX=3

prompt='Repository: Memorithm/test
Task: ISSUE
Title: watchdog selftest
Marker: WATCHDOG_STDIN_SECRET'

set +e
printf '%s' "$prompt" | FAKE_AGENT_EDIT=0 bash "$ROOT/scripts/opencode" run --auto --model ollama/qwen3.8:latest >/tmp/orchestrator-watchdog-no-edit.log 2>&1
status=$?
set -e
if [[ "$status" -ne 124 ]]; then
  cat /tmp/orchestrator-watchdog-no-edit.log >&2 || true
  printf 'expected no-edit watchdog status 124, got %s\n' "$status" >&2
  exit 1
fi
if ! "$REAL_GIT_BIN" diff --quiet -- .; then
  printf 'no-edit watchdog unexpectedly changed repository\n' >&2
  exit 1
fi

set +e
printf '%s' "$prompt" | FAKE_AGENT_EDIT=1 FAKE_AGENT_EDIT_FILE="$REPO/tracked.txt" \
  bash "$ROOT/scripts/opencode" run --auto --model ollama/qwen3.8:latest >/tmp/orchestrator-watchdog-edit.log 2>&1
status=$?
set -e
if [[ "$status" -ne 0 ]]; then
  cat /tmp/orchestrator-watchdog-edit.log >&2 || true
  printf 'expected edited watchdog status 0, got %s\n' "$status" >&2
  exit 1
fi
if "$REAL_GIT_BIN" diff --quiet -- .; then
  printf 'edited watchdog failed to preserve working-tree progress\n' >&2
  exit 1
fi

printf 'general watchdog selftest: PASS\n'
