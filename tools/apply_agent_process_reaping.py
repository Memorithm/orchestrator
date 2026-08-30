from pathlib import Path


path = Path("scripts/opencode")
text = path.read_text()
anchor = '''stop_agent_group() {
  local pid="$1"
  kill -INT -- "-$pid" 2>/dev/null || kill -INT "$pid" 2>/dev/null || true
  sleep 2
  if kill -0 "$pid" 2>/dev/null; then
    kill -TERM -- "-$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
  fi
  sleep 3
  if kill -0 "$pid" 2>/dev/null; then
    kill -KILL -- "-$pid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
  fi
}
'''
insert = anchor + '''
wait_active_agent() {
  local pid="${AGENT_PID:-}"
  local status

  if [[ -z "$pid" ]]; then
    return 0
  fi

  wait "$pid"
  status=$?
  if [[ "${AGENT_PID:-}" == "$pid" ]]; then
    AGENT_PID=""
  fi
  return "$status"
}

cleanup_active_agent() {
  local pid="${AGENT_PID:-}"

  if [[ -z "$pid" ]]; then
    return 0
  fi
  if kill -0 "$pid" 2>/dev/null; then
    stop_agent_group "$pid"
  fi
  wait "$pid" 2>/dev/null || true
  if [[ "${AGENT_PID:-}" == "$pid" ]]; then
    AGENT_PID=""
  fi
}

handle_wrapper_signal() {
  local status="$1"
  trap - INT TERM HUP EXIT
  cleanup_active_agent
  exit "$status"
}

trap 'handle_wrapper_signal 130' INT
trap 'handle_wrapper_signal 143' TERM
trap 'handle_wrapper_signal 129' HUP
trap 'cleanup_active_agent' EXIT
'''
if text.count(anchor) != 1:
    raise SystemExit("stop_agent_group anchor changed")
text = text.replace(anchor, insert, 1)

old_bounded = 'wait "$AGENT_PID" 2>/dev/null || true'
count = text.count(old_bounded)
if count != 4:
    raise SystemExit(f"expected 4 bounded agent waits, found {count}")
text = text.replace(old_bounded, 'wait_active_agent 2>/dev/null || true')

old_normal = '''  wait "$AGENT_PID"
  status=$?'''
count = text.count(old_normal)
if count != 2:
    raise SystemExit(f"expected 2 normal agent waits, found {count}")
text = text.replace(old_normal, '''  wait_active_agent
  status=$?''')
path.write_text(text)

unit = Path("scripts/render-systemd-unit.sh")
text = unit.read_text()
old = '''# The Rust parent receives SIGINT first. Any descendant still alive after the
# grace period is killed as part of this unit's cgroup, never by a global pkill.
KillMode=mixed
KillSignal=SIGINT'''
new = '''# Signal the entire service cgroup immediately. scripts/opencode traps the
# signal and explicitly reaps its detached setsid Ollama/OpenCode group; the
# final SIGKILL remains a bounded fallback rather than the normal stop path.
KillMode=control-group
KillSignal=SIGINT'''
if text.count(old) != 1:
    raise SystemExit("systemd kill-mode anchor changed")
unit.write_text(text.replace(old, new, 1))
