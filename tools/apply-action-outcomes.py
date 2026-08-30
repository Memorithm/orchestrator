#!/usr/bin/env python3
from pathlib import Path

main_path = Path("src/main.rs")
state_path = Path("src/state.rs")
health_path = Path("src/health.rs")
main = main_path.read_text()
state = state_path.read_text()
health = health_path.read_text()


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return source.replace(old, new, 1)


def transform_section(source: str, start: str, end: str, transform, label: str) -> str:
    start_index = source.find(start)
    if start_index < 0:
        raise SystemExit(f"{label}: start marker not found")
    end_index = source.find(end, start_index)
    if end_index < 0:
        raise SystemExit(f"{label}: end marker not found")
    section = source[start_index:end_index]
    changed = transform(section)
    if changed == section:
        raise SystemExit(f"{label}: transform produced no change")
    return source[:start_index] + changed + source[end_index:]


# ---------------------------------------------------------------------------
# Persistent retry state: schema v5 adds explicit no_progress/deferred outcomes.
# ---------------------------------------------------------------------------
state = replace_once(
    state,
    'const STATE_VERSION: &str = "v4";\nconst LEGACY_STATE_VERSIONS: &[&str] = &["v1", "v2", "v3"];\n',
    'const STATE_VERSION: &str = "v5";\nconst LEGACY_STATE_VERSIONS: &[&str] = &["v1", "v2", "v3", "v4"];\n',
    "state schema v5",
)

state = replace_once(
    state,
    '''pub(crate) enum AttemptOutcome {
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
''',
    '''pub(crate) enum AttemptOutcome {
    Success,
    NoProgress,
    Deferred,
    Failure,
}

impl AttemptOutcome {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::NoProgress => "no_progress",
            Self::Deferred => "deferred",
            Self::Failure => "failure",
        }
    }

    fn parse(value: &str) -> Result<Option<Self>, String> {
        match value {
            "" | "none" => Ok(None),
            "success" => Ok(Some(Self::Success)),
            "no_progress" => Ok(Some(Self::NoProgress)),
            "deferred" => Ok(Some(Self::Deferred)),
            "failure" => Ok(Some(Self::Failure)),
            other => Err(format!("unknown attempt outcome: {other}")),
        }
    }
}
''',
    "attempt outcomes",
)

state = replace_once(
    state,
    '''impl RetryPolicy {
    fn failure_cooldown(self, failures: u32) -> u64 {
        let exponent = failures.saturating_sub(1).min(31);
        let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
        self.failure_base_cooldown_secs
            .saturating_mul(multiplier)
            .min(self.failure_max_cooldown_secs)
    }
}
''',
    '''impl RetryPolicy {
    fn failure_cooldown(self, failures: u32) -> u64 {
        let exponent = failures.saturating_sub(1).min(31);
        let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
        self.failure_base_cooldown_secs
            .saturating_mul(multiplier)
            .min(self.failure_max_cooldown_secs)
    }

    pub(crate) fn no_progress_cooldown(self) -> u64 {
        self.success_cooldown_secs
            .saturating_mul(4)
            .max(self.transient_failure_cooldown_secs)
    }
}
''',
    "no progress cooldown policy",
)

state = replace_once(
    state,
    '''        match outcome {
            AttemptOutcome::Success => {
                let mut state = self.load_for_revision(key, revision)?;
                state.in_progress_since = 0;
                self.apply_success(&mut state, now);
                self.save(key, &state)?;
                Ok(state)
            }
            AttemptOutcome::Failure => {
                self.record_failure_for_revision(key, revision, FailureClass::Validation, now)
            }
        }
''',
    '''        match outcome {
            AttemptOutcome::Success | AttemptOutcome::NoProgress | AttemptOutcome::Deferred => {
                let mut state = self.load_for_revision(key, revision)?;
                state.in_progress_since = 0;
                match outcome {
                    AttemptOutcome::Success => self.apply_success(&mut state, now),
                    AttemptOutcome::NoProgress => self.apply_no_progress(&mut state, now),
                    AttemptOutcome::Deferred => self.apply_deferred(&mut state, now),
                    AttemptOutcome::Failure => unreachable!(),
                }
                self.save(key, &state)?;
                Ok(state)
            }
            AttemptOutcome::Failure => {
                self.record_failure_for_revision(key, revision, FailureClass::Validation, now)
            }
        }
''',
    "record non-failure outcomes",
)

state = replace_once(
    state,
    '''    fn apply_failure(&self, state: &mut AttemptState, class: FailureClass, now: u64) {
''',
    '''    fn apply_no_progress(&self, state: &mut AttemptState, now: u64) {
        state.total_attempts = state.total_attempts.saturating_add(1);
        state.last_attempt_at = now;
        state.last_outcome = Some(AttemptOutcome::NoProgress);
        state.last_failure_class = None;
        state.consecutive_failures = 0;
        state.quarantine_until = 0;
        state.next_eligible_at = now.saturating_add(self.policy.no_progress_cooldown());
    }

    fn apply_deferred(&self, state: &mut AttemptState, now: u64) {
        state.total_attempts = state.total_attempts.saturating_add(1);
        state.last_attempt_at = now;
        state.last_outcome = Some(AttemptOutcome::Deferred);
        state.last_failure_class = None;
        state.next_eligible_at =
            now.saturating_add(self.policy.transient_failure_cooldown_secs);
    }

    fn apply_failure(&self, state: &mut AttemptState, class: FailureClass, now: u64) {
''',
    "non-failure state transitions",
)

state = replace_once(
    state,
    '    let legacy_failure_class = version != STATE_VERSION;\n',
    '    let legacy_failure_class = matches!(version, "v1" | "v2" | "v3");\n',
    "v4 failure class compatibility",
)

# Old migration tests should now expect v5 serialization.
state = state.replace('starts_with("v4\\nrevision=', 'starts_with("v5\\nrevision=')

state = replace_once(
    state,
    '''    #[test]
    fn transient_failures_rotate_without_semantic_quarantine() {
''',
    '''    #[test]
    fn no_progress_is_distinct_and_uses_long_rotation_cooldown() {
        let root = temporary_root("no-progress");
        let store = AttemptStore::new(root.clone(), test_policy());
        let key = WorkKey::new("Memorithm/ADA", "ISSUE", 7);

        store
            .record_failure_for_revision(&key, "issue-v1", FailureClass::Validation, 100)
            .unwrap();
        let state = store
            .record_for_revision(&key, "issue-v1", AttemptOutcome::NoProgress, 200)
            .unwrap();
        assert_eq!(state.last_outcome, Some(AttemptOutcome::NoProgress));
        assert_eq!(state.last_failure_class, None);
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.quarantine_until, 0);
        assert_eq!(state.next_eligible_at, 320);
        assert!(!state.is_eligible(319));
        assert!(state.is_eligible(320));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn deferred_uses_transient_rotation_without_counting_a_failure() {
        let root = temporary_root("deferred");
        let store = AttemptStore::new(root.clone(), test_policy());
        let key = WorkKey::new("Memorithm/FLAT-ATTENTION", "PR_ATTENTION", 132);

        let state = store
            .record_for_revision(&key, "head-a", AttemptOutcome::Deferred, 100)
            .unwrap();
        assert_eq!(state.last_outcome, Some(AttemptOutcome::Deferred));
        assert_eq!(state.last_failure_class, None);
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.next_eligible_at, 105);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn v4_state_migrates_to_v5_with_existing_failure_class() {
        let root = temporary_root("v4-outcome");
        let key = WorkKey::new("Memorithm/scirust", "FIX_CI", 1338);
        let path = key.state_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "v4\\nrevision=head-a\\ntotal_attempts=2\\nconsecutive_failures=1\\nlast_attempt_at=100\\nnext_eligible_at=110\\nquarantine_until=0\\nin_progress_since=0\\nlast_outcome=failure\\nlast_failure_class=validation\\n",
        )
        .unwrap();
        let store = AttemptStore::new(root.clone(), test_policy());
        let loaded = store.load_for_revision(&key, "head-a").unwrap();
        assert_eq!(loaded.last_failure_class, Some(FailureClass::Validation));
        store
            .record_for_revision(&key, "head-a", AttemptOutcome::NoProgress, 200)
            .unwrap();
        let rewritten = fs::read_to_string(path).unwrap();
        assert!(rewritten.starts_with("v5\\nrevision=head-a\\n"));
        assert!(rewritten.contains("last_outcome=no_progress\\n"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn transient_failures_rotate_without_semantic_quarantine() {
''',
    "outcome state tests",
)

# ---------------------------------------------------------------------------
# Runtime execution outcome: distinguish real progress, no diff, and deferral.
# ---------------------------------------------------------------------------
main = replace_once(
    main,
    '''#[derive(Debug)]
struct ActionFailure {
''',
    '''#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionOutcome {
    Progress,
    NoProgress,
    Deferred,
}

impl ActionOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Progress => "progress",
            Self::NoProgress => "no_progress",
            Self::Deferred => "deferred",
        }
    }

    const fn state_outcome(self) -> state::AttemptOutcome {
        match self {
            Self::Progress => state::AttemptOutcome::Success,
            Self::NoProgress => state::AttemptOutcome::NoProgress,
            Self::Deferred => state::AttemptOutcome::Deferred,
        }
    }
}

#[derive(Debug)]
struct ActionFailure {
''',
    "action outcome enum",
)


def patch_execute_issue(section: str) -> str:
    section = replace_once(
        section,
        ') -> Result<(), ActionFailure> {\n',
        ') -> Result<ActionOutcome, ActionFailure> {\n',
        "execute_issue return type",
    )
    section = replace_once(
        section,
        '''        return resume_issue_publication(config, repository, item, &store, &key, pending)
            .classified(state::FailureClass::Publication);
''',
        '''        return resume_issue_publication(config, repository, item, &store, &key, pending)
            .classified(state::FailureClass::Publication)
            .map(|()| ActionOutcome::Progress);
''',
        "resume outcome",
    )
    section = replace_once(
        section,
        '''        println!("Agent produced no working-tree changes; nothing will be pushed.");
        return Ok(());
''',
        '''        println!("Agent produced no working-tree changes; recording NO_PROGRESS.");
        return Ok(ActionOutcome::NoProgress);
''',
        "issue no progress",
    )
    if not section.rstrip().endswith('Ok(())\n}'):
        raise SystemExit("execute_issue: final Ok(()) anchor not found")
    section = section.rstrip()[:-len('Ok(())\n}')] + 'Ok(ActionOutcome::Progress)\n}\n'
    return section


main = transform_section(
    main,
    'fn execute_issue(\n',
    'fn execute_ci_fix(',
    patch_execute_issue,
    "execute_issue",
)


def patch_execute_ci(section: str) -> str:
    section = replace_once(
        section,
        'fn execute_ci_fix(config: &RunConfig, item: &WorkItem) -> Result<(), ActionFailure> {\n',
        'fn execute_ci_fix(\n    config: &RunConfig,\n    item: &WorkItem,\n) -> Result<ActionOutcome, ActionFailure> {\n',
        "execute_ci return type",
    )
    section = replace_once(
        section,
        '''        println!("Agent produced no working-tree changes; CI may already have moved on.");
        return Ok(());
''',
        '''        println!("Agent produced no working-tree changes; recording NO_PROGRESS.");
        return Ok(ActionOutcome::NoProgress);
''',
        "ci no progress",
    )
    section = replace_once(
        section,
        '''    attest_repaired_pr_head(config, &workspace, item)
}
''',
        '''    attest_repaired_pr_head(config, &workspace, item)?;
    Ok(ActionOutcome::Progress)
}
''',
        "ci progress outcome",
    )
    return section


main = transform_section(
    main,
    'fn execute_ci_fix(',
    'fn merge_attestation_store(',
    patch_execute_ci,
    "execute_ci_fix",
)


def patch_pr_attention(section: str) -> str:
    section = replace_once(
        section,
        ') -> Result<(), ActionFailure> {\n',
        ') -> Result<ActionOutcome, ActionFailure> {\n',
        "pr attention return type",
    )
    section = replace_once(
        section,
        '''        return Ok(());
''',
        '''        return Ok(ActionOutcome::Deferred);
''',
        "auto merge disabled deferred",
    )
    section = replace_once(
        section,
        '''        return Ok(());
''',
        '''        return Ok(ActionOutcome::Deferred);
''',
        "outside merge scope deferred",
    )
    section = replace_once(
        section,
        '''    if status.success() {
        Ok(())
''',
        '''    if status.success() {
        Ok(ActionOutcome::Progress)
''',
        "merge progress",
    )
    return section


main = transform_section(
    main,
    'fn handle_pr_attention(\n',
    'fn runtime_preflight(',
    patch_pr_attention,
    "handle_pr_attention",
)


def patch_execute_item(section: str) -> str:
    section = replace_once(
        section,
        ') -> Result<(), ActionFailure> {\n',
        ') -> Result<ActionOutcome, ActionFailure> {\n',
        "execute_item return type",
    )
    section = replace_once(
        section,
        '        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => Ok(()),\n',
        '        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => {\n            Ok(ActionOutcome::Deferred)\n        }\n',
        "non-actionable deferred",
    )
    return section


main = transform_section(
    main,
    'fn execute_item(\n',
    'fn run_loop(',
    patch_execute_item,
    "execute_item",
)

main = replace_once(
    main,
    '''    println!(
        "transient retry  : {}s (infrastructure/publication)",
        config.retry_policy.transient_failure_cooldown_secs
    );
''',
    '''    println!(
        "transient retry  : {}s (infrastructure/publication/deferred)",
        config.retry_policy.transient_failure_cooldown_secs
    );
    println!(
        "no-progress wait : {}s",
        config.retry_policy.no_progress_cooldown()
    );
''',
    "runtime policy display",
)

main = replace_once(
    main,
    '''                        match execute_item(&config, &snapshot, item) {
                            Ok(()) => {
                                let finished_at = unix_timestamp();
                                if let Err(journal_error) = journal.record(
                                    trajectory::EventPhase::AttemptFinished,
                                    "success",
                                    "execution completed",
                                    finished_at,
                                ) {
                                    eprintln!("trajectory finalization failed: {journal_error}");
                                    return ExitCode::FAILURE;
                                }
                                match attempt_store.record_for_revision(
                                    &key,
                                    &revision,
                                    state::AttemptOutcome::Success,
                                    finished_at,
                                ) {
                                    Ok(attempt_state) => println!(
                                        "Scheduler recorded success; {}#{} next eligible at unix={}",
                                        item.repository,
                                        item.number,
                                        attempt_state.eligible_at()
                                    ),
                                    Err(state_error) => {
                                        eprintln!(
                                            "scheduler state write failed after success: {state_error}"
                                        );
                                        return ExitCode::FAILURE;
                                    }
                                }
                            }
''',
    '''                        match execute_item(&config, &snapshot, item) {
                            Ok(action_outcome) => {
                                let finished_at = unix_timestamp();
                                if let Err(journal_error) = journal.record(
                                    trajectory::EventPhase::AttemptFinished,
                                    action_outcome.as_str(),
                                    "execution completed",
                                    finished_at,
                                ) {
                                    eprintln!("trajectory finalization failed: {journal_error}");
                                    return ExitCode::FAILURE;
                                }
                                match attempt_store.record_for_revision(
                                    &key,
                                    &revision,
                                    action_outcome.state_outcome(),
                                    finished_at,
                                ) {
                                    Ok(attempt_state) => println!(
                                        "Scheduler recorded {}; {}#{} next eligible at unix={}",
                                        action_outcome.as_str(),
                                        item.repository,
                                        item.number,
                                        attempt_state.eligible_at()
                                    ),
                                    Err(state_error) => {
                                        eprintln!(
                                            "scheduler state write failed after {}: {state_error}",
                                            action_outcome.as_str()
                                        );
                                        return ExitCode::FAILURE;
                                    }
                                }
                            }
''',
    "run loop structured outcomes",
)

# ---------------------------------------------------------------------------
# Offline health: expose last outcome distribution and accept v5 state.
# ---------------------------------------------------------------------------
health = replace_once(
    health,
    '''struct WorkCounts {
    total: usize,
    ready: usize,
    cooldown: usize,
    quarantine: usize,
    in_progress: usize,
    corrupt: usize,
}
''',
    '''struct WorkCounts {
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
''',
    "health work outcome counts",
)

health = replace_once(
    health,
    '''    lines.push(format!(
        "work items       : total={} ready={} cooldown={} quarantine={} in_progress={} corrupt={}",
        work.total, work.ready, work.cooldown, work.quarantine, work.in_progress, work.corrupt
    ));
''',
    '''    lines.push(format!(
        "work items       : total={} ready={} cooldown={} quarantine={} in_progress={} corrupt={}",
        work.total, work.ready, work.cooldown, work.quarantine, work.in_progress, work.corrupt
    ));
    lines.push(format!(
        "last outcomes    : progress={} no_progress={} deferred={} failure={} unknown={}",
        work.progress, work.no_progress, work.deferred, work.failure, work.unknown_outcome
    ));
''',
    "health outcome line",
)

old_inspect_work = '''fn inspect_work_items(root: &Path, now: u64) -> (WorkCounts, bool) {
    let (files, overflow) = collect_files(root, Some("state"));
    let mut counts = WorkCounts {
        total: files.len(),
        ..WorkCounts::default()
    };
    for path in files {
        match parse_key_value_state(&path, &["v1", "v2", "v3", "v4"]) {
            Ok(fields) => {
                let next = numeric_field(&fields, "next_eligible_at");
                let quarantine = numeric_field(&fields, "quarantine_until");
                let in_progress = numeric_field(&fields, "in_progress_since");
                match (next, quarantine, in_progress) {
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
                    }
                    _ => counts.corrupt += 1,
                }
            }
            Err(_) => counts.corrupt += 1,
        }
    }
    let degraded = overflow || counts.corrupt != 0;
    (counts, degraded)
}
'''
new_inspect_work = '''fn inspect_work_items(root: &Path, now: u64) -> (WorkCounts, bool) {
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
'''
health = replace_once(health, old_inspect_work, new_inspect_work, "health v5 parser")

health = replace_once(
    health,
    '''    #[test]
    fn corrupt_state_marks_health_degraded() {
''',
    '''    #[test]
    fn explicit_action_outcomes_are_reported_offline() {
        let root = temp_root("outcomes");
        let state_root = root.join("state/work-items/Memorithm__ADA");
        fs::create_dir_all(&state_root).unwrap();
        let write = |name: &str, outcome: &str| {
            fs::write(
                state_root.join(name),
                format!(
                    "v5\\nrevision=issue-v1\\ntotal_attempts=1\\nconsecutive_failures=0\\nlast_attempt_at=50\\nnext_eligible_at=0\\nquarantine_until=0\\nin_progress_since=0\\nlast_outcome={outcome}\\nlast_failure_class=none\\n"
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
''',
    "health action outcome test",
)

main_path.write_text(main)
state_path.write_text(state)
health_path.write_text(health)
