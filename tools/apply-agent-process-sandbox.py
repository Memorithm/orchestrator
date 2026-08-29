#!/usr/bin/env python3
from pathlib import Path

path = Path("scripts/opencode")
source = path.read_text()
old = '''  PATH="${ORCHESTRATOR_AGENT_PATH:?ORCHESTRATOR_AGENT_PATH is not set}" \\
  OPENCODE_PERMISSION="$effective_permission" \\
  OPENCODE_DISABLE_DEFAULT_PLUGINS=1 \\
  OPENCODE_DISABLE_CLAUDE_CODE=1 \\
  setsid ollama launch opencode --model "$effective_model_id" -- \\
    --pure --print-logs --log-level INFO "${args[@]}" \\
    > >(tee -a "$log_file") 2>&1 &
'''
new = '''  OPENCODE_PERMISSION="$effective_permission" \\
  setsid "${ORCHESTRATOR_AGENT_SANDBOX:?ORCHESTRATOR_AGENT_SANDBOX is not set}" -- \\
    ollama launch opencode --model "$effective_model_id" -- \\
    --pure --print-logs --log-level INFO "${args[@]}" \\
    > >(tee -a "$log_file") 2>&1 &
'''
count = source.count(old)
if count != 1:
    raise SystemExit(f"agent sandbox launch anchor: expected exactly one match, found {count}")
path.write_text(source.replace(old, new, 1))
