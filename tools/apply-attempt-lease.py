#!/usr/bin/env python3
from pathlib import Path

state_path = Path("src/state.rs")
main_path = Path("src/main.rs")
state = state_path.read_text()
main = main_path.read_text()


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return source.replace(old, new, 1)

state = replace_once(
    state,
    'const STATE_VERSION: &str = "v1";\n',
    'const STATE_VERSION: &str = "v2";\nconst LEGACY_STATE_VERSION: &str = "v1";\n',
    "state version",
)

state = replace_once(
    state,
    '''    pub(crate) quarantine_until: u64,
    pub(crate) last_outcome: Option<AttemptOutcome>,
''',
    '''    pub(crate) quarantine_until: u64,
    pub(crate) in_progress_since: u64,
    pub(crate) last_outcome: Option<AttemptOutcome>,
''',
    "in progress field",
)

state = replace_once(
    state,
    '''    pub(crate) fn record(
        &self,
        key: &WorkKey,
        outcome: AttemptOutcome,
        now: u64,
    ) -> Result<AttemptState, String> {
        let mut state = self.load(key)?;
        state.total_attempts = state.total_attempts.saturating_add(1);
        state.last_attempt_at = now;
        state.last_outcome = Some(outcome);
''',
    '''    pub(crate) fn begin(&self, key: &WorkKey, now: u64) -> Result<AttemptState, String> {
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
''',
    "attempt lifecycle methods",
)

state = replace_once(
    state,
    '''        }

        self.save(key, &state)?;
        Ok(state)
    }

    fn save(&self, key: &WorkKey, state: &AttemptState) -> Result<(), String> {
''',
    '''        }
    }

    fn save(&self, key: &WorkKey, state: &AttemptState) -> Result<(), String> {
''',
    "remove old record tail",
)

state = replace_once(
    state,
    '''        "{STATE_VERSION}\\ntotal_attempts={}\\nconsecutive_failures={}\\nlast_attempt_at={}\\nnext_eligible_at={}\\nquarantine_until={}\\nlast_outcome={}\\n",
        state.total_attempts,
        state.consecutive_failures,
        state.last_attempt_at,
        state.next_eligible_at,
        state.quarantine_until,
        state.last_outcome.map_or("none", AttemptOutcome::as_str)
''',
    '''        "{STATE_VERSION}\\ntotal_attempts={}\\nconsecutive_failures={}\\nlast_attempt_at={}\\nnext_eligible_at={}\\nquarantine_until={}\\nin_progress_since={}\\nlast_outcome={}\\n",
        state.total_attempts,
        state.consecutive_failures,
        state.last_attempt_at,
        state.next_eligible_at,
        state.quarantine_until,
        state.in_progress_since,
        state.last_outcome.map_or("none", AttemptOutcome::as_str)
''',
    "state serialization",
)

state = replace_once(
    state,
    '''    let version = lines.next().unwrap_or_default();
    if version != STATE_VERSION {
        return Err(format!("unsupported state version: {version}"));
    }
''',
    '''    let version = lines.next().unwrap_or_default();
    if version != STATE_VERSION && version != LEGACY_STATE_VERSION {
        return Err(format!("unsupported state version: {version}"));
    }
    let legacy = version == LEGACY_STATE_VERSION;
''',
    "legacy parser",
)

state = replace_once(
    state,
    '''            "quarantine_until" => {
                state.quarantine_until = value
                    .parse()
                    .map_err(|error| format!("invalid quarantine_until {value}: {error}"))?;
            }
            "last_outcome" => state.last_outcome = AttemptOutcome::parse(value)?,
''',
    '''            "quarantine_until" => {
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
''',
    "lease parser",
)

state = replace_once(
    state,
    '''    #[test]
    fn corrupt_or_future_state_fails_closed() {
''',
    '''    #[test]
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
            "v1\\ntotal_attempts=3\\nconsecutive_failures=2\\nlast_attempt_at=100\\nnext_eligible_at=140\\nquarantine_until=0\\nlast_outcome=failure\\n",
        )
        .unwrap();

        let store = AttemptStore::new(root.clone(), test_policy());
        let loaded = store.load(&key).unwrap();
        assert_eq!(loaded.total_attempts, 3);
        assert_eq!(loaded.consecutive_failures, 2);
        assert_eq!(loaded.in_progress_since, 0);
        store.record(&key, AttemptOutcome::Success, 200).unwrap();
        let rewritten = fs::read_to_string(path).unwrap();
        assert!(rewritten.starts_with("v2\\n"));
        assert!(rewritten.contains("in_progress_since=0\\n"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_or_future_state_fails_closed() {
''',
    "lease tests",
)

main = replace_once(
    main,
    '''        let key = work_key(item);
        let attempt_state = attempt_store.load(&key)?;
        if attempt_state.is_eligible(now) {
            return Ok(Some(item));
        }
''',
    '''        let key = work_key(item);
        if let Some(recovered) = attempt_store.recover_interrupted(&key, now)? {
            println!(
                "Scheduler recovered interrupted attempt: {}#{} {} -> failure {}; next eligible at unix={}",
                item.repository,
                item.number,
                item.kind.as_str(),
                recovered.consecutive_failures,
                recovered.eligible_at()
            );
            continue;
        }
        let attempt_state = attempt_store.load(&key)?;
        if attempt_state.is_eligible(now) {
            return Ok(Some(item));
        }
''',
    "recover interrupted selection",
)

main = replace_once(
    main,
    '''                        match execute_item(&config, &snapshot, item) {
''',
    '''                        if let Err(state_error) = attempt_store.begin(&key, selection_time) {
                            eprintln!("scheduler failed to persist attempt lease: {state_error}");
                            return ExitCode::FAILURE;
                        }

                        match execute_item(&config, &snapshot, item) {
''',
    "persist attempt lease",
)

state_path.write_text(state)
main_path.write_text(main)
