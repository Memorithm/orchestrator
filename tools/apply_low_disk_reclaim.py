from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if text.count(old) != 1:
        raise SystemExit(f"expected exactly one match in {path}: {old!r}")
    file.write_text(text.replace(old, new, 1))


replace_once(
    "src/main.rs",
    "mod publication;\nmod resource;",
    "mod publication;\nmod reclaim;\nmod resource;",
)
replace_once(
    "src/main.rs",
    "    resource_policy: resource::ResourcePolicy,\n    trajectory_max_per_item: usize,",
    "    resource_policy: resource::ResourcePolicy,\n    low_disk_reclaim_max_targets: usize,\n    trajectory_max_per_item: usize,",
)
replace_once(
    "src/main.rs",
    "            resource_policy: resource::ResourcePolicy::from_env()?,\n            trajectory_max_per_item: trajectory::max_files_per_item_from_env()?,",
    "            resource_policy: resource::ResourcePolicy::from_env()?,\n            low_disk_reclaim_max_targets: usize::try_from(env_u64(\n                \"ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS\",\n                4,\n            ))\n            .map_err(|_| \"ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS does not fit usize\".to_owned())?,\n            trajectory_max_per_item: trajectory::max_files_per_item_from_env()?,",
)
replace_once(
    "src/main.rs",
    "    println!(\n        \"trajectory keep  : {} per work item (0=unlimited)\",\n        config.trajectory_max_per_item\n    );",
    "    println!(\n        \"disk reclaim cap : {} workspace targets per pressure event (0=disabled)\",\n        config.low_disk_reclaim_max_targets\n    );\n    println!(\n        \"trajectory keep  : {} per work item (0=unlimited)\",\n        config.trajectory_max_per_item\n    );",
)
replace_once(
    "src/main.rs",
    "    let resources = resource::sample_linux(&config.data_root)\n        .classified(state::FailureClass::Infrastructure)?;\n    match config.resource_policy.evaluate(resources) {",
    "    let mut resources = resource::sample_linux(&config.data_root)\n        .classified(state::FailureClass::Infrastructure)?;\n    let mut admission = config.resource_policy.evaluate(resources);\n    if matches!(\n        admission,\n        resource::Admission::Deferred {\n            pressure: resource::PressureKind::Disk,\n            ..\n        }\n    ) && config.low_disk_reclaim_max_targets > 0\n    {\n        println!(\n            \"resource gate: low disk detected; reclaiming at most {} managed workspace target cache(s)\",\n            config.low_disk_reclaim_max_targets\n        );\n        match reclaim::reclaim_workspace_targets(\n            &config.data_root,\n            config.low_disk_reclaim_max_targets,\n        ) {\n            Ok(report) => {\n                println!(\n                    \"disk reclaim: scanned={} removed={}\",\n                    report.scanned, report.removed\n                );\n                if report.removed > 0 {\n                    match resource::sample_linux(&config.data_root) {\n                        Ok(resampled) => {\n                            resources = resampled;\n                            admission = config.resource_policy.evaluate(resources);\n                        }\n                        Err(error) => {\n                            println!(\n                                \"disk reclaim: resource re-sample failed; keeping original deferral: {error}\"\n                            );\n                        }\n                    }\n                }\n            }\n            Err(error) => {\n                println!(\n                    \"disk reclaim: failed safely; keeping original resource deferral: {error}\"\n                );\n            }\n        }\n    }\n    match admission {",
)
replace_once(
    "src/main.rs",
    "        resource::Admission::Deferred { snapshot, reason } => {",
    "        resource::Admission::Deferred {\n            snapshot, reason, ..\n        } => {",
)

replace_once(
    "scripts/start.sh",
    'export ORCHESTRATOR_MAX_LOAD_PER_CPU="${ORCHESTRATOR_MAX_LOAD_PER_CPU:-2.0}"\nexport ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM="${ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM:-50}"',
    'export ORCHESTRATOR_MAX_LOAD_PER_CPU="${ORCHESTRATOR_MAX_LOAD_PER_CPU:-2.0}"\nexport ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS="${ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS:-4}"\nexport ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM="${ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM:-50}"',
)
replace_once(
    "scripts/start.sh",
    "printf 'max_load_per_cpu=%s\\n' \"$ORCHESTRATOR_MAX_LOAD_PER_CPU\"\nprintf 'trajectory_max_per_item=%s\\n' \"$ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM\"",
    "printf 'max_load_per_cpu=%s\\n' \"$ORCHESTRATOR_MAX_LOAD_PER_CPU\"\nprintf 'low_disk_reclaim_max_targets=%s\\n' \"$ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS\"\nprintf 'trajectory_max_per_item=%s\\n' \"$ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM\"",
)

replace_once(
    "scripts/install-systemd.sh",
    "ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0\nORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=50",
    "ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0\nORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS=4\nORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=50",
)

replace_once(
    "README.md",
    "ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0\nORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=50",
    "ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0\nORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS=4\nORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=50",
)
replace_once(
    "README.md",
    "Before executing a selected work item, the Linux runtime samples `MemAvailable`, free space on the filesystem containing `ORCHESTRATOR_DATA_ROOT`, and the one-minute load average. By default it defers work when less than 4096 MiB of memory or 8192 MiB of data-root disk is available, or when load exceeds 2.0 per available CPU. A resource deferral is not a research failure and therefore does not increase the failure/quarantine count. Set any resource threshold to `0` to disable that gate.",
    "Before executing a selected work item, the Linux runtime samples `MemAvailable`, free space on the filesystem containing `ORCHESTRATOR_DATA_ROOT`, and the one-minute load average. By default it defers work when less than 4096 MiB of memory or 8192 MiB of data-root disk is available, or when load exceeds 2.0 per available CPU. A resource deferral is not a research failure and therefore does not increase the failure/quarantine count. Set any resource threshold to `0` to disable that gate.\n\nWhen disk pressure alone causes the deferral, Orchestrator may reclaim at most `ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS` managed workspace build caches (default 4) before resampling resources. Only a real `<data_root>/workspaces/<owner>__<repo>/target` directory is eligible, and only after the workspace has a real `.git` directory and its `origin` exactly matches the encoded GitHub repository. Symlink targets, foreign origins, sources, Git metadata, state, and trajectories are never removed. Set the reclaim limit to `0` to disable automatic cache reclamation.",
)
