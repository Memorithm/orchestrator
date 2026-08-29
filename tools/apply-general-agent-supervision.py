#!/usr/bin/env python3
from pathlib import Path

path = Path("scripts/opencode")
source = path.read_text()


def replace_once(old: str, new: str, label: str) -> None:
    global source
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    source = source.replace(old, new, 1)


replace_once(
    '''surgical_edit_max_tools="${ORCHESTRATOR_SURGICAL_EDIT_MAX_TOOLS:-16}"
surgical_edit_idle_secs="${ORCHESTRATOR_SURGICAL_EDIT_IDLE_SECS:-300}"
backend_error_max="${ORCHESTRATOR_BACKEND_ERROR_MAX:-3}"
''',
    '''surgical_edit_max_tools="${ORCHESTRATOR_SURGICAL_EDIT_MAX_TOOLS:-16}"
surgical_edit_idle_secs="${ORCHESTRATOR_SURGICAL_EDIT_IDLE_SECS:-300}"
general_max_tools="${ORCHESTRATOR_GENERAL_MAX_TOOLS:-96}"
general_idle_secs="${ORCHESTRATOR_GENERAL_IDLE_SECS:-600}"
general_max_secs="${ORCHESTRATOR_GENERAL_MAX_SECS:-5400}"
backend_error_max="${ORCHESTRATOR_BACKEND_ERROR_MAX:-3}"
''',
    "general supervision variables",
)

replace_once(
    '''  "$surgical_edit_max_tools" \\
  "$surgical_edit_idle_secs" \\
  "$backend_error_max"; do
''',
    '''  "$surgical_edit_max_tools" \\
  "$surgical_edit_idle_secs" \\
  "$general_max_tools" \\
  "$general_idle_secs" \\
  "$general_max_secs" \\
  "$backend_error_max"; do
''',
    "general supervision validation",
)

replace_once(
    '''fi

validate_rust_ci() {
''',
    '''else
  args[$last_index]="${args[$last_index]}

AUTONOMOUS WORK BUDGET:
- This headless attempt has a hard budget of ${general_max_tools} tool actions and ${general_max_secs} seconds total runtime.
- More than ${general_idle_secs} seconds without a tool action or working-tree diff is treated as a stall.
- Do not spend the whole budget surveying history or adjacent code. Once enough evidence is gathered, implement the smallest coherent deliverable and validate it.
- As the budget gets low, stop expanding scope and leave the working tree in a validated, reviewable state."
fi

validate_rust_ci() {
''',
    "general budget prompt",
)

replace_once(
    '''if (( is_ci_fix == 1 && local_failure_available == 1 )); then
  printf 'first-edit watchdog: %s tool actions / %ss idle\\n' "$primary_edit_max_tools" "$primary_edit_idle_secs"
  printf 'surgical retry: %s tool actions / %ss idle, history+GitHub denied\\n' "$surgical_edit_max_tools" "$surgical_edit_idle_secs"
  printf 'surgical fallback: %s\\n' "$surgical_model_id"
fi
''',
    '''if (( is_ci_fix == 1 && local_failure_available == 1 )); then
  printf 'first-edit watchdog: %s tool actions / %ss idle\\n' "$primary_edit_max_tools" "$primary_edit_idle_secs"
  printf 'surgical retry: %s tool actions / %ss idle, history+GitHub denied\\n' "$surgical_edit_max_tools" "$surgical_edit_idle_secs"
  printf 'surgical fallback: %s\\n' "$surgical_model_id"
else
  printf 'general watchdog: %s tool actions / %ss idle / %ss total\\n' \\
    "$general_max_tools" "$general_idle_secs" "$general_max_secs"
fi
''',
    "general budget logging",
)

replace_once(
    '''  while kill -0 "$AGENT_PID" 2>/dev/null; do
    if trip_backend_circuit_if_needed "$log_file"; then
      stop_agent_group "$AGENT_PID"
      wait "$AGENT_PID" 2>/dev/null || true
      rm -f "$log_file"
      return 70
    fi

    current_signature="$(diff_signature)"
''',
    '''  while kill -0 "$AGENT_PID" 2>/dev/null; do
    if trip_backend_circuit_if_needed "$log_file"; then
      stop_agent_group "$AGENT_PID"
      wait "$AGENT_PID" 2>/dev/null || true
      current_signature="$(diff_signature)"
      rm -f "$log_file"
      if [[ "$current_signature" != "$baseline_signature" ]]; then
        printf 'backend failed after a working-tree edit; preserving edits for deterministic validation\\n' >&2
        return 0
      fi
      return 70
    fi

    current_signature="$(diff_signature)"
''',
    "CI backend partial-edit recovery",
)

replace_once(
    '''      stop_agent_group "$AGENT_PID"
      wait "$AGENT_PID" 2>/dev/null || true
      rm -f "$log_file"
      return 124
    fi
    sleep 2
''',
    '''      stop_agent_group "$AGENT_PID"
      wait "$AGENT_PID" 2>/dev/null || true
      rm -f "$log_file"
      if (( edited == 1 )); then
        printf 'agent stalled after editing; preserving edits for deterministic validation\\n'
        return 0
      fi
      return 124
    fi
    sleep 2
''',
    "CI post-edit stall recovery",
)

old_general = '''run_agent_unbounded() {
  local log_file
  local status
  log_file="$(mktemp "${TMPDIR:-/tmp}/memorithm-opencode-pass.XXXXXX.log")" || return 1
  start_agent "$permission_json" "$log_file" "$model_id"

  while kill -0 "$AGENT_PID" 2>/dev/null; do
    if trip_backend_circuit_if_needed "$log_file"; then
      stop_agent_group "$AGENT_PID"
      wait "$AGENT_PID" 2>/dev/null || true
      rm -f "$log_file"
      return 70
    fi
    sleep 2
  done

  wait "$AGENT_PID"
  status=$?
  rm -f "$log_file"
  return "$status"
}
'''

new_general = '''run_agent_with_general_watchdog() {
  local baseline_signature
  local current_signature
  local last_signature
  local log_file
  local tool_count
  local last_tool_count=0
  local idle_deadline
  local wall_deadline
  local status
  local stop_reason
  local stop_status

  baseline_signature="$(diff_signature)"
  last_signature="$baseline_signature"
  log_file="$(mktemp "${TMPDIR:-/tmp}/memorithm-opencode-pass.XXXXXX.log")" || return 1
  idle_deadline=$((SECONDS + general_idle_secs))
  wall_deadline=$((SECONDS + general_max_secs))
  start_agent "$permission_json" "$log_file" "$model_id"

  while kill -0 "$AGENT_PID" 2>/dev/null; do
    current_signature="$(diff_signature)"
    if [[ "$current_signature" != "$last_signature" ]]; then
      last_signature="$current_signature"
      idle_deadline=$((SECONDS + general_idle_secs))
    fi

    tool_count="$(grep -c 'message=evaluated permission=' "$log_file" 2>/dev/null || true)"
    if (( tool_count > last_tool_count )); then
      last_tool_count=$tool_count
      idle_deadline=$((SECONDS + general_idle_secs))
    fi

    stop_reason=""
    stop_status=124
    if trip_backend_circuit_if_needed "$log_file"; then
      stop_reason="fatal backend/template error budget"
      stop_status=70
    elif (( tool_count >= general_max_tools )); then
      stop_reason="tool-action budget (${tool_count}/${general_max_tools})"
    elif (( SECONDS >= wall_deadline )); then
      stop_reason="total runtime budget (${general_max_secs}s)"
    elif (( SECONDS >= idle_deadline )); then
      stop_reason="idle budget (${general_idle_secs}s without tool action or diff change)"
    fi

    if [[ -n "$stop_reason" ]]; then
      printf '\\n===== GENERAL AGENT WATCHDOG =====\\n' >&2
      printf 'stopping agent: %s\\n' "$stop_reason" >&2
      stop_agent_group "$AGENT_PID"
      wait "$AGENT_PID" 2>/dev/null || true
      current_signature="$(diff_signature)"
      rm -f "$log_file"
      if [[ "$current_signature" != "$baseline_signature" ]]; then
        printf 'working-tree progress exists; preserving it for orchestrator validation\\n' >&2
        return 0
      fi
      return "$stop_status"
    fi
    sleep 2
  done

  wait "$AGENT_PID"
  status=$?
  rm -f "$log_file"
  return "$status"
}
'''
replace_once(old_general, new_general, "general watchdog function")

replace_once(
    '''else
  run_agent_unbounded
  status=$?
fi
''',
    '''else
  run_agent_with_general_watchdog
  status=$?
fi
''',
    "general watchdog call",
)

path.write_text(source)
