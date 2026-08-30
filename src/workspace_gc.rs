use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};

const USAGE_VERSION: &str = "v1";
const MAX_GIT_REF_SCAN_ENTRIES: usize = 10_000;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkspaceGcReport {
    pub(crate) scanned: usize,
    pub(crate) removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceUsage {
    repository: String,
    last_used: u64,
}

pub(crate) fn record_workspace_use(
    data_root: &Path,
    repository: &str,
    timestamp: u64,
) -> Result<(), String> {
    validate_repository(repository)?;
    let root = usage_root(data_root);
    fs::create_dir_all(&root)
        .map_err(|error| format!("failed to create workspace usage root {}: {error}", root.display()))?;

    let path = usage_path(data_root, repository);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = root.join(format!(
        ".{}.{}.{sequence}.tmp",
        repository_component(repository),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| format!("failed to create workspace usage temp {}: {error}", temp.display()))?;
    let contents = format!(
        "{USAGE_VERSION}\nrepository={repository}\nlast_used={timestamp}\n"
    );
    if let Err(error) = file.write_all(contents.as_bytes()).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(format!(
            "failed to persist workspace usage temp {}: {error}",
            temp.display()
        ));
    }
    drop(file);

    if let Err(error) = fs::rename(&temp, &path) {
        let _ = fs::remove_file(&temp);
        return Err(format!(
            "failed to publish workspace usage {}: {error}",
            path.display()
        ));
    }
    File::open(&root)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("failed to sync workspace usage root {}: {error}", root.display()))?;
    Ok(())
}

pub(crate) fn reclaim_stale_workspaces(
    data_root: &Path,
    current_repository: &str,
    now: u64,
    min_idle_secs: u64,
    max_workspaces: usize,
) -> Result<WorkspaceGcReport, String> {
    if min_idle_secs == 0 || max_workspaces == 0 {
        return Ok(WorkspaceGcReport {
            scanned: 0,
            removed: 0,
        });
    }

    let root = usage_root(data_root);
    if !root.exists() {
        return Ok(WorkspaceGcReport {
            scanned: 0,
            removed: 0,
        });
    }

    let mut markers = fs::read_dir(&root)
        .map_err(|error| format!("failed to read workspace usage root {}: {error}", root.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let kind = entry.file_type().ok()?;
            if !kind.is_file() || kind.is_symlink() {
                return None;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("used") {
                return None;
            }
            Some(path)
        })
        .collect::<Vec<_>>();
    markers.sort();

    let mut candidates = Vec::new();
    let mut scanned = 0usize;
    for marker in markers {
        scanned = scanned.saturating_add(1);
        let Ok(contents) = fs::read_to_string(&marker) else {
            continue;
        };
        let Ok(usage) = parse_usage(&contents) else {
            continue;
        };
        if usage.repository == current_repository || usage.last_used > now {
            continue;
        }
        if now.saturating_sub(usage.last_used) < min_idle_secs {
            continue;
        }
        if marker != usage_path(data_root, &usage.repository) {
            continue;
        }
        candidates.push((usage.last_used, usage.repository, marker));
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));

    let mut removed = 0usize;
    for (_, repository, marker) in candidates {
        if removed >= max_workspaces {
            break;
        }
        let workspace = workspace_path(data_root, &repository);
        match workspace_is_disposable(&workspace, &repository) {
            Ok(true) => {}
            Ok(false) | Err(_) => continue,
        }

        fs::remove_dir_all(&workspace).map_err(|error| {
            format!(
                "failed to remove verified stale workspace {}: {error}",
                workspace.display()
            )
        })?;
        removed = removed.saturating_add(1);
        if let Err(error) = fs::remove_file(&marker) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(format!(
                    "removed workspace {} but failed to remove usage marker {}: {error}",
                    workspace.display(),
                    marker.display()
                ));
            }
        }
    }

    Ok(WorkspaceGcReport { scanned, removed })
}

fn workspace_is_disposable(workspace: &Path, repository: &str) -> Result<bool, String> {
    if !real_directory(workspace) {
        return Ok(false);
    }
    let git_dir = workspace.join(".git");
    if !real_directory(&git_dir) {
        return Ok(false);
    }
    if workspace.join(".gitmodules").exists()
        || git_dir.join("worktrees").exists()
        || git_dir.join("modules").exists()
    {
        return Ok(false);
    }
    if git_is_busy(&git_dir)? {
        return Ok(false);
    }

    let origin = git_stdout(workspace, &["remote", "get-url", "origin"])?;
    if !github_remote_matches_repository(&origin, repository) {
        return Ok(false);
    }

    let head = git_status(workspace, &["symbolic-ref", "-q", "HEAD"])?;
    if !head.success() {
        return Ok(false);
    }

    let status = git_stdout(
        workspace,
        &[
            "status",
            "--porcelain=v1",
            "--ignored=matching",
            "--untracked-files=all",
        ],
    )?;
    if !status.is_empty() {
        return Ok(false);
    }

    let tags = git_stdout(workspace, &["for-each-ref", "--format=%(refname)", "refs/tags"])?;
    if !tags.is_empty() {
        return Ok(false);
    }

    let stash = git_status(workspace, &["show-ref", "--verify", "--quiet", "refs/stash"])?;
    match stash.code() {
        Some(0) => return Ok(false),
        Some(1) => {}
        _ => return Ok(false),
    }

    let branches = git_stdout(
        workspace,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(objectname)",
            "refs/heads",
        ],
    )?;
    if branches.is_empty() {
        return Ok(false);
    }
    for line in branches.lines() {
        let Some((branch, local_sha)) = line.split_once('\t') else {
            return Ok(false);
        };
        if branch.is_empty() || local_sha.is_empty() {
            return Ok(false);
        }
        let remote_ref = format!("refs/remotes/origin/{branch}");
        let remote = git_stdout(workspace, &["rev-parse", "--verify", &remote_ref]);
        let Ok(remote_sha) = remote else {
            return Ok(false);
        };
        if remote_sha.is_empty() {
            return Ok(false);
        }
        let ancestor = git_status(
            workspace,
            &["merge-base", "--is-ancestor", local_sha, &remote_sha],
        )?;
        if !ancestor.success() {
            return Ok(false);
        }
    }

    if git_is_busy(&git_dir)? {
        return Ok(false);
    }
    Ok(true)
}

fn git_is_busy(git_dir: &Path) -> Result<bool, String> {
    for relative in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "REBASE_HEAD",
        "BISECT_LOG",
        "AUTO_MERGE",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
        "index.lock",
        "HEAD.lock",
        "config.lock",
        "packed-refs.lock",
        "shallow.lock",
    ] {
        if git_dir.join(relative).exists() {
            return Ok(true);
        }
    }
    contains_lock_file(&git_dir.join("refs"))
}

fn contains_lock_file(root: &Path) -> Result<bool, String> {
    if !root.exists() {
        return Ok(false);
    }
    if fs::symlink_metadata(root)
        .map_err(|error| format!("failed to inspect {}: {error}", root.display()))?
        .file_type()
        .is_symlink()
    {
        return Ok(true);
    }

    let mut pending = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("failed to inspect git refs {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| {
                format!("failed to inspect git refs entry in {}: {error}", directory.display())
            })?;
            visited = visited.saturating_add(1);
            if visited > MAX_GIT_REF_SCAN_ENTRIES {
                return Err("git refs scan exceeded safety bound".to_owned());
            }
            let kind = entry
                .file_type()
                .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
            if kind.is_symlink() {
                return Ok(true);
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if kind.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(".lock"))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn git_stdout(workspace: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(args)
        .output()
        .map_err(|error| format!("failed to run git in {}: {error}", workspace.display()))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed in {} with {}: {}",
            args.join(" "),
            workspace.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("invalid UTF-8 from git in {}: {error}", workspace.display()))
}

fn git_status(workspace: &Path, args: &[&str]) -> Result<ExitStatus, String> {
    Command::new("git")
        .current_dir(workspace)
        .args(args)
        .status()
        .map_err(|error| format!("failed to run git in {}: {error}", workspace.display()))
}

fn parse_usage(contents: &str) -> Result<WorkspaceUsage, String> {
    let mut lines = contents.lines();
    if lines.next() != Some(USAGE_VERSION) {
        return Err("unsupported workspace usage version".to_owned());
    }
    let repository = lines
        .next()
        .and_then(|line| line.strip_prefix("repository="))
        .ok_or_else(|| "workspace usage missing repository".to_owned())?;
    validate_repository(repository)?;
    let last_used = lines
        .next()
        .and_then(|line| line.strip_prefix("last_used="))
        .ok_or_else(|| "workspace usage missing last_used".to_owned())?
        .parse::<u64>()
        .map_err(|error| format!("invalid workspace last_used: {error}"))?;
    if lines.any(|line| !line.is_empty()) {
        return Err("workspace usage has unexpected fields".to_owned());
    }
    Ok(WorkspaceUsage {
        repository: repository.to_owned(),
        last_used,
    })
}

fn validate_repository(repository: &str) -> Result<(), String> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(format!("invalid repository identity: {repository:?}"));
    }
    for component in [owner, name] {
        if component == "."
            || component == ".."
            || !component
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
        {
            return Err(format!("unsafe repository identity: {repository:?}"));
        }
    }
    Ok(())
}

fn usage_root(data_root: &Path) -> PathBuf {
    data_root.join("state/workspaces")
}

fn usage_path(data_root: &Path, repository: &str) -> PathBuf {
    usage_root(data_root).join(format!("{}.used", repository_component(repository)))
}

fn workspace_path(data_root: &Path, repository: &str) -> PathBuf {
    data_root
        .join("workspaces")
        .join(repository_component(repository))
}

fn repository_component(repository: &str) -> String {
    repository.replace('/', "__")
}

fn real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn github_remote_matches_repository(remote: &str, repository: &str) -> bool {
    let remote = remote.trim().trim_end_matches('/');
    let repository = repository.trim().trim_end_matches('/');
    [
        format!("https://github.com/{repository}"),
        format!("https://github.com/{repository}.git"),
        format!("git@github.com:{repository}"),
        format!("git@github.com:{repository}.git"),
        format!("ssh://git@github.com/{repository}"),
        format!("ssh://git@github.com/{repository}.git"),
    ]
    .iter()
    .any(|expected| remote.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "orchestrator-workspace-gc-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn git(workspace: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(workspace)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    fn managed_workspace(root: &Path, repository: &str) -> PathBuf {
        let workspace = workspace_path(root, repository);
        fs::create_dir_all(&workspace).unwrap();
        git(&workspace, &["init", "-b", "main"]);
        git(&workspace, &["config", "user.name", "orchestrator-test"]);
        git(
            &workspace,
            &["config", "user.email", "orchestrator-test@example.invalid"],
        );
        fs::write(workspace.join("tracked.txt"), b"tracked\n").unwrap();
        git(&workspace, &["add", "tracked.txt"]);
        git(&workspace, &["commit", "-m", "initial"]);
        let origin = format!("https://github.com/{repository}.git");
        git(&workspace, &["remote", "add", "origin", &origin]);
        let head = git_stdout(&workspace, &["rev-parse", "HEAD"]).unwrap();
        git(
            &workspace,
            &["update-ref", "refs/remotes/origin/main", &head],
        );
        workspace
    }

    #[test]
    fn usage_round_trips_strictly() {
        let usage = parse_usage("v1\nrepository=Memorithm/ADA\nlast_used=123\n").unwrap();
        assert_eq!(usage.repository, "Memorithm/ADA");
        assert_eq!(usage.last_used, 123);
        assert!(parse_usage("v2\nrepository=Memorithm/ADA\nlast_used=123\n").is_err());
        assert!(parse_usage("v1\nrepository=../ADA\nlast_used=123\n").is_err());
        assert!(parse_usage("v1\nrepository=Memorithm/ADA\nlast_used=x\n").is_err());
        assert!(parse_usage(
            "v1\nrepository=Memorithm/ADA\nlast_used=123\nextra=true\n"
        )
        .is_err());
    }

    #[test]
    fn usage_record_is_atomic_and_replaceable() {
        let root = temporary_root("usage");
        record_workspace_use(&root, "Memorithm/ADA", 10).unwrap();
        record_workspace_use(&root, "Memorithm/ADA", 20).unwrap();
        let contents = fs::read_to_string(usage_path(&root, "Memorithm/ADA")).unwrap();
        assert_eq!(parse_usage(&contents).unwrap().last_used, 20);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_clean_remote_backed_workspace_is_removed() {
        let root = temporary_root("remove");
        let workspace = managed_workspace(&root, "Memorithm/ADA");
        record_workspace_use(&root, "Memorithm/ADA", 100).unwrap();
        let report = reclaim_stale_workspaces(
            &root,
            "Memorithm/Current",
            700_000,
            604_800,
            1,
        )
        .unwrap();
        assert_eq!(report.removed, 1);
        assert!(!workspace.exists());
        assert!(!usage_path(&root, "Memorithm/ADA").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn current_and_recent_workspaces_are_preserved() {
        let root = temporary_root("fresh");
        let current = managed_workspace(&root, "Memorithm/Current");
        let recent = managed_workspace(&root, "Memorithm/Recent");
        record_workspace_use(&root, "Memorithm/Current", 1).unwrap();
        record_workspace_use(&root, "Memorithm/Recent", 699_900).unwrap();
        let report = reclaim_stale_workspaces(
            &root,
            "Memorithm/Current",
            700_000,
            604_800,
            1,
        )
        .unwrap();
        assert_eq!(report.removed, 0);
        assert!(current.exists());
        assert!(recent.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dirty_or_unpushed_workspace_is_preserved() {
        let root = temporary_root("dirty");
        let dirty = managed_workspace(&root, "Memorithm/Dirty");
        fs::write(dirty.join("tracked.txt"), b"changed\n").unwrap();
        record_workspace_use(&root, "Memorithm/Dirty", 1).unwrap();
        let report = reclaim_stale_workspaces(
            &root,
            "Memorithm/Current",
            700_000,
            604_800,
            1,
        )
        .unwrap();
        assert_eq!(report.removed, 0);
        assert!(dirty.exists());

        fs::write(dirty.join("tracked.txt"), b"tracked\n").unwrap();
        git(&dirty, &["add", "tracked.txt"]);
        git(&dirty, &["commit", "-m", "local-only"]);
        let report = reclaim_stale_workspaces(
            &root,
            "Memorithm/Current",
            700_000,
            604_800,
            1,
        )
        .unwrap();
        assert_eq!(report.removed, 0);
        assert!(dirty.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ignored_file_stash_or_tag_preserves_workspace() {
        let root = temporary_root("local-state");
        let workspace = managed_workspace(&root, "Memorithm/LocalState");
        fs::write(workspace.join(".gitignore"), b"private.bin\n").unwrap();
        git(&workspace, &["add", ".gitignore"]);
        git(&workspace, &["commit", "-m", "ignore"]);
        let head = git_stdout(&workspace, &["rev-parse", "HEAD"]).unwrap();
        git(
            &workspace,
            &["update-ref", "refs/remotes/origin/main", &head],
        );
        fs::write(workspace.join("private.bin"), b"local\n").unwrap();
        record_workspace_use(&root, "Memorithm/LocalState", 1).unwrap();
        assert_eq!(
            reclaim_stale_workspaces(
                &root,
                "Memorithm/Current",
                700_000,
                604_800,
                1,
            )
            .unwrap()
            .removed,
            0
        );
        assert!(workspace.exists());
        fs::remove_file(workspace.join("private.bin")).unwrap();

        fs::write(workspace.join("tracked.txt"), b"stash me\n").unwrap();
        git(&workspace, &["stash", "push", "-m", "local-stash"]);
        assert_eq!(
            reclaim_stale_workspaces(
                &root,
                "Memorithm/Current",
                700_000,
                604_800,
                1,
            )
            .unwrap()
            .removed,
            0
        );
        git(&workspace, &["stash", "drop"]);

        git(&workspace, &["tag", "local-tag"]);
        assert_eq!(
            reclaim_stale_workspaces(
                &root,
                "Memorithm/Current",
                700_000,
                604_800,
                1,
            )
            .unwrap()
            .removed,
            0
        );
        assert!(workspace.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn git_operation_marker_preserves_workspace() {
        let root = temporary_root("operation");
        let workspace = managed_workspace(&root, "Memorithm/Operation");
        fs::write(workspace.join(".git/index.lock"), b"").unwrap();
        assert!(git_is_busy(&workspace.join(".git")).unwrap());
        fs::remove_file(workspace.join(".git/index.lock")).unwrap();
        fs::create_dir_all(workspace.join(".git/rebase-merge")).unwrap();
        assert!(git_is_busy(&workspace.join(".git")).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_symlink_is_never_followed() {
        use std::os::unix::fs::symlink;

        let root = temporary_root("symlink");
        let outside = temporary_root("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), b"safe").unwrap();
        fs::create_dir_all(root.join("workspaces")).unwrap();
        symlink(&outside, workspace_path(&root, "Memorithm/Linked")).unwrap();
        record_workspace_use(&root, "Memorithm/Linked", 1).unwrap();
        let report = reclaim_stale_workspaces(
            &root,
            "Memorithm/Current",
            700_000,
            604_800,
            1,
        )
        .unwrap();
        assert_eq!(report.removed, 0);
        assert!(outside.join("keep").exists());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn cap_removes_only_oldest_bounded_candidate() {
        let root = temporary_root("cap");
        let oldest = managed_workspace(&root, "Memorithm/Oldest");
        let newer = managed_workspace(&root, "Memorithm/Newer");
        record_workspace_use(&root, "Memorithm/Oldest", 1).unwrap();
        record_workspace_use(&root, "Memorithm/Newer", 2).unwrap();
        let report = reclaim_stale_workspaces(
            &root,
            "Memorithm/Current",
            700_000,
            604_800,
            1,
        )
        .unwrap();
        assert_eq!(report.removed, 1);
        assert!(!oldest.exists());
        assert!(newer.exists());
        let _ = fs::remove_dir_all(root);
    }
}
