use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const DETAIL_LIMIT_CHARS: usize = 16_000;

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
        self.file
            .write_all(line.as_bytes())
            .map_err(|error| format!("failed to append trajectory {}: {error}", self.path.display()))?;
        self.file
            .sync_data()
            .map_err(|error| format!("failed to sync trajectory {}: {error}", self.path.display()))
    }
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

    #[test]
    fn json_escape_handles_log_control_characters() {
        assert_eq!(json_escape("a\"b\\c\nd\te"), "a\\\"b\\\\c\\nd\\te");
    }

    #[test]
    fn trajectory_is_append_only_and_persisted_per_event() {
        let root = temporary_root("append");
        let path = {
            let mut journal = AttemptJournal::create(
                &root,
                "Memorithm/ADA",
                "ISSUE",
                7,
                "ollama/qwen3.8:latest",
                100,
            )
            .unwrap();
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
}
