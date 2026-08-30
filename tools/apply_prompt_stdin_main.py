#!/usr/bin/env python3
from pathlib import Path

path = Path("src/main.rs")
text = path.read_text()


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return source.replace(old, new, 1)


text = replace_once(
    text,
    'use std::process::{Command, ExitCode};\n',
    'use std::process::{Command, ExitCode, Stdio};\n',
    "rust Stdio import",
)

text = replace_once(
    text,
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

path.write_text(text)
