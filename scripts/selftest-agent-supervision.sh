#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/orchestrator-supervision.XXXXXX")"
trap 'rm -rf "$TMP_ROOT"' EXIT

fail() {
  printf 'SELFTEST FAIL: %s\n' "$*" >&2
  exit 1
}

mkdir -p "$TMP_ROOT/bin" "$TMP_ROOT/repo"
cat >"$TMP_ROOT/bin/ollama" <<'EOF'
#!/usr/bin/env bash
set -u
case "${SELFTEST_MODE:-}" in
  idle)
    sleep 30
    ;;
  backend)
    printf '%s\n' \
      'timestamp=x level=ERROR message="No user query found in messages"' \
      'timestamp=x level=ERROR message="No user query found in messages"'
    sleep 30
    ;;
  tools)
    for index in 1 2 3 4; do
      printf 'timestamp=x level=INFO message=evaluated permission=read index=%s\n' "$index"
      sleep 0.2
    done
    sleep 30
    ;;
  wall)
    for index in 1 2 3 4 5 6 7 8 9 10 11 12; do
      printf 'timestamp=x level=INFO message=evaluated permission=read index=%s\n' "$index"
      sleep 0.5
    done
    sleep 30
    ;;
  edit-stall)
    printf 'agent-edit\n' >>tracked.txt
    sleep 30
    ;;
  *)
    printf 'unknown SELFTEST_MODE=%s\n' "${SELFTEST_MODE:-}" >&2
    exit 2
    ;;
esac
EOF
chmod 700 "$TMP_ROOT/bin/ollama"

(
  cd "$TMP_ROOT/repo"
  git init -q
  git config user.name selftest
  git config user.email selftest@localhost
  printf 'baseline\n' >tracked.txt
  git add tracked.txt
  git commit -qm baseline
)

export ORCHESTRATOR_REAL_OPENCODE=/bin/false
export ORCHESTRATOR_AGENT_PATH="$TMP_ROOT/bin:/usr/bin:/bin"
export ORCHESTRATOR_SURGICAL_MODEL=ollama/qwen3.8:latest
export OPENCODE_CONFIG_CONTENT='{"permission":{}}'
export ORCHESTRATOR_BACKEND_ERROR_MAX=2
export ORCHESTRATOR_GENERAL_MAX_TOOLS=100
export ORCHESTRATOR_GENERAL_IDLE_SECS=10
export ORCHESTRATOR_GENERAL_MAX_SECS=20

run_case() {
  local mode="$1"
  local expected="$2"
  local output_file="$TMP_ROOT/$mode.out"
  local status

  set +e
  (
    cd "$TMP_ROOT/repo"
    SELFTEST_MODE="$mode" bash "$ROOT/scripts/opencode" \
      run --auto --model ollama/qwen3.8:latest \
      'Repository: Memorithm/test
Task: ISSUE
Title: supervision selftest'
  ) >"$output_file" 2>&1
  status=$?
  set -e

  if [[ "$status" -ne "$expected" ]]; then
    cat "$output_file" >&2
    fail "$mode returned $status, expected $expected"
  fi
}

ORCHESTRATOR_GENERAL_IDLE_SECS=2 run_case idle 124
git -C "$TMP_ROOT/repo" reset --hard -q HEAD

ORCHESTRATOR_GENERAL_IDLE_SECS=10 run_case backend 70
git -C "$TMP_ROOT/repo" reset --hard -q HEAD

ORCHESTRATOR_GENERAL_MAX_TOOLS=2 ORCHESTRATOR_GENERAL_IDLE_SECS=10 run_case tools 124
git -C "$TMP_ROOT/repo" reset --hard -q HEAD

ORCHESTRATOR_GENERAL_MAX_TOOLS=100 ORCHESTRATOR_GENERAL_IDLE_SECS=10 ORCHESTRATOR_GENERAL_MAX_SECS=2 run_case wall 124
git -C "$TMP_ROOT/repo" reset --hard -q HEAD

ORCHESTRATOR_GENERAL_MAX_TOOLS=100 ORCHESTRATOR_GENERAL_IDLE_SECS=2 ORCHESTRATOR_GENERAL_MAX_SECS=20 run_case edit-stall 0
if git -C "$TMP_ROOT/repo" diff --quiet -- tracked.txt; then
  fail 'edit-stall did not preserve working-tree progress'
fi

grep -q 'idle budget' "$TMP_ROOT/idle.out" || fail 'idle watchdog marker missing'
grep -q 'BACKEND CIRCUIT BREAKER' "$TMP_ROOT/backend.out" || fail 'backend circuit marker missing'
grep -q 'tool-action budget' "$TMP_ROOT/tools.out" || fail 'tool budget marker missing'
grep -q 'total runtime budget' "$TMP_ROOT/wall.out" || fail 'wall budget marker missing'
grep -q 'preserving it for orchestrator validation' "$TMP_ROOT/edit-stall.out" || fail 'edit preservation marker missing'

printf 'agent supervision selftest: PASS\n'
