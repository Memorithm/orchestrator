from pathlib import Path

path = Path("src/main.rs")
text = path.read_text()

old = '''fn has_changes(workspace: &Path) -> Result<bool, String> {
    Ok(
        !capture_in_dir(workspace, "git", &["status", "--porcelain"])?
            .trim()
            .is_empty(),
    )
}
'''
new = '''fn merge_in_progress(workspace: &Path) -> Result<bool, String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .output()
        .map_err(|error| format!("failed to inspect MERGE_HEAD in {}: {error}", workspace.display()))?;

    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }

    Err(format!(
        "git rev-parse MERGE_HEAD failed in {} with {}: {}",
        workspace.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn has_changes(workspace: &Path) -> Result<bool, String> {
    let porcelain = capture_in_dir(workspace, "git", &["status", "--porcelain"])?;
    if !porcelain.trim().is_empty() {
        return Ok(true);
    }
    merge_in_progress(workspace)
}
'''
if text.count(old) != 1:
    raise SystemExit(f"expected exactly one has_changes anchor, got {text.count(old)}")
text = text.replace(old, new)

anchor = '''#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_runtime_contains_local_agent_stack() {
'''
insert = '''#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_merge_state_is_publishable_even_without_porcelain_changes() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = env::temp_dir().join(format!(
            "orchestrator-merge-head-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&repository).unwrap();

        let git = |args: &[&str]| {
            let status = Command::new("git")
                .current_dir(&repository)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {} failed with {status}", args.join(" "));
        };

        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "orchestrator-test"]);
        git(&["config", "user.email", "orchestrator-test@example.invalid"]);
        fs::write(repository.join("tracked.txt"), "base\n").unwrap();
        git(&["add", "tracked.txt"]);
        git(&["commit", "-q", "-m", "base"]);
        git(&["checkout", "-q", "-b", "side"]);
        git(&["commit", "-q", "--allow-empty", "-m", "side"]);
        git(&["checkout", "-q", "main"]);
        git(&["commit", "-q", "--allow-empty", "-m", "main"]);

        assert!(!has_changes(&repository).unwrap());
        git(&["merge", "--no-commit", "--no-ff", "side"]);
        let porcelain = capture_in_dir(&repository, "git", &["status", "--porcelain"]).unwrap();
        assert!(porcelain.trim().is_empty());
        assert!(merge_in_progress(&repository).unwrap());
        assert!(has_changes(&repository).unwrap());

        fs::remove_dir_all(repository).unwrap();
    }

    #[test]
    fn required_runtime_contains_local_agent_stack() {
'''
if text.count(anchor) != 1:
    raise SystemExit(f"expected exactly one tests anchor, got {text.count(anchor)}")
text = text.replace(anchor, insert)
path.write_text(text)
