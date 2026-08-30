use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAX_SCAN_FILES: usize = 10_000;
const RECENT_TRAJECTORIES: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HealthReport {
    pub(crate) text: String,
    pub(crate) degraded: bool,
}

#[derive(Debug, Default)]
struct WorkCounts {
    total: usize,
    ready: usize,
    cooldown: usize,
    quarantine: usize,
    in_progress: usize,
    progress: usize,
    no_progress: usize,
    deferred: usize,
    failure: usize,
    unknown_outcome: usize,
    corrupt: usize,
}

#[derive(Debug, Default)]
struct PublicationCounts {
    total: usize,
    prepared: usize,
    pushed: usize,
    corrupt: usize,
}

pub(crate) fn inspect(data_root: &Path, now: u64) -> HealthReport {
    let mut lines = Vec::new();
    let mut degraded = false;
    lines.push("Memorithm Orchestrator health".to_owned());
    lines.push("============================".to_owned());
    lines.push(format!("data root        : {}", data_root.display()));

    let (lock_text, lock_degraded) = inspect_lock(data_root);
    degraded |= lock_degraded;
    lines.push(format!("instance lock    : {lock_text}"));

    let workspace_count = immediate_directory_count(&data_root.join("workspaces"));
    lines.push(format!("workspaces       : {workspace_count}"));

    let (work, work_degraded) = inspect_work_items(&data_root.join("state/work-items"), now);
    degraded |= work_degraded;
    lines.push(format!(
        "work items       : total={} ready={} cooldown={} quarantine={} in_progress={} corrupt={}",
        work.total, work.ready, work.cooldown, work.quarantine, work.in_progress, work.corrupt
    ));
    lines.push(format!(
        "last outcomes    : progress={} no_progress={} deferred={} failure={} unknown={}",
        work.progress, work.no_progress, work.deferred, work.failure, work.unknown_outcome
    ));

    let (publications, publication_degraded) =
        inspect_publications(&data_root.join("state/publications"));
    degraded |= publication_degraded;
    lines.push(format!(
        "publications     : total={} prepared={} pushed={} corrupt={}",
        publications.total, publications.prepared, publications.pushed, publications.corrupt
    ));

    let (attestations, attestation_corrupt, attestation_degraded) =
        inspect_attestations(&data_root.join("state/merge-attestations"));
    degraded |= attestation_degraded;
    lines.push(format!(
        "merge attest.    : total={attestations} corrupt={attestation_corrupt}"
    ));

    let trajectory_root = data_root.join("trajectories");
    let (trajectory_files, trajectory_overflow) = collect_files(&trajectory_root, Some("jsonl"));
    if trajectory_overflow {
        degraded = true;
    }
    lines.push(format!(
        "trajectories     : {}{}",
        trajectory_files.len(),
        if trajectory_overflow {
            "+ (scan bounded)"
        } else {
            ""
        }
    ));

    let mut recent = trajectory_files
        .into_iter()
        .filter_map(|path| {
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            let timestamp = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((timestamp, path))
        })
        .collect::<Vec<_>>();
    recent.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    if !recent.is_empty() {
        lines.push("recent trajectories:".to_owned());
        for (timestamp, path) in recent.into_iter().take(RECENT_TRAJECTORIES) {
            let relative = path.strip_prefix(data_root).unwrap_or(path.as_path());
            lines.push(format!("  unix={timestamp} {}", relative.display()));
        }
    }

    lines.push(format!(
        "overall          : {}",
        if degraded { "DEGRADED" } else { "OK" }
    ));
    HealthReport {
        text: lines.join("\n"),
        degraded,
    }
}

fn inspect_lock(data_root: &Path) -> (String, bool) {
    let path = data_root.join("orchestrator.lock");
    if !path.exists() {
        return ("absent".to_owned(), false);
    }
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) => return (format!("unreadable ({error})"), true),
    };
    let pid = match contents.trim().parse::<u32>() {
        Ok(pid) if pid != 0 => pid,
        _ => return ("malformed".to_owned(), true),
    };
    if process_is_alive(pid) {
        (format!("active pid={pid}"), false)
    } else {
        (format!("STALE pid={pid}"), true)
    }
}

fn process_is_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

fn immediate_directory_count(root: &Path) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .count()
}

fn inspect_work_items(root: &Path, now: u64) -> (WorkCounts, bool) {
    let (files, overflow) = collect_files(root, Some("state"));
    let mut counts = WorkCounts {
        total: files.len(),
        ..WorkCounts::default()
    };
    for path in files {
        match parse_key_value_state(&path, &["v1", "v2", "v3", "v4", "v5"]) {
            Ok(fields) => {
                let next = numeric_field(&fields, "next_eligible_at");
                let quarantine = numeric_field(&fields, "quarantine_until");
                let in_progress = numeric_field(&fields, "in_progress_since");
                let schedule_valid = match (next, quarantine, in_progress) {
                    (Ok(next), Ok(quarantine), Ok(in_progress)) => {
                        if in_progress != 0 {
                            counts.in_progress += 1;
                        } else if quarantine > now {
                            counts.quarantine += 1;
                        } else if next > now {
                            counts.cooldown += 1;
                        } else {
                            counts.ready += 1;
                        }
                        true
                    }
                    _ => false,
                };
                if !schedule_valid {
                    counts.corrupt += 1;
                    continue;
                }

                match fields.get("last_outcome").map(String::as_str) {
                    Some("success") => counts.progress += 1,
                    Some("no_progress") => counts.no_progress += 1,
                    Some("deferred") => counts.deferred += 1,
                    Some("failure") => counts.failure += 1,
                    Some("none") | None => counts.unknown_outcome += 1,
                    Some(_) => counts.corrupt += 1,
                }
            }
            Err(_) => counts.corrupt += 1,
        }
    }
    let degraded = overflow || counts.corrupt != 0;
    (counts, degraded)
}

fn inspect_publications(root: &Path) -> (PublicationCounts, bool) {
    let (files, overflow) = collect_files(root, Some("state"));
    let mut counts = PublicationCounts {
        total: files.len(),
        ..PublicationCounts::default()
    };
    for path in files {
        match parse_key_value_state(&path, &["v1"]) {
            Ok(fields) => match fields.get("phase").map(String::as_str) {
                Some("prepared") => counts.prepared += 1,
                Some("pushed") => counts.pushed += 1,
                _ => counts.corrupt += 1,
            },
            Err(_) => counts.corrupt += 1,
        }
    }
    let degraded = overflow || counts.corrupt != 0;
    (counts, degraded)
}

fn inspect_attestations(root: &Path) -> (usize, usize, bool) {
    let (files, overflow) = collect_files(root, Some("state"));
    let total = files.len();
    let mut corrupt = 0;
    for path in files {
        match parse_key_value_state(&path, &["v1"]) {
            Ok(fields) => {
                let sha_valid = fields.get("head_sha").is_some_and(|sha| {
                    sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
                });
                let number_valid =
                    numeric_field(&fields, "pr_number").is_ok_and(|value| value != 0);
                if !sha_valid || !number_valid {
                    corrupt += 1;
                }
            }
            Err(_) => corrupt += 1,
        }
    }
    (total, corrupt, overflow || corrupt != 0)
}

fn numeric_field(fields: &BTreeMap<String, String>, name: &str) -> Result<u64, ()> {
    fields.get(name).ok_or(())?.parse::<u64>().map_err(|_| ())
}

fn parse_key_value_state(
    path: &Path,
    accepted_versions: &[&str],
) -> Result<BTreeMap<String, String>, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut lines = contents.lines();
    let version = lines.next().unwrap_or_default();
    if !accepted_versions.contains(&version) {
        return Err(format!("unsupported state version {version}"));
    }
    let mut fields = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed field {line}"))?;
        if fields.insert(name.to_owned(), value.to_owned()).is_some() {
            return Err(format!("duplicate field {name}"));
        }
    }
    Ok(fields)
}

fn collect_files(root: &Path, extension: Option<&str>) -> (Vec<PathBuf>, bool) {
    if !root.exists() {
        return (Vec::new(), false);
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut overflow = false;
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            overflow = true;
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                overflow = true;
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let path = entry.path();
            if extension.is_some_and(|expected| {
                path.extension().and_then(|value| value.to_str()) != Some(expected)
            }) {
                continue;
            }
            if files.len() >= MAX_SCAN_FILES {
                overflow = true;
                return (files, overflow);
            }
            files.push(path);
        }
    }
    (files, overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "orchestrator-health-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    #[test]
    fn empty_data_root_is_healthy() {
        let root = temp_root("empty");
        let report = inspect(&root, 100);
        assert!(!report.degraded);
        assert!(report.text.contains("work items       : total=0"));
        assert!(report.text.contains("overall          : OK"));
    }

    #[test]
    fn work_item_categories_are_reported_offline() {
        let root = temp_root("work-items");
        let state_root = root.join("state/work-items/Memorithm__ADA");
        fs::create_dir_all(&state_root).unwrap();
        let base = |next: u64, quarantine: u64, in_progress: u64| {
            format!(
                "v4\nrevision=issue-v1\ntotal_attempts=1\nconsecutive_failures=0\nlast_attempt_at=50\nnext_eligible_at={next}\nquarantine_until={quarantine}\nin_progress_since={in_progress}\nlast_outcome=success\nlast_failure_class=none\n"
            )
        };
        fs::write(state_root.join("ISSUE-1.state"), base(0, 0, 0)).unwrap();
        fs::write(state_root.join("ISSUE-2.state"), base(200, 0, 0)).unwrap();
        fs::write(state_root.join("ISSUE-3.state"), base(0, 300, 0)).unwrap();
        fs::write(state_root.join("ISSUE-4.state"), base(0, 0, 80)).unwrap();

        let report = inspect(&root, 100);
        assert!(!report.degraded);
        assert!(report.text.contains(
            "work items       : total=4 ready=1 cooldown=1 quarantine=1 in_progress=1 corrupt=0"
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_action_outcomes_are_reported_offline() {
        let root = temp_root("outcomes");
        let state_root = root.join("state/work-items/Memorithm__ADA");
        fs::create_dir_all(&state_root).unwrap();
        let write = |name: &str, outcome: &str| {
            fs::write(
                state_root.join(name),
                format!(
                    "v5\nrevision=issue-v1\ntotal_attempts=1\nconsecutive_failures=0\nlast_attempt_at=50\nnext_eligible_at=0\nquarantine_until=0\nin_progress_since=0\nlast_outcome={outcome}\nlast_failure_class=none\n"
                ),
            )
            .unwrap();
        };
        write("ISSUE-1.state", "success");
        write("ISSUE-2.state", "no_progress");
        write("ISSUE-3.state", "deferred");
        write("ISSUE-4.state", "failure");
        write("ISSUE-5.state", "none");

        let report = inspect(&root, 100);
        assert!(!report.degraded);
        assert!(report.text.contains(
            "last outcomes    : progress=1 no_progress=1 deferred=1 failure=1 unknown=1"
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_state_marks_health_degraded() {
        let root = temp_root("corrupt");
        let state_root = root.join("state/work-items/Memorithm__ADA");
        fs::create_dir_all(&state_root).unwrap();
        fs::write(state_root.join("ISSUE-7.state"), "v99\nbroken=true\n").unwrap();
        let report = inspect(&root, 100);
        assert!(report.degraded);
        assert!(report.text.contains("corrupt=1"));
        assert!(report.text.contains("overall          : DEGRADED"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pending_publication_and_attestation_are_visible() {
        let root = temp_root("publication");
        let publication = root.join("state/publications/Memorithm__ADA");
        fs::create_dir_all(&publication).unwrap();
        fs::write(
            publication.join("issue-7.state"),
            "v1\nbranch=orchestrator/issue-7-1\ncommit=0123456789abcdef0123456789abcdef01234567\nbase_branch=main\nphase=pushed\n",
        )
        .unwrap();
        let attestation = root.join("state/merge-attestations/Memorithm__ADA");
        fs::create_dir_all(&attestation).unwrap();
        fs::write(
            attestation.join("pr-34.state"),
            "v1\nrepository=Memorithm/ADA\npr_number=34\nhead_sha=0123456789abcdef0123456789abcdef01234567\nvalidated_at=123\n",
        )
        .unwrap();
        let report = inspect(&root, 100);
        assert!(!report.degraded);
        assert!(
            report
                .text
                .contains("publications     : total=1 prepared=0 pushed=1 corrupt=0")
        );
        assert!(report.text.contains("merge attest.    : total=1 corrupt=0"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_lock_is_degraded_without_network_access() {
        let root = temp_root("lock");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("orchestrator.lock"), "not-a-pid\n").unwrap();
        let report = inspect(&root, 100);
        assert!(report.degraded);
        assert!(report.text.contains("instance lock    : malformed"));
        let _ = fs::remove_dir_all(root);
    }
}
