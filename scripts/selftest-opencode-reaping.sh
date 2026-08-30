#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REAL_GIT="$(command -v git)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/orchestrator-reaping-test.XXXXXX")"
AGENT_PID=""
AGENT_CHILD_PID=""

cleanup() {
  if [[ -n "$AGENT_PID" ]] && kill -0 "$AGENT_PID" 2>/dev/null; then
    kill -KILL -- "-$AGENT_PID" 2>/dev/null || kill -KILL "$AGENT_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

BIN="$TMP/bin"
REPO="$TMP/repo"
mkdir -p "$BIN" "$REPO"

cat >"$BIN/ollama" <<'EOF'
#!/usr/bin/env bash
set -u

stdin_payload="$(cat)"
if [[ "$stdin_payload" != *"REAPING_STDIN_MARKER"* ]]; then
  printf 'fake ollama did not receive prompt on stdin\n' >&2
  exit 92
fi
printf '%s\n' "$$" >"${FAKE_AGENT_PID_FILE:?}"

sleep 300 &
child_pid=$!
printf '%s\n' "$child_pid" >"${FAKE_AGENT_CHILD_PID_FILE:?}"

trap 'exit 0' INT TERM HUP
wait "$child_pid" 2>/dev/null || true
EOF
chmod 700 "$BIN/ollama"

cd "$REPO"
"$REAL_GIT" init -q
"$REAL_GIT" config user.name test
"$REAL_GIT" config user.email test@example.invalid
printf 'base\n' > tracked.txt
"$REAL_GIT" add tracked.txt
"$REAL_GIT" commit -qm base

export PATH="$BIN:$PATH"
export ORCHESTRATOR_AGENT_PATH="$BIN:$PATH"
export ORCHESTRATOR_REAL_OPENCODE=/bin/true
export ORCHESTRATOR_GENERAL_MAX_TOOLS=96
export ORCHESTRATOR_GENERAL_IDLE_SECS=120
export ORCHESTRATOR_GENERAL_MAX_SECS=300
export ORCHESTRATOR_BACKEND_ERROR_MAX=3

wait_for_dead() {
  local pid="$1"
  local attempt
  for attempt in $(seq 1 120); do
    if ! kill -0 "$pid" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  printf 'process %s survived wrapper shutdown\n' "$pid" >&2
  return 1
}

run_signal_case() {
  local signal="$1"
  local expected_status="$2"
  local agent_file="$TMP/agent-$signal.pid"
  local child_file="$TMP/child-$signal.pid"
  local log_file="$TMP/wrapper-$signal.log"
  local status
  local prompt

  prompt=$'Repository: Memorithm/test\nTask: ISSUE\nTitle: process reaping selftest\nMarker: REAPING_STDIN_MARKER'
  rm -f "$agent_file" "$child_file"

  set +e
  printf '%s' "$prompt" | \
    FAKE_AGENT_PID_FILE="$agent_file" \
    FAKE_AGENT_CHILD_PID_FILE="$child_file" \
    timeout --preserve-status --signal="$signal" 2s \
      bash "$ROOT/scripts/opencode" run --auto --model ollama/qwen3.8:latest \
      >"$log_file" 2>&1
  status=$?
  set -e

  if [[ ! -s "$agent_file" || ! -s "$child_file" ]]; then
    cat "$log_file" >&2 || true
    printf 'fake agent did not publish process identifiers before %s\n' "$signal" >&2
    return 1
  fi
  AGENT_PID="$(cat "$agent_file")"
  AGENT_CHILD_PID="$(cat "$child_file")"

  if [[ "$status" -ne "$expected_status" ]]; then
    cat "$log_file" >&2 || true
    printf 'expected wrapper status %s after %s, got %s\n' "$expected_status" "$signal" "$status" >&2
    return 1
  fi

  wait_for_dead "$AGENT_PID"
  wait_for_dead "$AGENT_CHILD_PID"
  AGENT_PID=""
  AGENT_CHILD_PID=""
}

run_signal_case INT 130
run_signal_case TERM 143

printf 'agent process reaping selftest: PASS\n'
