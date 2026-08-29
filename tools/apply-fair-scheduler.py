#!/usr/bin/env python3
from pathlib import Path

path = Path("src/main.rs")
source = path.read_text()


def replace_once(old: str, new: str, label: str) -> None:
    global source
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    source = source.replace(old, new, 1)


old_selector = '''fn selected_for_run_with_state<'a>(
    snapshot: &'a TriageSnapshot,
    auto_merge: bool,
    attempt_store: &state::AttemptStore,
    now: u64,
) -> Result<Option<&'a WorkItem>, String> {
    for item in &snapshot.items {
        if !work_item_runnable(item, auto_merge) {
            continue;
        }

        let key = work_key(item);
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

        println!(
            "Scheduler cooldown: {}#{} {} deferred until unix={} (failures={})",
            item.repository,
            item.number,
            item.kind.as_str(),
            attempt_state.eligible_at(),
            attempt_state.consecutive_failures
        );
    }
    Ok(None)
}
'''

new_selector = '''fn selected_for_run_with_state<'a>(
    snapshot: &'a TriageSnapshot,
    auto_merge: bool,
    attempt_store: &state::AttemptStore,
    now: u64,
) -> Result<Option<&'a WorkItem>, String> {
    let mut eligible = Vec::new();
    for item in &snapshot.items {
        if !work_item_runnable(item, auto_merge) {
            continue;
        }

        let key = work_key(item);
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
            eligible.push((item, attempt_state.last_attempt_at));
        } else {
            println!(
                "Scheduler cooldown: {}#{} {} deferred until unix={} (failures={})",
                item.repository,
                item.number,
                item.kind.as_str(),
                attempt_state.eligible_at(),
                attempt_state.consecutive_failures
            );
        }
    }

    eligible.sort_by(|(left_item, left_last), (right_item, right_last)| {
        left_item
            .kind
            .rank()
            .cmp(&right_item.kind.rank())
            .then(left_last.cmp(right_last))
            .then(left_item.repository.cmp(&right_item.repository))
            .then(left_item.number.cmp(&right_item.number))
    });
    Ok(eligible.first().map(|(item, _)| *item))
}
'''
replace_once(old_selector, new_selector, "fair selector")

insert_before = '''    #[test]
    fn state_aware_selection_skips_cooling_priority_item() {
'''
new_tests = '''    #[test]
    fn fair_selection_prefers_never_attempted_item_within_same_priority() {
        let root = std::env::temp_dir().join(format!(
            "orchestrator-fair-selection-test-{}-{}",
            std::process::id(),
            unix_timestamp()
        ));
        let policy = state::RetryPolicy {
            success_cooldown_secs: 1,
            failure_base_cooldown_secs: 1,
            failure_max_cooldown_secs: 1,
            quarantine_after_failures: 4,
            quarantine_secs: 60,
        };
        let store = state::AttemptStore::new(root.clone(), policy);
        let earlier_alpha = WorkItem {
            kind: WorkKind::Issue,
            repository: "Memorithm/AAA".to_owned(),
            number: 1,
            title: "already attempted".to_owned(),
            detail: "open issue".to_owned(),
            ci_state: None,
            draft: false,
        };
        let later_alpha = WorkItem {
            kind: WorkKind::Issue,
            repository: "Memorithm/ZZZ".to_owned(),
            number: 2,
            title: "never attempted".to_owned(),
            detail: "open issue".to_owned(),
            ci_state: None,
            draft: false,
        };
        store
            .record(&work_key(&earlier_alpha), state::AttemptOutcome::Success, 100)
            .unwrap();
        let snapshot = TriageSnapshot {
            repositories: Vec::new(),
            items: vec![earlier_alpha, later_alpha],
            eligible_count: 2,
            repositories_with_open_pr: 0,
        };

        let selected = selected_for_run_with_state(&snapshot, false, &store, 200)
            .unwrap()
            .unwrap();
        assert_eq!(selected.repository, "Memorithm/ZZZ");
        assert_eq!(selected.number, 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fair_selection_never_sacrifices_work_kind_priority() {
        let root = std::env::temp_dir().join(format!(
            "orchestrator-fair-priority-test-{}-{}",
            std::process::id(),
            unix_timestamp()
        ));
        let policy = state::RetryPolicy {
            success_cooldown_secs: 1,
            failure_base_cooldown_secs: 1,
            failure_max_cooldown_secs: 1,
            quarantine_after_failures: 4,
            quarantine_secs: 60,
        };
        let store = state::AttemptStore::new(root.clone(), policy);
        let ci = WorkItem {
            kind: WorkKind::FixCi,
            repository: "Memorithm/ZZZ".to_owned(),
            number: 3,
            title: "CI".to_owned(),
            detail: "ci=FAILED ready".to_owned(),
            ci_state: Some(CiState::Failed),
            draft: false,
        };
        let issue = WorkItem {
            kind: WorkKind::Issue,
            repository: "Memorithm/AAA".to_owned(),
            number: 1,
            title: "issue".to_owned(),
            detail: "open issue".to_owned(),
            ci_state: None,
            draft: false,
        };
        store
            .record(&work_key(&ci), state::AttemptOutcome::Success, 100)
            .unwrap();
        let snapshot = TriageSnapshot {
            repositories: Vec::new(),
            items: vec![issue, ci],
            eligible_count: 2,
            repositories_with_open_pr: 0,
        };

        let selected = selected_for_run_with_state(&snapshot, false, &store, 200)
            .unwrap()
            .unwrap();
        assert_eq!(selected.kind, WorkKind::FixCi);
        assert_eq!(selected.number, 3);
        let _ = fs::remove_dir_all(root);
    }

'''
replace_once(insert_before, new_tests + insert_before, "fairness tests")

path.write_text(source)
