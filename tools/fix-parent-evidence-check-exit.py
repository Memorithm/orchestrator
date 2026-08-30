#!/usr/bin/env python3
from pathlib import Path

path = Path("src/evidence.rs")
source = path.read_text()

old = '''    let checks = capture_gh(&[
        "pr",
        "checks",
'''
new = '''    let checks = capture_pr_checks(&[
        "pr",
        "checks",
'''
if source.count(old) != 1:
    raise SystemExit(f"checks capture anchor: expected 1, found {source.count(old)}")
source = source.replace(old, new, 1)

anchor = '''fn capture_gh(args: &[&str]) -> Result<String, String> {
'''
helper = '''fn capture_pr_checks(args: &[&str]) -> Result<String, String> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .map_err(|error| format!("failed to execute gh: {error}"))?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("invalid UTF-8 from gh pr checks: {error}"))?;
    if !stdout.trim().is_empty() || output.status.success() {
        return Ok(stdout.trim().to_owned());
    }
    let stderr = sanitize_inline(&String::from_utf8_lossy(&output.stderr));
    if stderr.contains("no checks reported") {
        Ok(String::new())
    } else {
        Err(format!("gh {} failed: {stderr}", args.join(" ")))
    }
}

fn capture_gh(args: &[&str]) -> Result<String, String> {
'''
if source.count(anchor) != 1:
    raise SystemExit(f"capture helper anchor: expected 1, found {source.count(anchor)}")
source = source.replace(anchor, helper, 1)
path.write_text(source)
