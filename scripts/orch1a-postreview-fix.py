from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    data = file.read_text()
    count = data.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}: {old[:120]!r}")
    file.write_text(data.replace(old, new, 1))


replace_once(
    "src/policy.rs",
    '''            if !task_mentions_policy_id(title, &canonical)
                && !task_mentions_policy_id(body, &canonical)
            {
''',
    '''            if !task_mentions_policy_id(title, &canonical)
                && !body_targets_policy_id(body, &canonical)
            {
''',
)

replace_once(
    "src/policy.rs",
    '''fn deny_basis(rule: &RoadmapTaskRule) -> Option<(&'static str, &str)> {
''',
    '''fn body_targets_policy_id(body: &str, canonical_id: &str) -> bool {
    body.lines().any(|line| {
        body_target_value(line)
            .is_some_and(|value| task_mentions_policy_id(value, canonical_id))
    })
}

fn body_target_value(line: &str) -> Option<&str> {
    let line = line
        .trim()
        .trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, '-' | '*' | '>' | '#')
        });
    let (label, value) = line.split_once(':')?;
    let label = label.trim().trim_matches('*').to_ascii_lowercase();
    if !matches!(
        label.as_str(),
        "target" | "current target" | "roadmap" | "roadmap item" | "milestone" | "stage" | "work item"
    ) {
        return None;
    }
    let value = value.trim().trim_start_matches('*').trim();
    (!value.is_empty()).then_some(value)
}

fn deny_basis(rule: &RoadmapTaskRule) -> Option<(&'static str, &str)> {
''',
)

replace_once(
    "src/policy.rs",
    '''    #[test]
    fn task_eligibility_rejects_duplicate_or_conflicting_structured_policy() {
''',
    '''    #[test]
    fn contextual_body_mentions_do_not_target_a_prohibited_item() {
        let snapshot = snapshot_with_policy_documents(&[r#"roadmap:
  - id: TDI7_1
    status: active
  - id: TDI7_2
    status: human_only_blocked
    agent_policy: forbidden_to_initiate
"#]);
        let broad_issue_body = r#"# Active research programme — TDI-7.x
## TDI-7.1 — deterministic evaluator
CI proves normal tests cannot produce TDI-7.2 results.
TDI-7.1 must stop before final holdout execution.
## TDI-7.2 — confirmatory result
The final TDI-7.2 holdout remains blocked.
Current development target:
- TDI-7.1 deterministic evaluator.
"#;
        assert_eq!(
            snapshot
                .task_eligibility(
                    "TDI-7.x — dynamic recovery diagnostics for attention",
                    broad_issue_body,
                )
                .unwrap(),
            TaskEligibility::Allowed
        );

        let targeted = snapshot
            .task_eligibility("Run confirmatory evaluation", "Target: TDI-7.2 final holdout")
            .unwrap();
        assert!(matches!(targeted, TaskEligibility::Deferred(_)));
    }

    #[test]
    fn task_eligibility_rejects_duplicate_or_conflicting_structured_policy() {
''',
)
