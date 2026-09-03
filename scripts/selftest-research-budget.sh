#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="$ROOT/scripts/opencode-env"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/orchestrator-research-budget.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

mkdir -p \
  "$TMP/agent-home" \
  "$TMP/agent-config" \
  "$TMP/agent-data" \
  "$TMP/agent-cache" \
  "$TMP/gh-config"

cat >"$TMP/fake-core" <<'CORE'
#!/usr/bin/env bash
set -euo pipefail
cat >/dev/null
printf '%s|%s|%s\n' \
  "${ORCHESTRATOR_GENERAL_MAX_TOOLS:-unset}" \
  "${ORCHESTRATOR_GENERAL_IDLE_SECS:-unset}" \
  "${ORCHESTRATOR_GENERAL_MAX_SECS:-unset}" \
  >"${RESEARCH_BUDGET_CAPTURE:?}"
CORE
chmod +x "$TMP/fake-core"

cat >"$TMP/fake-helper" <<'HELPER'
#!/usr/bin/env bash
set -euo pipefail
mode="${1:-transform}"
case "$mode" in
  transform)
    prompt="$(cat)"
    if [[ "$prompt" == *"EXPLICIT_RESEARCH_OPT_IN"* ]]; then
      printf '%s\nAUTONOMOUS RESEARCH MISSION (EXPLICIT ISSUE OPT-IN)' "$prompt"
    else
      printf '%s' "$prompt"
    fi
    ;;
  record)
    cat >/dev/null
    ;;
  *) exit 2 ;;
esac
HELPER
chmod +x "$TMP/fake-helper"

common_env=(
  ORCHESTRATOR_OPENCODE_CORE="$TMP/fake-core"
  ORCHESTRATOR_AGENT_HOME="$TMP/agent-home"
  ORCHESTRATOR_AGENT_CONFIG_DIR="$TMP/agent-config"
  ORCHESTRATOR_AGENT_DATA_DIR="$TMP/agent-data"
  ORCHESTRATOR_AGENT_CACHE_DIR="$TMP/agent-cache"
  ORCHESTRATOR_GH_CONFIG_DIR="$TMP/gh-config"
  ORCHESTRATOR_RESEARCH_PROMPT_HELPER="$TMP/fake-helper"
)

run_case() {
  local name="$1"
  local expected="$2"
  local prompt="$3"
  shift 3
  local capture="$TMP/$name.capture"
  rm -f "$capture"
  env \
    -u ORCHESTRATOR_GENERAL_MAX_TOOLS \
    -u ORCHESTRATOR_GENERAL_IDLE_SECS \
    -u ORCHESTRATOR_GENERAL_MAX_SECS \
    -u ORCHESTRATOR_RESEARCH_MAX_TOOLS \
    -u ORCHESTRATOR_RESEARCH_IDLE_SECS \
    -u ORCHESTRATOR_RESEARCH_MAX_SECS \
    "${common_env[@]}" \
    RESEARCH_BUDGET_CAPTURE="$capture" \
    "$@" \
    bash "$TARGET" run --auto --model ollama/test <<<"$prompt"
  actual="$(cat "$capture")"
  if [[ "$actual" != "$expected" ]]; then
    printf 'research budget selftest %s: expected %s got %s\n' "$name" "$expected" "$actual" >&2
    exit 1
  fi
}

run_case ordinary 'unset|unset|unset' 'ordinary issue prompt'
run_case research-default '48|420|2700' 'EXPLICIT_RESEARCH_OPT_IN'
run_case general-stricter '20|300|1000' 'EXPLICIT_RESEARCH_OPT_IN' \
  ORCHESTRATOR_GENERAL_MAX_TOOLS=20 \
  ORCHESTRATOR_GENERAL_IDLE_SECS=300 \
  ORCHESTRATOR_GENERAL_MAX_SECS=1000
run_case research-stricter '12|60|600' 'EXPLICIT_RESEARCH_OPT_IN' \
  ORCHESTRATOR_RESEARCH_MAX_TOOLS=12 \
  ORCHESTRATOR_RESEARCH_IDLE_SECS=60 \
  ORCHESTRATOR_RESEARCH_MAX_SECS=600
run_case research-cannot-expand '40|400|2000' 'EXPLICIT_RESEARCH_OPT_IN' \
  ORCHESTRATOR_GENERAL_MAX_TOOLS=40 \
  ORCHESTRATOR_GENERAL_IDLE_SECS=400 \
  ORCHESTRATOR_GENERAL_MAX_SECS=2000 \
  ORCHESTRATOR_RESEARCH_MAX_TOOLS=80 \
  ORCHESTRATOR_RESEARCH_IDLE_SECS=800 \
  ORCHESTRATOR_RESEARCH_MAX_SECS=4000

# Invalid research-only configuration is ignored for ordinary work because the
# helper did not classify it as an opted-in research cycle.
run_case ordinary-ignores-invalid 'unset|unset|unset' 'ordinary issue prompt' \
  ORCHESTRATOR_RESEARCH_MAX_TOOLS=0

capture="$TMP/invalid.capture"
rm -f "$capture"
set +e
env \
  -u ORCHESTRATOR_GENERAL_MAX_TOOLS \
  -u ORCHESTRATOR_GENERAL_IDLE_SECS \
  -u ORCHESTRATOR_GENERAL_MAX_SECS \
  -u ORCHESTRATOR_RESEARCH_IDLE_SECS \
  -u ORCHESTRATOR_RESEARCH_MAX_SECS \
  "${common_env[@]}" \
  RESEARCH_BUDGET_CAPTURE="$capture" \
  ORCHESTRATOR_RESEARCH_MAX_TOOLS=0 \
  bash "$TARGET" run --auto --model ollama/test <<<"EXPLICIT_RESEARCH_OPT_IN"
status=$?
set -e
if [[ "$status" -ne 70 ]]; then
  printf 'research budget selftest invalid config: expected exit 70 got %s\n' "$status" >&2
  exit 1
fi
if [[ -e "$capture" ]]; then
  printf 'research budget selftest invalid config: core ran despite rejected budget\n' >&2
  exit 1
fi

printf 'research resource budget selftest: PASS\n'
