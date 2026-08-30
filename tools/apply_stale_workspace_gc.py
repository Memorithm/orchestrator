from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if text.count(old) != 1:
        raise SystemExit(f"expected exactly one match in {path}: {old!r}")
    file.write_text(text.replace(old, new, 1))


main = Path("src/main.rs")
text = main.read_text()

if text.count("mod workspace_gc;") != 0:
    raise SystemExit("workspace_gc module already wired")
text = text.replace("mod trajectory;\n", "mod trajectory;\nmod workspace_gc;\n", 1)

old_fields = (
    "    resource_policy: resource::ResourcePolicy,\n"
    "    low_disk_reclaim_max_targets: usize,\n"
    "    trajectory_max_per_item: usize,"
)
new_fields = (
    "    resource_policy: resource::ResourcePolicy,\n"
    "    low_disk_reclaim_max_targets: usize,\n"
    "    low_disk_reclaim_max_workspaces: usize,\n"
    "    workspace_min_idle_secs: u64,\n"
    "    trajectory_max_per_item: usize,"
)
if text.count(old_fields) != 1:
    raise SystemExit("RunConfig resource fields anchor changed")
text = text.replace(old_fields, new_fields, 1)

old_config = '''            low_disk_reclaim_max_targets: usize::try_from(env_u64(
                "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS",
                4,
            ))
            .map_err(|_| {
                "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS does not fit usize".to_owned()
            })?,
            trajectory_max_per_item: trajectory::max_files_per_item_from_env()?,'''
new_config = '''            low_disk_reclaim_max_targets: usize::try_from(env_u64(
                "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS",
                4,
            ))
            .map_err(|_| {
                "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS does not fit usize".to_owned()
            })?,
            low_disk_reclaim_max_workspaces: usize::try_from(env_u64(
                "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES",
                1,
            ))
            .map_err(|_| {
                "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES does not fit usize".to_owned()
            })?,
            workspace_min_idle_secs: env_u64("ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS", 604_800),
            trajectory_max_per_item: trajectory::max_files_per_item_from_env()?,'''
if text.count(old_config) != 1:
    raise SystemExit("RunConfig initialization anchor changed")
text = text.replace(old_config, new_config, 1)

old_log = '''    println!(
        "disk reclaim cap : {} workspace targets per pressure event (0=disabled)",
        config.low_disk_reclaim_max_targets
    );
    println!(
        "trajectory keep  : {} per work item (0=unlimited)",'''
new_log = '''    println!(
        "disk reclaim cap : {} workspace targets per pressure event (0=disabled)",
        config.low_disk_reclaim_max_targets
    );
    println!(
        "workspace GC cap : {} stale clones per pressure event (0=disabled)",
        config.low_disk_reclaim_max_workspaces
    );
    println!(
        "workspace idle   : {}s minimum before GC (0=disabled)",
        config.workspace_min_idle_secs
    );
    println!(
        "trajectory keep  : {} per work item (0=unlimited)",'''
if text.count(old_log) != 1:
    raise SystemExit("runtime resource log anchor changed")
text = text.replace(old_log, new_log, 1)

ensure_start = text.index("fn ensure_clone(config: &RunConfig, repository: &str) -> Result<PathBuf, String> {")
ensure_end = text.index("\nfn clean_and_fetch", ensure_start)
ensure = text[ensure_start:ensure_end]
existing_anchor = "        return Ok(workspace);\n    }\n\n    if workspace.exists() {"
if ensure.count(existing_anchor) != 1:
    raise SystemExit("existing workspace return anchor changed")
ensure = ensure.replace(
    existing_anchor,
    '''        workspace_gc::record_workspace_use(
            &config.data_root,
            repository,
            unix_timestamp(),
        )?;
        return Ok(workspace);
    }

    if workspace.exists() {''',
    1,
)
new_clone_anchor = "    Ok(workspace)\n}"
if ensure.count(new_clone_anchor) != 1:
    raise SystemExit("new clone return anchor changed")
ensure = ensure.replace(
    new_clone_anchor,
    '''    let remote = capture_in_dir(&workspace, "git", &["remote", "get-url", "origin"])?;
    if !github_remote_matches_repository(&remote, repository) {
        return Err(format!(
            "new workspace {} has unexpected origin {remote}",
            workspace.display()
        ));
    }
    workspace_gc::record_workspace_use(&config.data_root, repository, unix_timestamp())?;
    Ok(workspace)
}''',
    1,
)
text = text[:ensure_start] + ensure + text[ensure_end:]

recovery_start = text.index("    let mut resources = resource::sample_linux(&config.data_root)")
recovery_end = text.index("    match admission {", recovery_start)
text = (
    text[:recovery_start]
    + "    let admission = resource_admission_with_recovery(config, &item.repository)?;\n"
    + text[recovery_end:]
)

helper = r'''fn resource_admission_with_recovery(
    config: &RunConfig,
    current_repository: &str,
) -> Result<resource::Admission, ActionFailure> {
    let resources = resource::sample_linux(&config.data_root)
        .classified(state::FailureClass::Infrastructure)?;
    let mut admission = config.resource_policy.evaluate(resources);

    if !matches!(
        &admission,
        resource::Admission::Deferred {
            pressure: resource::PressureKind::Disk,
            ..
        }
    ) {
        return Ok(admission);
    }

    if config.low_disk_reclaim_max_targets > 0 {
        println!(
            "resource gate: low disk detected; reclaiming at most {} managed workspace target cache(s)",
            config.low_disk_reclaim_max_targets
        );
        match reclaim::reclaim_workspace_targets(
            &config.data_root,
            config.low_disk_reclaim_max_targets,
        ) {
            Ok(report) => {
                println!(
                    "disk reclaim: scanned={} removed={}",
                    report.scanned, report.removed
                );
                if report.removed > 0 {
                    match resource::sample_linux(&config.data_root) {
                        Ok(resampled) => {
                            admission = config.resource_policy.evaluate(resampled);
                        }
                        Err(error) => {
                            println!(
                                "disk reclaim: resource re-sample failed; keeping deferral: {error}"
                            );
                            return Ok(admission);
                        }
                    }
                }
            }
            Err(error) => {
                println!(
                    "disk reclaim: failed safely; keeping resource deferral: {error}"
                );
                return Ok(admission);
            }
        }
    }

    if !matches!(
        &admission,
        resource::Admission::Deferred {
            pressure: resource::PressureKind::Disk,
            ..
        }
    ) {
        return Ok(admission);
    }

    if config.low_disk_reclaim_max_workspaces == 0 || config.workspace_min_idle_secs == 0 {
        return Ok(admission);
    }

    println!(
        "resource gate: disk pressure remains; reclaiming at most {} verified stale workspace(s) idle for >= {}s",
        config.low_disk_reclaim_max_workspaces, config.workspace_min_idle_secs
    );
    match workspace_gc::reclaim_stale_workspaces(
        &config.data_root,
        current_repository,
        unix_timestamp(),
        config.workspace_min_idle_secs,
        config.low_disk_reclaim_max_workspaces,
    ) {
        Ok(report) => {
            println!(
                "workspace GC: scanned={} removed={}",
                report.scanned, report.removed
            );
            if report.removed > 0 {
                match resource::sample_linux(&config.data_root) {
                    Ok(resampled) => {
                        admission = config.resource_policy.evaluate(resampled);
                    }
                    Err(error) => {
                        println!(
                            "workspace GC: resource re-sample failed; keeping deferral: {error}"
                        );
                    }
                }
            }
        }
        Err(error) => {
            println!("workspace GC: failed safely; keeping resource deferral: {error}");
        }
    }
    Ok(admission)
}

'''
execute_anchor = "fn execute_item(\n"
if text.count(execute_anchor) != 1:
    raise SystemExit("execute_item anchor changed")
text = text.replace(execute_anchor, helper + execute_anchor, 1)
main.write_text(text)

replace_once(
    "scripts/start.sh",
    'export ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS="${ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS:-4}"\nexport ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM="${ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM:-50}"',
    'export ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS="${ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS:-4}"\nexport ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES="${ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES:-1}"\nexport ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS="${ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS:-604800}"\nexport ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM="${ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM:-50}"',
)
replace_once(
    "scripts/start.sh",
    "printf 'low_disk_reclaim_max_targets=%s\\n' \"$ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS\"\nprintf 'trajectory_max_per_item=%s\\n' \"$ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM\"",
    "printf 'low_disk_reclaim_max_targets=%s\\n' \"$ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS\"\nprintf 'low_disk_reclaim_max_workspaces=%s\\n' \"$ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES\"\nprintf 'workspace_min_idle_secs=%s\\n' \"$ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS\"\nprintf 'trajectory_max_per_item=%s\\n' \"$ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM\"",
)

replace_once(
    "scripts/install-systemd.sh",
    "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS=4\nORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=50",
    "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS=4\nORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES=1\nORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS=604800\nORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=50",
)

replace_once(
    "README.md",
    "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS=4\nORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=50",
    "ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS=4\nORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES=1\nORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS=604800\nORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=50",
)
replace_once(
    "README.md",
    "When disk pressure alone causes the deferral, Orchestrator may reclaim at most `ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS` managed workspace build caches (default 4) before resampling resources. Only a real `<data_root>/workspaces/<owner>__<repo>/target` directory is eligible, and only after the workspace has a real `.git` directory and its `origin` exactly matches the encoded GitHub repository. Symlink targets, foreign origins, sources, Git metadata, state, and trajectories are never removed. Set the reclaim limit to `0` to disable automatic cache reclamation.",
    "When disk pressure alone causes the deferral, Orchestrator first may reclaim at most `ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS` managed workspace build caches (default 4) before resampling resources. Only a real `<data_root>/workspaces/<owner>__<repo>/target` directory is eligible, and only after the workspace has a real `.git` directory and its `origin` exactly matches the encoded GitHub repository. Symlink targets, foreign origins, sources, Git metadata, state, and trajectories are never removed. Set the target reclaim limit to `0` to disable this first stage.\n\nIf disk pressure remains, Orchestrator may remove at most `ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES` complete managed clones (default 1) that have been unused by Orchestrator for at least `ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS` seconds (default 604800, seven days). A clone becomes eligible only after Orchestrator has verified it and written an atomic usage marker; pre-existing unadopted clones are never garbage-collected. The currently selected repository is always excluded. Before deletion, the clone must be a real directory with a real `.git`, an exact matching GitHub origin, a symbolic branch HEAD, a completely clean status including ignored/untracked files, no stash, no local tags, no submodules or linked worktrees, no Git operation/lock marker, and every local branch must be recoverable from the same-named `origin/*` branch. Any uncertainty preserves the clone. Setting either the workspace reclaim limit or minimum idle time to `0` disables complete-clone GC.",
)
