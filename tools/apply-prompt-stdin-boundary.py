#!/usr/bin/env python3
from pathlib import Path

main_path = Path("src/main.rs")
wrapper_path = Path("scripts/opencode")
selftest_path = Path("scripts/selftest-opencode-watchdog.sh")
main = main_path.read_text()
wrapper = wrapper_path.read_text()
selftest = selftest_path.read_text()


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return source.replace(old, new, 1)


main = replace_once(
    main,
    'use std::process::{Command, ExitCode};\n',
    'use std::process::{Command, ExitCode, Stdio};\n',
    "rust Stdio import",
)
main = replace_once(
    main,
    '''    let status = Command::new("opencode")
        .current_dir(workspace)
        .env("OPENCODE_CONFIG_CONTENT", OPENCODE_INLINE_CONFIG)
        .env("OPENCODE_DISABLE_AUTOUPDATE", "1")
        .args(["run", "--auto", "--model"])
        .arg(&config.model)
        .arg(prompt)
        .status()
        .map_err(|error| {
            ActionFailure::new(
                state::FailureClass::Infrastructure,
                format!("failed to execute opencode: {error}"),
            )
        })?;
''',
    '''    let mut child = Command::new("opencode")
        .current_dir(workspace)
        .env("OPENCODE_CONFIG_CONTENT", OPENCODE_INLINE_CONFIG)
        .env("OPENCODE_DISABLE_AUTOUPDATE", "1")
        .args(["run", "--auto", "--model"])
        .arg(&config.model)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| {
            ActionFailure::new(
                state::FailureClass::Infrastructure,
                format!("failed to execute opencode: {error}"),
            )
        })?;

    let write_error = match child.stdin.take() {
        Some(mut stdin) => stdin.write_all(prompt.as_bytes()).err(),
        None => Some(std::io::Error::other("opencode stdin pipe unavailable")),
    };
    let status = child.wait().map_err(|error| {
        ActionFailure::new(
            state::FailureClass::Infrastructure,
            format!("failed to wait for opencode: {error}"),
        )
    })?;
    if let Some(error) = write_error {
        return Err(ActionFailure::new(
            state::FailureClass::Infrastructure,
            format!("failed to stream prompt to opencode stdin: {error}; child status {status}"),
        ));
    }
''',
    "run_agent stdin transport",
)

wrapper = replace_once(
    wrapper,
    '''args=("$@")
last_index=$((${#args[@]} - 1))
is_ci_fix=0
if (( last_index >= 0 )) && [[ "${args[$last_index]}" == *"Task: FIX_CI"* ]]; then
  is_ci_fix=1
  args[$last_index]="${args[$last_index]}

CI repair hard requirement:
- A reproduced local compiler/lint/test failure supplied by Orchestrator is authoritative evidence for this repair pass.
- Do not query GitHub or inspect Git history when Orchestrator already reproduced the failure locally.
- Inspect only current files directly implicated by the reproduced error.
- Once the failing symbols/files are identified, edit immediately, then validate.
- Preserve any deterministic repair already left in the working tree.
- Continue until the reproduced local CI gates pass, or leave the narrowest safe partial fix with a precise blocker."
else
  args[$last_index]="${args[$last_index]}

AUTONOMOUS WORK BUDGET:
- This headless attempt has a hard budget of ${general_max_tools} tool actions and ${general_max_secs} seconds total runtime.
- More than ${general_idle_secs} seconds without a tool action or working-tree diff is treated as a stall.
- Do not spend the whole budget surveying history or adjacent code. Once enough evidence is gathered, implement the smallest coherent deliverable and validate it.
- As the budget gets low, stop expanding scope and leave the working tree in a validated, reviewable state."
fi
''',
    '''# The autonomous Rust parent uses one strict argv shape and transports the
# entire task prompt over stdin. This keeps issue bodies, CI evidence, and
# reproduced failures out of /proc/*/cmdline and avoids temporary prompt files.
if [[ "$#" -ne 4 || "${1:-}" != "run" || "${2:-}" != "--auto" || "${3:-}" != "--model" ]]; then
  printf 'orchestrator opencode wrapper: unsupported autonomous argv shape; prompt must arrive on stdin\\n' >&2
  exit 2
fi
if [[ -t 0 ]]; then
  printf 'orchestrator opencode wrapper: refusing TTY stdin; autonomous prompt must be piped\\n' >&2
  exit 2
fi
agent_prompt="$(cat)"
if [[ -z "$agent_prompt" ]]; then
  printf 'orchestrator opencode wrapper: empty autonomous prompt on stdin\\n' >&2
  exit 2
fi
args=("$@")
is_ci_fix=0
if [[ "$agent_prompt" == *"Task: FIX_CI"* ]]; then
  is_ci_fix=1
  agent_prompt="${agent_prompt}

CI repair hard requirement:
- A reproduced local compiler/lint/test failure supplied by Orchestrator is authoritative evidence for this repair pass.
- Do not query GitHub or inspect Git history when Orchestrator already reproduced the failure locally.
- Inspect only current files directly implicated by the reproduced error.
- Once the failing symbols/files are identified, edit immediately, then validate.
- Preserve any deterministic repair already left in the working tree.
- Continue until the reproduced local CI gates pass, or leave the narrowest safe partial fix with a precise blocker."
else
  agent_prompt="${agent_prompt}

AUTONOMOUS WORK BUDGET:
- This headless attempt has a hard budget of ${general_max_tools} tool actions and ${general_max_secs} seconds total runtime.
- More than ${general_idle_secs} seconds without a tool action or working-tree diff is treated as a stall.
- Do not spend the whole budget surveying history or adjacent code. Once enough evidence is gathered, implement the smallest coherent deliverable and validate it.
- As the budget gets low, stop expanding scope and leave the working tree in a validated, reviewable state."
fi
''',
    "wrapper private prompt state",
)
wrapper = replace_once(
    wrapper,
    '''  args[$last_index]="${args[$last_index]}

Orchestrator reproduced the next failing local Rust CI gate. The failure below is authoritative. Do not query GitHub or inspect Git history. Inspect only the current files directly implicated by the error, make the narrowest correction, then run the CI-matched validation commands.

Reproduced local CI failure (tail, up to 16000 bytes):
${failure_log}"
''',
    '''  agent_prompt="${agent_prompt}

Orchestrator reproduced the next failing local Rust CI gate. The failure below is authoritative. Do not query GitHub or inspect Git history. Inspect only the current files directly implicated by the error, make the narrowest correction, then run the CI-matched validation commands.

Reproduced local CI failure (tail, up to 16000 bytes):
${failure_log}"
''',
    "append local failure to private prompt",
)
wrapper = replace_once(
    wrapper,
    '''  setsid ollama launch opencode --model "$effective_model_id" -- \\
    --pure --print-logs --log-level INFO "${args[@]}" \\
    > >(tee -a "$log_file") 2>&1 &
''',
    '''  setsid ollama launch opencode --model "$effective_model_id" -- \\
    --pure --print-logs --log-level INFO "${args[@]}" \\
    < <(printf '%s' "$agent_prompt") \\
    > >(tee -a "$log_file") 2>&1 &
''',
    "agent stdin redirection",
)
wrapper = replace_once(
    wrapper,
    '''  args[$last_index]="${args[$last_index]}

SURGICAL RETRY MODE:
- You are the local fallback repair agent because the primary model inspected the failure without editing it.
- Remote GitHub queries and Git-history archaeology are technically blocked for this pass.
- The reproduced local CI failure already identifies the defect class.
- Read only the current files directly implicated by that failure.
- Make the smallest correct working-tree edit immediately, then validate."
''',
    '''  agent_prompt="${agent_prompt}

SURGICAL RETRY MODE:
- You are the local fallback repair agent because the primary model inspected the failure without editing it.
- Remote GitHub queries and Git-history archaeology are technically blocked for this pass.
- The reproduced local CI failure already identifies the defect class.
- Read only the current files directly implicated by that failure.
- Make the smallest correct working-tree edit immediately, then validate."
''',
    "surgical private prompt",
)
wrapper = replace_once(
    wrapper,
    '''  args[$last_index]="${args[$last_index]}

FOLLOW-UP REPAIR MODE:
- Your previous edits remain in the working tree.
- Do not restart investigation, query GitHub, or inspect Git history.
- Repair only the residual local failure below and preserve correct prior changes."
''',
    '''  agent_prompt="${agent_prompt}

FOLLOW-UP REPAIR MODE:
- Your previous edits remain in the working tree.
- Do not restart investigation, query GitHub, or inspect Git history.
- Repair only the residual local failure below and preserve correct prior changes."
''',
    "follow-up private prompt",
)

selftest = replace_once(
    selftest,
    '''if [[ "${FAKE_AGENT_EDIT:-0}" == "1" ]]; then
  printf 'changed\\n' >> "${FAKE_AGENT_EDIT_FILE:?}"
fi

for _ in 1 2 3 4 5 6; do
''',
    '''if [[ "$*" == *"WATCHDOG_STDIN_SECRET"* ]]; then
  printf 'prompt secret leaked into fake ollama argv\\n' >&2
  exit 91
fi
stdin_payload="$(cat)"
if [[ "$stdin_payload" != *"WATCHDOG_STDIN_SECRET"* ]]; then
  printf 'prompt secret missing from fake ollama stdin\\n' >&2
  exit 92
fi

if [[ "${FAKE_AGENT_EDIT:-0}" == "1" ]]; then
  printf 'changed\\n' >> "${FAKE_AGENT_EDIT_FILE:?}"
fi

for _ in 1 2 3 4 5 6; do
''',
    "fake ollama stdin assertion",
)
selftest = replace_once(
    selftest,
    '''prompt='Repository: Memorithm/test
Task: ISSUE
Title: watchdog selftest'
''',
    '''prompt='Repository: Memorithm/test
Task: ISSUE
Title: watchdog selftest
Marker: WATCHDOG_STDIN_SECRET'
''',
    "selftest secret marker",
)
selftest = replace_once(
    selftest,
    '''FAKE_AGENT_EDIT=0 bash "$ROOT/scripts/opencode" run --auto --model ollama/qwen3.8:latest "$prompt" >/tmp/orchestrator-watchdog-no-edit.log 2>&1
''',
    '''printf '%s' "$prompt" | FAKE_AGENT_EDIT=0 bash "$ROOT/scripts/opencode" run --auto --model ollama/qwen3.8:latest >/tmp/orchestrator-watchdog-no-edit.log 2>&1
''',
    "no-edit stdin invocation",
)
selftest = replace_once(
    selftest,
    '''FAKE_AGENT_EDIT=1 FAKE_AGENT_EDIT_FILE="$REPO/tracked.txt" \\
  bash "$ROOT/scripts/opencode" run --auto --model ollama/qwen3.8:latest "$prompt" >/tmp/orchestrator-watchdog-edit.log 2>&1
''',
    '''printf '%s' "$prompt" | FAKE_AGENT_EDIT=1 FAKE_AGENT_EDIT_FILE="$REPO/tracked.txt" \\
  bash "$ROOT/scripts/opencode" run --auto --model ollama/qwen3.8:latest >/tmp/orchestrator-watchdog-edit.log 2>&1
''',
    "edited stdin invocation",
)

main_path.write_text(main)
wrapper_path.write_text(wrapper)
selftest_path.write_text(selftest)
