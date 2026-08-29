use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const STATE_VERSION: &str = "v2";
const LEGACY_STATE_VERSION: &str = "v1";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct WorkKey {
    pub(crate) repository: String,
    pub(crate) kind: String,
    pub(crate) number: u64,
}

impl WorkKey {
    pub(crate) fn new(repository: &str, kind: &str, number: u64) -> Self {
        Self {
            repository: repository.to_owned(),
            kind: kind.to_owned(),
            number,
        }
    }

    fn state_path(&self, root: &Path) -> PathBuf {
        let repository = self.repository.replace('/', "__");
        let kind = self
            .kind
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        root.join(repository)
            .join(format!("{kind}-{}.state", self.number))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttemptOutcome {
    Success,
    Failure,
}

impl AttemptOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }

    fn parse(value: &str) -> Result<Option<Self>, String> {
        match value {
            "" | "none" => Ok(None),
            "success" => Ok(Some(Self::Success)),
            "failure" => Ok(Some(Self::Failure)),
            other => Err(format!("unknown attempt outcome: {other}")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AttemptState {
    pub(crate) total_attempts: u64,
    pub(crate) consecutive_failures: u32,
    pub(crate) last_attempt_at: u64,
    pub(crate) next_eligible_at: u64,
    pub(crate) quarantine_until: u64,
    pub(crate) in_progress_since: u64,
    pub(crate) last_outcome: Option<AttemptOutcome>,
}

impl AttemptState {
    pub(crate) fn eligible_at(&self) -> u64 {
        self.next_eligible_at.max(self.quarantine_until)
    }

    pub(crate) fn is_eligible(&self, now: u64) -> bool {
        now >= self.eligible_at()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetryPolicy {
    pub(crate) success_cooldown_secs: u64,
    pub(crate) failure_base_cooldown_secs: u64,
    pub(crate) failure_max_cooldown_secs: u64,
    pub(crate) quarantine_after_failures: u32,
    pub(crate) quarantine_secs: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            success_cooldown_secs: 900,
            failure_base_cooldown_secs: 300,
            failure_max_cooldown_secs: 7_200,
            quarantine_after_failures: 4,
            quarantine_secs: 21_600,
        }
    }
}

impl RetryPolicy {
    fn failure_cooldown(self, failures: u32) -> u64 {
        let exponent = failures.saturating_sub(1).min(31);
        let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
        self.failure_base_cooldown_secs
            .saturating_mul(multiplier)
            .min(self.failure_max_cooldown_secs)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AttemptStore {
    root: PathBuf,
    policy: RetryPolicy,
}

impl AttemptStore {
    pub(crate) fn new(root: PathBuf, policy: RetryPolicy) -> Self {
        Self { root, policy }
    }

    pub(crate) fn load(&self, key: &WorkKey) -> Result<AttemptState, String> {
        let path = key.state_path(&self.root);
        if !path.exists() {
            return Ok(AttemptState::default());
        }
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read attempt state {}: {error}", path.display()))?;
        parse_state(&contents)
            .map_err(|error| format!("invalid attempt state {}: {error}", path.display()))
    }

    pub(crate) fn begin(&self, key: &WorkKey, now: u64) -> Result<AttemptState, String> {
        let mut state = self.load(key)?;
        if state.in_progress_since != 0 {
            return Err(format!(
                "attempt already marked in progress since unix={}",
                state.in_progress_since
            ));
        }
        state.in_progress_since = now;
        self.save(key, &state)?;
        Ok(state)
    }

    pub(crate) fn recover_interrupted(
        &self,
        key: &WorkKey,
        now: u64,
    ) -> Result<Option<AttemptState>, String> {
        let mut state = self.load(key)?;
        if state.in_progress_since == 0 {
            return Ok(None);
        }
        state.in_progress_since = 0;
        self.apply_outcome(&mut state, AttemptOutcome::Failure, now);
        self.save(key, &state)?;
        Ok(Some(state))
    }

    pub(crate) fn record(
        &self,
        key: &WorkKey,
        outcome: AttemptOutcome,
        now: u64,
    ) -> Result<AttemptState, String> {
        let mut state = self.load(key)?;
        state.in_progress_since = 0;
        self.apply_outcome(&mut state, outcome, now);
        self.save(key, &state)?;
        Ok(state)
    }

    fn apply_outcome(&self, state: &mut AttemptState, outcome: AttemptOutcome, now: u64) {
        state.total_attempts = state.total_attempts.saturating_add(1);
        state.last_attempt_at = now;
        state.last_outcome = Some(outcome);

        match outcome {
            AttemptOutcome::Success => {
                state.consecutive_failures = 0;
                state.quarantine_until = 0;
                state.next_eligible_at = now.saturating_add(self.policy.success_cooldown_secs);
            }
            AttemptOutcome::Failure => {
                state.consecutive_failures = state.consecutive_failures.saturating_add(1);
                let cooldown = self.policy.failure_cooldown(state.consecutive_failures);
                state.next_eligible_at = now.saturating_add(cooldown);
                if state.consecutive_failures >= self.policy.quarantine_after_failures {
                    state.quarantine_until = now.saturating_add(self.policy.quarantine_secs);
                }
            }
        }
    }

    fn save(&self, key: &WorkKey, state: &AttemptState) -> Result<(), String> {
        let path = key.state_path(&self.root);
        let parent = path
            .parent()
            .ok_or_else(|| format!("attempt state path has no parent: {}", path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;

        let temporary = path.with_extension(format!("state.tmp.{}", std::process::id()));
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("failed to open {}: {error}", temporary.display()))?;
        file.write_all(serialize_state(state).as_bytes())
            .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("failed to sync {}: {error}", temporary.display()))?;
        fs::rename(&temporary, &path).map_err(|error| {
            format!(
                "failed to atomically replace attempt state {} with {}: {error}",
                path.display(),
                temporary.display()
            )
        })
    }
}

fn serialize_state(state: &AttemptState) -> String {
    format!(
        "{STATE_VERSION}\ntotal_attempts={}\nconsecutive_failures={}\nlast_attempt_at={}\nnext_eligible_at={}\nquarantine_until={}\nin_progress_since={}\nlast_outcome={}\n",
        state.total_attempts,
        state.consecutive_failures,
        state.last_attempt_at,
        state.next_eligible_at,
        state.quarantine_until,
        state.in_progress_since,
        state.last_outcome.map_or("none", AttemptOutcome::as_str)
    )
}

fn parse_state(contents: &str) -> Result<AttemptState, String> {
    let mut lines = contents.lines();
    let version = lines.next().unwrap_or_default();
    if version != STATE_VERSION && version != LEGACY_STATE_VERSION {
        return Err(format!("unsupported state version: {version}"));
    }
    let legacy = version == LEGACY_STATE_VERSION;

    let mut state = AttemptState::default();
    let mut seen = std::collections::BTreeSet::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed state field: {line}"))?;
        if !seen.insert(name.to_owned()) {
            return Err(format!("duplicate state field: {name}"));
        }
        match name {
            "total_attempts" => {
                state.total_attempts = value
                    .parse()
                    .map_err(|error| format!("invalid total_attempts {value}: {error}"))?;
            }
            "consecutive_failures" => {
                state.consecutive_failures = value
                    .parse()
                    .map_err(|error| format!("invalid consecutive_failures {value}: {error}"))?;
            }
            "last_attempt_at" => {
                state.last_attempt_at = value
                    .parse()
                    .map_err(|error| format!("invalid last_attempt_at {value}: {error}"))?;
            }
            "next_eligible_at" => {
                state.next_eligible_at = value
                    .parse()
                    .map_err(|error| format!("invalid next_eligible_at {value}: {error}"))?;
            }
            "quarantine_until" => {
                state.quarantine_until = value
                    .parse()
                    .map_err(|error| format!("invalid quarantine_until {value}: {error}"))?;
            }
            "in_progress_since" => {
                if legacy {
                    return Err("v1 state cannot contain in_progress_since".to_owned());
                }
                state.in_progress_since = value
                    .parse()
                    .map_err(|error| format!("invalid in_progress_since {value}: {error}"))?;
            }
            "last_outcome" => state.last_outcome = AttemptOutcome::parse(value)?,
            other => return Err(format!("unknown state field: {other}")),
        }
    }
    Ok(state)
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
            "orchestrator-state-test-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn test_policy() -> RetryPolicy {
        RetryPolicy {
            success_cooldown_secs: 30,
            failure_base_cooldown_secs: 10,
            failure_max_cooldown_secs: 40,
            quarantine_after_failures: 4,
            quarantine_secs: 300,
        }
    }

    #[test]
    fn new_work_is_immediately_eligible() {
        assert!(AttemptState::default().is_eligible(0));
        assert!(AttemptState::default().is_eligible(10_000));
    }

    #[test]
    fn failure_backoff_is_exponential_and_bounded() {
        let root = temporary_root("backoff");
        let store = AttemptStore::new(root.clone(), test_policy());
        let key = WorkKey::new("Memorithm/ADA", "ISSUE", 7);

        let first = store.record(&key, AttemptOutcome::Failure, 100).unwrap();
        assert_eq!(first.consecutive_failures, 1);
        assert_eq!(first.next_eligible_at, 110);

        let second = store.record(&key, AttemptOutcome::Failure, 200).unwrap();
        assert_eq!(second.consecutive_failures, 2);
        assert_eq!(second.next_eligible_at, 220);

        let third = store.record(&key, AttemptOutcome::Failure, 300).unwrap();
        assert_eq!(third.consecutive_failures, 3);
        assert_eq!(third.next_eligible_at, 340);

        let fourth = store.record(&key, AttemptOutcome::Failure, 400).unwrap();
        assert_eq!(fourth.next_eligible_at, 440);
        assert_eq!(fourth.quarantine_until, 700);
        assert!(!fourth.is_eligible(699));
        assert!(fourth.is_eligible(700));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn success_resets_failures_and_applies_rotation_cooldown() {
        let root = temporary_root("success");
        let store = AttemptStore::new(root.clone(), test_policy());
        let key = WorkKey::new("Memorithm/TDI", "ISSUE", 57);

        store.record(&key, AttemptOutcome::Failure, 100).unwrap();
        let success = store.record(&key, AttemptOutcome::Success, 200).unwrap();
        assert_eq!(success.consecutive_failures, 0);
        assert_eq!(success.quarantine_until, 0);
        assert_eq!(success.next_eligible_at, 230);
        assert!(!success.is_eligible(229));
        assert!(success.is_eligible(230));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn state_round_trips_through_atomic_store() {
        let root = temporary_root("roundtrip");
        let store = AttemptStore::new(root.clone(), test_policy());
        let key = WorkKey::new("Memorithm/FLAT-ATTENTION", "FIX_CI", 132);

        let recorded = store.record(&key, AttemptOutcome::Failure, 1234).unwrap();
        let loaded = store.load(&key).unwrap();
        assert_eq!(recorded, loaded);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn interrupted_attempt_is_recovered_as_failure() {
        let root = temporary_root("interrupted");
        let store = AttemptStore::new(root.clone(), test_policy());
        let key = WorkKey::new("Memorithm/ADA", "ISSUE", 7);

        let begun = store.begin(&key, 100).unwrap();
        assert_eq!(begun.in_progress_since, 100);
        assert_eq!(begun.total_attempts, 0);

        let recovered = store.recover_interrupted(&key, 120).unwrap().unwrap();
        assert_eq!(recovered.in_progress_since, 0);
        assert_eq!(recovered.total_attempts, 1);
        assert_eq!(recovered.consecutive_failures, 1);
        assert_eq!(recovered.last_outcome, Some(AttemptOutcome::Failure));
        assert_eq!(recovered.next_eligible_at, 130);

        assert!(store.recover_interrupted(&key, 121).unwrap().is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn v1_state_migrates_without_losing_retry_history() {
        let root = temporary_root("v1");
        let key = WorkKey::new("Memorithm/TDI", "ISSUE", 57);
        let path = key.state_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "v1\ntotal_attempts=3\nconsecutive_failures=2\nlast_attempt_at=100\nnext_eligible_at=140\nquarantine_until=0\nlast_outcome=failure\n",
        )
        .unwrap();

        let store = AttemptStore::new(root.clone(), test_policy());
        let loaded = store.load(&key).unwrap();
        assert_eq!(loaded.total_attempts, 3);
        assert_eq!(loaded.consecutive_failures, 2);
        assert_eq!(loaded.in_progress_since, 0);
        store.record(&key, AttemptOutcome::Success, 200).unwrap();
        let rewritten = fs::read_to_string(path).unwrap();
        assert!(rewritten.starts_with("v2\n"));
        assert!(rewritten.contains("in_progress_since=0\n"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_or_future_state_fails_closed() {
        let root = temporary_root("corrupt");
        let key = WorkKey::new("Memorithm/ADA", "ISSUE", 7);
        let path = key.state_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "v99\ntotal_attempts=1\n").unwrap();

        let store = AttemptStore::new(root.clone(), test_policy());
        assert!(store.load(&key).is_err());

        let _ = fs::remove_dir_all(root);
    }
}
