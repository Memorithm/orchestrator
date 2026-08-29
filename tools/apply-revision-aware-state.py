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
    'const STATE_VERSION: &str = "v2";\nconst LEGACY_STATE_VERSION: &str = "v1";\n',
    'const STATE_VERSION: &str = "v3";\nconst LEGACY_STATE_VERSIONS: &[&str] = &["v1", "v2"];\nconst LEGACY_REVISION: &str = "legacy";\n',
    "state version v3",
)

state = replace_once(
    state,
    '''pub(crate) struct AttemptState {
    pub(crate) total_attempts: u64,
''',
    '''pub(crate) struct AttemptState {
    pub(crate) revision: String,
    pub(crate) total_attempts: u64,
''',
    "attempt revision field",
)

old_load = '''    pub(crate) fn load(&self, key: &WorkKey) -> Result<AttemptState, String> {
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
'''
new_load = '''    pub(crate) fn load(&self, key: &WorkKey) -> Result<AttemptState, String> {
        self.load_for_revision(key, LEGACY_REVISION)
    }

    pub(crate) fn load_for_revision(
        &self,
        key: &WorkKey,
        revision: &str,
    ) -> Result<AttemptState, String> {
        validate_revision(revision)?;
        let path = key.state_path(&self.root);
        if !path.exists() {
            return Ok(fresh_state(revision));
        }
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read attempt state {}: {error}", path.display()))?;
        let mut state = parse_state(&contents)
            .map_err(|error| format!("invalid attempt state {}: {error}", path.display()))?;
        if state.revision.is_empty() {
            state.revision = revision.to_owned();
            return Ok(state);
        }
        if state.revision != revision {
            return Ok(fresh_state(revision));
        }
        Ok(state)
    }

    pub(crate) fn begin(&self, key: &WorkKey, now: u64) -> Result<AttemptState, String> {
        self.begin_for_revision(key, LEGACY_REVISION, now)
    }

    pub(crate) fn begin_for_revision(
        &self,
        key: &WorkKey,
        revision: &str,
        now: u64,
    ) -> Result<AttemptState, String> {
        let mut state = self.load_for_revision(key, revision)?;
'''
state = replace_once(state, old_load, new_load, "revision-aware load and begin")

state = replace_once(
    state,
    '''    pub(crate) fn recover_interrupted(
        &self,
        key: &WorkKey,
        now: u64,
    ) -> Result<Option<AttemptState>, String> {
        let mut state = self.load(key)?;
''',
    '''    pub(crate) fn recover_interrupted(
        &self,
        key: &WorkKey,
        now: u64,
    ) -> Result<Option<AttemptState>, String> {
        self.recover_interrupted_for_revision(key, LEGACY_REVISION, now)
    }

    pub(crate) fn recover_interrupted_for_revision(
        &self,
        key: &WorkKey,
        revision: &str,
        now: u64,
    ) -> Result<Option<AttemptState>, String> {
        let mut state = self.load_for_revision(key, revision)?;
''',
    "revision-aware interruption recovery",
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
''',
    '''    pub(crate) fn record(
        &self,
        key: &WorkKey,
        outcome: AttemptOutcome,
        now: u64,
    ) -> Result<AttemptState, String> {
        self.record_for_revision(key, LEGACY_REVISION, outcome, now)
    }

    pub(crate) fn record_for_revision(
        &self,
        key: &WorkKey,
        revision: &str,
        outcome: AttemptOutcome,
        now: u64,
    ) -> Result<AttemptState, String> {
        let mut state = self.load_for_revision(key, revision)?;
''',
    "revision-aware record",
)

state = replace_once(
    state,
    '''    fn save(&self, key: &WorkKey, state: &AttemptState) -> Result<(), String> {
        let path = key.state_path(&self.root);
''',
    '''    fn save(&self, key: &WorkKey, state: &AttemptState) -> Result<(), String> {
        validate_revision(&state.revision)?;
        let path = key.state_path(&self.root);
''',
    "revision validation on save",
)

state = replace_once(
    state,
    '''fn serialize_state(state: &AttemptState) -> String {
    format!(
        "{STATE_VERSION}\\ntotal_attempts={}\\nconsecutive_failures={}\\nlast_attempt_at={}\\nnext_eligible_at={}\\nquarantine_until={}\\nin_progress_since={}\\nlast_outcome={}\\n",
        state.total_attempts,
''',
    '''fn fresh_state(revision: &str) -> AttemptState {
    AttemptState {
        revision: revision.to_owned(),
        ..AttemptState::default()
    }
}

fn validate_revision(revision: &str) -> Result<(), String> {
    if revision.is_empty()
        || revision.len() > 256
        || revision.bytes().any(|byte| matches!(byte, b'\\n' | b'\\r' | 0))
    {
        return Err("invalid work revision".to_owned());
    }
    Ok(())
}

fn serialize_state(state: &AttemptState) -> String {
    format!(
        "{STATE_VERSION}\\nrevision={}\\ntotal_attempts={}\\nconsecutive_failures={}\\nlast_attempt_at={}\\nnext_eligible_at={}\\nquarantine_until={}\\nin_progress_since={}\\nlast_outcome={}\\n",
        state.revision,
        state.total_attempts,
''',
    "v3 serialization",
)

state = replace_once(
    state,
    '''    let version = lines.next().unwrap_or_default();
    if version != STATE_VERSION && version != LEGACY_STATE_VERSION {
        return Err(format!("unsupported state version: {version}"));
    }
    let legacy = version == LEGACY_STATE_VERSION;
''',
    '''    let version = lines.next().unwrap_or_default();
    if version != STATE_VERSION && !LEGACY_STATE_VERSIONS.contains(&version) {
        return Err(format!("unsupported state version: {version}"));
    }
    let legacy_v1 = version == "v1";
    let legacy_revision = version != STATE_VERSION;
''',
    "v3 parser version",
)

state = replace_once(
    state,
    '''        match name {
            "total_attempts" => {
''',
    '''        match name {
            "revision" => {
                if legacy_revision {
                    return Err(format!("{version} state cannot contain revision"));
                }
                validate_revision(value)?;
                state.revision = value.to_owned();
            }
            "total_attempts" => {
''',
    "revision parser field",
)

state = state.replace(
    '''                if legacy {
                    return Err("v1 state cannot contain in_progress_since".to_owned());
                }
''',
    '''                if legacy_v1 {
                    return Err("v1 state cannot contain in_progress_since".to_owned());
                }
''',
    1,
)

state = replace_once(
    state,
    '''    Ok(state)
}

#[cfg(test)]
''',
    '''    if version == STATE_VERSION && state.revision.is_empty() {
        return Err("v3 state missing revision".to_owned());
    }
    Ok(state)
}

#[cfg(test)]
''',
    "require v3 revision",
)

state = replace_once(
    state,
    '''    #[test]
    fn corrupt_or_future_state_fails_closed() {
''',
    '''    #[test]
    fn revision_change_resets_cooldown_and_failure_history() {
        let root = temporary_root("revision-reset");
        let store = AttemptStore::new(root.clone(), test_policy());
        let key = WorkKey::new("Memorithm/scirust", "FIX_CI", 1338);

        let failed = store
            .record_for_revision(&key, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", AttemptOutcome::Failure, 100)
            .unwrap();
        assert_eq!(failed.consecutive_failures, 1);
        assert!(!failed.is_eligible(105));

        let fresh = store
            .load_for_revision(&key, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .unwrap();
        assert_eq!(fresh.revision, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert_eq!(fresh.total_attempts, 0);
        assert_eq!(fresh.consecutive_failures, 0);
        assert!(fresh.is_eligible(105));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn v2_state_adopts_current_revision_without_losing_history() {
        let root = temporary_root("v2-revision");
        let key = WorkKey::new("Memorithm/scirust", "FIX_CI", 1338);
        let path = key.state_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "v2\\ntotal_attempts=2\\nconsecutive_failures=1\\nlast_attempt_at=100\\nnext_eligible_at=110\\nquarantine_until=0\\nin_progress_since=0\\nlast_outcome=failure\\n",
        )
        .unwrap();
        let store = AttemptStore::new(root.clone(), test_policy());
        let loaded = store.load_for_revision(&key, "head-a").unwrap();
        assert_eq!(loaded.revision, "head-a");
        assert_eq!(loaded.total_attempts, 2);
        assert_eq!(loaded.consecutive_failures, 1);
        store
            .record_for_revision(&key, "head-a", AttemptOutcome::Success, 200)
            .unwrap();
        let rewritten = fs::read_to_string(path).unwrap();
        assert!(rewritten.starts_with("v3\\nrevision=head-a\\n"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_or_future_state_fails_closed() {
''',
    "revision tests",
)

# Main runtime: revision is fetched only for actionable work. Issues use a stable logical revision.
main = replace_once(
    main,
    '''fn work_key(item: &WorkItem) -> state::WorkKey {
    state::WorkKey::new(&item.repository, item.kind.as_str(), item.number)
}
''',
    '''fn work_key(item: &WorkItem) -> state::WorkKey {
    state::WorkKey::new(&item.repository, item.kind.as_str(), item.number)
}

fn work_revision(item: &WorkItem) -> Result<String, String> {
    match item.kind {
        WorkKind::Issue => Ok("issue-v1".to_owned()),
        WorkKind::FixCi | WorkKind::PullRequest => pr_head_sha(&item.repository, item.number),
        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => {
            Ok("non-actionable".to_owned())
        }
    }
}
''',
    "work revision helper",
)

main = replace_once(
    main,
    '''        let key = work_key(item);
        if let Some(recovered) = attempt_store.recover_interrupted(&key, now)? {
''',
    '''        let key = work_key(item);
        let revision = work_revision(item)?;
        if let Some(recovered) = attempt_store.recover_interrupted_for_revision(&key, &revision, now)? {
''',
    "selector revision recovery",
)

main = replace_once(
    main,
    '''        let attempt_state = attempt_store.load(&key)?;
''',
    '''        let attempt_state = attempt_store.load_for_revision(&key, &revision)?;
''',
    "selector revision load",
)

main = replace_once(
    main,
    '''                        let key = work_key(item);
                        let mut journal = match trajectory::AttemptJournal::create(
''',
    '''                        let key = work_key(item);
                        let revision = match work_revision(item) {
                            Ok(revision) => revision,
                            Err(revision_error) => {
                                eprintln!("failed to resolve selected work revision: {revision_error}");
                                return ExitCode::FAILURE;
                            }
                        };
                        let mut journal = match trajectory::AttemptJournal::create(
''',
    "selected revision capture",
)

main = replace_once(
    main,
    '''                        if let Err(state_error) = attempt_store.begin(&key, selection_time) {
''',
    '''                        if let Err(state_error) =
                            attempt_store.begin_for_revision(&key, &revision, selection_time)
                        {
''',
    "revision begin",
)

main = main.replace(
    '''                                match attempt_store.record(
                                    &key,
                                    state::AttemptOutcome::Success,
                                    finished_at,
                                ) {
''',
    '''                                match attempt_store.record_for_revision(
                                    &key,
                                    &revision,
                                    state::AttemptOutcome::Success,
                                    finished_at,
                                ) {
''',
    1,
)
main = main.replace(
    '''                                match attempt_store.record(
                                    &key,
                                    state::AttemptOutcome::Failure,
                                    finished_at,
                                ) {
''',
    '''                                match attempt_store.record_for_revision(
                                    &key,
                                    &revision,
                                    state::AttemptOutcome::Failure,
                                    finished_at,
                                ) {
''',
    1,
)

state_path.write_text(state)
main_path.write_text(main)
