use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReclaimReport {
    pub(crate) scanned: usize,
    pub(crate) removed: usize,
}

pub(crate) fn reclaim_workspace_targets(
    data_root: &Path,
    max_targets: usize,
) -> Result<ReclaimReport, String> {
    if max_targets == 0 {
        return Ok(ReclaimReport {
            scanned: 0,
            removed: 0,
        });
    }

    let root = data_root.join("workspaces");
    if !root.exists() {
        return Ok(ReclaimReport {
            scanned: 0,
            removed: 0,
        });
    }

    let mut workspaces = fs::read_dir(&root)
        .map_err(|error| format!("failed to read workspace root {}: {error}", root.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let kind = entry.file_type().ok()?;
            if !kind.is_dir() || kind.is_symlink() {
                return None;
            }
            Some(entry.path())
        })
        .collect::<Vec<_>>();
    workspaces.sort();

    let mut report = ReclaimReport {
        scanned: 0,
        removed: 0,
    };

    for workspace in workspaces {
        if report.removed >= max_targets {
            break;
        }
        report.scanned = report.scanned.saturating_add(1);

        let Some(repository) = repository_from_workspace(&workspace) else {
            continue;
        };
        if !real_directory(&workspace.join(".git")) {
            continue;
        }

        let origin = match git_origin(&workspace) {
            Ok(origin) => origin,
            Err(_) => continue,
        };
        if reclaim_verified_target(&workspace, &repository, &origin)? {
            report.removed = report.removed.saturating_add(1);
        }
    }

    Ok(report)
}

fn repository_from_workspace(workspace: &Path) -> Option<String> {
    let name = workspace.file_name()?.to_str()?;
    let (owner, repository) = name.split_once("__")?;
    if owner.is_empty() || repository.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repository}"))
}

fn git_origin(workspace: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|error| {
            format!(
                "failed to inspect git origin in {}: {error}",
                workspace.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "git remote get-url origin failed in {} with {}",
            workspace.display(),
            output.status
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| {
            format!(
                "invalid UTF-8 git origin in {}: {error}",
                workspace.display()
            )
        })
}

fn reclaim_verified_target(
    workspace: &Path,
    repository: &str,
    origin: &str,
) -> Result<bool, String> {
    if !github_remote_matches_repository(origin, repository) {
        return Ok(false);
    }

    let target = workspace.join("target");
    if !real_directory(&target) {
        return Ok(false);
    }

    fs::remove_dir_all(&target).map_err(|error| {
        format!(
            "failed to remove managed build cache {}: {error}",
            target.display()
        )
    })?;
    Ok(true)
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
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "orchestrator-reclaim-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    #[test]
    fn workspace_name_decodes_repository_identity() {
        let path = Path::new("/tmp/workspaces/Memorithm__ElasticXxx");
        assert_eq!(
            repository_from_workspace(path).as_deref(),
            Some("Memorithm/ElasticXxx")
        );
        assert!(repository_from_workspace(Path::new("/tmp/workspaces/broken")).is_none());
    }

    #[test]
    fn github_origin_matching_is_exact() {
        assert!(github_remote_matches_repository(
            "https://github.com/Memorithm/ADA.git",
            "Memorithm/ADA"
        ));
        assert!(github_remote_matches_repository(
            "git@github.com:Memorithm/ADA.git",
            "Memorithm/ADA"
        ));
        assert!(!github_remote_matches_repository(
            "https://github.com/other/ADA.git",
            "Memorithm/ADA"
        ));
        assert!(!github_remote_matches_repository(
            "https://github.com/Memorithm/ADA-evil.git",
            "Memorithm/ADA"
        ));
    }

    #[test]
    fn verified_workspace_target_is_removed() {
        let root = temporary_root("remove");
        let workspace = root.join("Memorithm__ADA");
        fs::create_dir_all(workspace.join(".git")).unwrap();
        fs::create_dir_all(workspace.join("target/debug")).unwrap();
        fs::write(workspace.join("target/debug/artifact"), b"x").unwrap();

        assert!(
            reclaim_verified_target(
                &workspace,
                "Memorithm/ADA",
                "https://github.com/Memorithm/ADA.git"
            )
            .unwrap()
        );
        assert!(!workspace.join("target").exists());
        assert!(workspace.join(".git").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn target_symlink_is_never_followed_or_removed() {
        use std::os::unix::fs::symlink;

        let root = temporary_root("symlink");
        let workspace = root.join("Memorithm__ADA");
        let outside = root.join("outside");
        fs::create_dir_all(workspace.join(".git")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), b"safe").unwrap();
        symlink(&outside, workspace.join("target")).unwrap();

        assert!(
            !reclaim_verified_target(
                &workspace,
                "Memorithm/ADA",
                "https://github.com/Memorithm/ADA.git"
            )
            .unwrap()
        );
        assert!(workspace.join("target").exists());
        assert!(outside.join("keep").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn foreign_origin_is_not_cleaned() {
        let root = temporary_root("foreign");
        let workspace = root.join("Memorithm__ADA");
        fs::create_dir_all(workspace.join(".git")).unwrap();
        fs::create_dir_all(workspace.join("target")).unwrap();

        assert!(
            !reclaim_verified_target(
                &workspace,
                "Memorithm/ADA",
                "https://github.com/attacker/ADA.git"
            )
            .unwrap()
        );
        assert!(workspace.join("target").exists());
        let _ = fs::remove_dir_all(root);
    }
}
