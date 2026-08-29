#!/usr/bin/env python3
from pathlib import Path

main_path = Path("src/main.rs")
state_path = Path("src/state.rs")
main = main_path.read_text()
state = state_path.read_text()

old_selector = '''    fn selected_for_run(&self, auto_merge: bool) -> Option<&WorkItem> {
        self.items.iter().find(|item| match item.kind {
            WorkKind::FixCi | WorkKind::Issue => true,
            WorkKind::PullRequest => auto_merge,
            WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => false,
        })
    }
'''
if main.count(old_selector) != 1:
    raise SystemExit("old selected_for_run method was not found exactly once")
main = main.replace(old_selector, "", 1)

start_marker = '''    #[test]
    fn run_selection_skips_green_pr_when_auto_merge_is_disabled() {'''
end_marker = '''    #[test]
    fn parses_pull_request_and_issue() {'''
start = main.find(start_marker)
end = main.find(end_marker, start + 1)
if start < 0 or end < 0:
    raise SystemExit("selection test boundaries not found")
replacement = '''    #[test]
    fn runtime_policy_skips_green_pr_when_auto_merge_is_disabled() {
        let green_pr = WorkItem {
            kind: WorkKind::PullRequest,
            repository: "Memorithm/AAA".to_owned(),
            number: 1,
            title: "green PR".to_owned(),
            detail: "ci=PASSING ready".to_owned(),
            ci_state: Some(CiState::Passing),
            draft: false,
        };
        let issue = WorkItem {
            kind: WorkKind::Issue,
            repository: "Memorithm/BBB".to_owned(),
            number: 2,
            title: "next issue".to_owned(),
            detail: "open issue".to_owned(),
            ci_state: None,
            draft: false,
        };

        assert!(!work_item_runnable(&green_pr, false));
        assert!(work_item_runnable(&green_pr, true));
        assert!(work_item_runnable(&issue, false));
    }

'''
main = main[:start] + replacement + main[end:]

old_derive = "#[derive(Debug, Clone, PartialEq, Eq)]\npub(crate) struct AttemptState {"
new_derive = "#[derive(Debug, Clone, Default, PartialEq, Eq)]\npub(crate) struct AttemptState {"
if state.count(old_derive) != 1:
    raise SystemExit("AttemptState derive marker not found exactly once")
state = state.replace(old_derive, new_derive, 1)

old_default = '''impl Default for AttemptState {
    fn default() -> Self {
        Self {
            total_attempts: 0,
            consecutive_failures: 0,
            last_attempt_at: 0,
            next_eligible_at: 0,
            quarantine_until: 0,
            last_outcome: None,
        }
    }
}

'''
if state.count(old_default) != 1:
    raise SystemExit("AttemptState manual Default impl not found exactly once")
state = state.replace(old_default, "", 1)

old_store_helper = '''    pub(crate) fn is_eligible(&self, key: &WorkKey, now: u64) -> Result<bool, String> {
        Ok(self.load(key)?.is_eligible(now))
    }

'''
if state.count(old_store_helper) != 1:
    raise SystemExit("redundant AttemptStore::is_eligible helper not found exactly once")
state = state.replace(old_store_helper, "", 1)

main_path.write_text(main)
state_path.write_text(state)
