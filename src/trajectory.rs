use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const DETAIL_LIMIT_CHARS: usize = 16_000;
const DEFAULT_MAX_FILES_PER_ITEM: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventPhase {
    AttemptStarted,
    AttemptFinished,
}

impl EventPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AttemptStarted => "attempt_started",
            Self::AttemptFinished => "attempt_finished",
        }
    }
}

pub(crate) fn max_files_per_item_from_env() -> Result<usize, String> {
    let value = match env::var("ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => return Ok(DEFAULT_MAX_FILES_PER_ITEM),
        Err(env::VarError::NotUnicode(_)) => {
            return Err("ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM is not valid UTF-8".to_owned());
        }
    };
    parse_retention_limit(&value)
}

fn parse_retention_limit(value: &str) -> Result<usize, String> {
    let parsed = value.parse::<u64>().map_err(|error| {
        format!("invalid ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM={value:?}: {error}")
    })?;
    usize::try_from(parsed)
        .map_err(|_| "ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM does not fit in usize".to_owned())
}

#[derive(Debug)]
pub(crate) struct AttemptJournal {
    path: PathBuf,
    file: File,
    sequence: u64,
}

impl AttemptJournal {
    pub(crate) fn create(
        root: &Path,
        repository: &str,
        kind: &str,
        number: u64,
        model: &str,
        started_at: u64,
        max_files_per_item: usize,
    ) -> Result<Self, String> {
        let repository_dir = repository.replace('/', "__");
        let kind_component = safe_component(kind);
        let attempt_dir = root
            .join(repository_dir)
            .join(format!("{kind_component}-{number}"));
        fs::create_dir_all(&attempt_dir).map_err(|error| {
            format!(
                "failed to create trajectory directory {}: {error}",
                attempt_dir.display()
            )
        })?;

        if max_files_per_item != 0 {
            prune_managed_trajectories(&attempt_dir, max_files_per_item.saturating_sub(1))?;
        }

        let path = attempt_dir.join(format!("{started_at}-{}.jsonl", std::process::id()));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("failed to create trajectory {}: {error}", path.display()))?;

        let mut journal = Self {
            path,
            file,
            sequence: 0,
        };
        journal.record(
            EventPhase::AttemptStarted,
            "running",
            &format!("model={model}"),
            started_at,
        )?;
        Ok(journal)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn record(
        &mut self,
        phase: EventPhase,
        outcome: &str,
        detail: &str,
        timestamp: u64,
    ) -> Result<(), String> {
        self.sequence = self.sequence.saturating_add(1);
        let detail = truncate_chars(detail, DETAIL_LIMIT_CHARS);
        let line = format!(
            "{{\"schema\":1,\"seq\":{},\"timestamp\":{},\"phase\":\"{}\",\"outcome\":\"{}\",\"detail\":\"{}\"}}\n",
            self.sequence,
            timestamp,
            phase.as_str(),
            json_escape(outcome),
            json_escape(&detail),
        );
        self.file.write_all(line.as_bytes()).map_err(|error| {
            format!(
                "failed to append trajectory {}: {error}",
                self.path.display()
            )
        })?;
        self.file
            .sync_data()
            .map_err(|error| format!("failed to sync trajectory {}: {error}", self.path.display()))
    }
}

fn prune_managed_trajectories(attempt_dir: &Path, keep_existing: usize) -> Result<usize, String> {
    let entries = fs::read_dir(attempt_dir).map_err(|error| {
        format!(
            "failed to read trajectory directory {}: {error}",
            attempt_dir.display()
        )
    })?;
    let mut managed = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to enumerate trajectory directory {}: {error}",
                attempt_dir.display()
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect trajectory entry {}: {error}",
                entry.path().display()
            )
        })?;
        if file_type.is_symlink() || !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if let Some(key) = managed_trajectory_key(&path) {
            managed.push((key, path));
        }
    }

    managed.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    let remove_count = managed.len().saturating_sub(keep_existing);
    for (_, path) in managed.into_iter().take(remove_count) {
        fs::remove_file(&path)
            .map_err(|error| format!("failed to prune trajectory {}: {error}", path.display()))?;
    }
    Ok(remove_count)
}

fn managed_trajectory_key(path: &Path) -> Option<(u64, u32)> {
    if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let (timestamp, pid) = stem.rsplit_once('-')?;
    Some((timestamp.parse().ok()?, pid.parse().ok()?))
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    let mut characters = value.chars();
    let prefix = characters.by_ref().take(maximum).collect::<String>();
    if characters.next().is_some() {
        format!("{prefix}\n...[truncated by orchestrator]")
    } else {
        prefix
    }
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{:04x}", u32::from(character));
            }
            character => escaped.push(character),
        }
    }
    escaped
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
            "orchestrator-trajectory-test-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn create_journal(root: &Path, started_at: u64, max_files_per_item: usize) -> AttemptJournal {
        AttemptJournal::create(
            root,
            "Memorithm/ADA",
            "ISSUE",
            7,
            "ollama/qwen3.8:latest",
            started_at,
            max_files_per_item,
        )
        .unwrap()
    }

    #[test]
    fn json_escape_handles_log_control_characters() {
        assert_eq!(json_escape("a\"b\\c\nd\te"), "a\\\"b\\\\c\\nd\\te");
    }

    #[test]
    fn trajectory_is_append_only_and_persisted_per_event() {
        let root = temporary_root("append");
        let path = {
            let mut journal = create_journal(&root, 100, 50);
            journal
                .record(EventPhase::AttemptFinished, "success", "done", 102)
                .unwrap();
            journal.path().to_path_buf()
        };

        let contents = fs::read_to_string(&path).unwrap();
        let lines = contents.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"seq\":1"));
        assert!(lines[0].contains("\"phase\":\"attempt_started\""));
        assert!(lines[1].contains("\"seq\":2"));
        assert!(lines[1].contains("\"phase\":\"attempt_finished\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn trajectory_path_cannot_escape_root_via_kind() {
        let root = temporary_root("safe-path");
        let journal = AttemptJournal::create(
            &root,
            "Memorithm/ADA",
            "../../ISSUE",
            7,
            "ollama/qwen3.8:latest",
            100,
            50,
        )
        .unwrap();
        assert!(journal.path().starts_with(&root));
        assert!(!journal.path().to_string_lossy().contains("../"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn details_are_bounded_before_persistence() {
        let oversized = "x".repeat(DETAIL_LIMIT_CHARS + 100);
        let truncated = truncate_chars(&oversized, DETAIL_LIMIT_CHARS);
        assert!(truncated.len() < oversized.len());
        assert!(truncated.ends_with("...[truncated by orchestrator]"));
    }

    #[test]
    fn retention_keeps_only_newest_managed_trajectories() {
        let root = temporary_root("retention");
        for started_at in 100..105 {
            drop(create_journal(&root, started_at, 3));
        }
        let attempt_dir = root.join("Memorithm__ADA/ISSUE-7");
        let mut names = fs::read_dir(&attempt_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        let pid = std::process::id();
        assert_eq!(
            names,
            vec![
                format!("102-{pid}.jsonl"),
                format!("103-{pid}.jsonl"),
                format!("104-{pid}.jsonl")
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn retention_never_deletes_unmanaged_jsonl_files() {
        let root = temporary_root("unmanaged");
        let attempt_dir = root.join("Memorithm__ADA/ISSUE-7");
        fs::create_dir_all(&attempt_dir).unwrap();
        fs::write(attempt_dir.join("manual.jsonl"), "keep\n").unwrap();
        for started_at in 100..103 {
            drop(create_journal(&root, started_at, 2));
        }
        assert!(attempt_dir.join("manual.jsonl").exists());
        let managed = fs::read_dir(&attempt_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| managed_trajectory_key(&entry.path()).is_some())
            .count();
        assert_eq!(managed, 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zero_retention_limit_disables_pruning() {
        let root = temporary_root("unlimited");
        for started_at in 100..104 {
            drop(create_journal(&root, started_at, 0));
        }
        let attempt_dir = root.join("Memorithm__ADA/ISSUE-7");
        let managed = fs::read_dir(&attempt_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| managed_trajectory_key(&entry.path()).is_some())
            .count();
        assert_eq!(managed, 4);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn retention_limit_parser_is_strict() {
        assert_eq!(parse_retention_limit("50").unwrap(), 50);
        assert_eq!(parse_retention_limit("0").unwrap(), 0);
        assert!(parse_retention_limit("-1").is_err());
        assert!(parse_retention_limit("many").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn retention_does_not_follow_or_delete_symlinks() {
        use std::os::unix::fs::symlink;

        let root = temporary_root("symlink");
        let attempt_dir = root.join("Memorithm__ADA/ISSUE-7");
        fs::create_dir_all(&attempt_dir).unwrap();
        let target = root.join("outside.jsonl");
        fs::write(&target, "outside\n").unwrap();
        symlink(&target, attempt_dir.join("1-1.jsonl")).unwrap();
        drop(create_journal(&root, 100, 1));
        assert!(target.exists());
        assert!(attempt_dir.join("1-1.jsonl").is_symlink());
        let _ = fs::remove_dir_all(root);
    }
}
