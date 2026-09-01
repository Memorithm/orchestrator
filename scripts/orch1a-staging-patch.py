from pathlib import Path
import re


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    data = file.read_text()
    count = data.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}: {old[:100]!r}")
    file.write_text(data.replace(old, new, 1))


policy_path = Path("src/policy.rs")
policy = policy_path.read_text()

types_marker = "impl PolicySnapshot {\n"
types = r'''#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskEligibility {
    Allowed,
    Deferred(PolicyDenial),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicyDenial {
    item_id: String,
    field: &'static str,
    value: String,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
}

impl PolicyDenial {
    pub(crate) fn reason(&self, repository: &str, snapshot: &PolicySnapshot) -> String {
        format!(
            "repository={repository} policy item={} denies autonomous initiation via {}={} source=origin/{}:{} commit={} blob={} policy_identity={}",
            self.item_id,
            self.field,
            self.value,
            self.source_ref,
            self.source_path,
            self.source_commit,
            self.source_blob,
            snapshot.identity_token()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoadmapTaskRule {
    id: String,
    status: Option<String>,
    agent_policy: Option<String>,
    execution_policy: Option<String>,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
}

'''
if policy.count(types_marker) != 1:
    raise SystemExit("policy.rs: PolicySnapshot impl marker mismatch")
policy = policy.replace(types_marker, types + types_marker, 1)

method_marker = '''    pub(crate) fn base_branch(&self) -> &str {
'''
method = r'''    pub(crate) fn task_eligibility(
        &self,
        title: &str,
        body: &str,
    ) -> Result<TaskEligibility, String> {
        let mut rules = BTreeMap::<String, RoadmapTaskRule>::new();
        for document in &self.documents {
            for rule in parse_roadmap_task_rules(document)? {
                let canonical = canonical_policy_id(&rule.id)?;
                if let Some(existing) = rules.get(&canonical) {
                    if rule_is_denied(existing) != rule_is_denied(&rule) {
                        return Err(format!(
                            "conflicting task eligibility for roadmap id {} across mandatory policy documents",
                            rule.id
                        ));
                    }
                    continue;
                }
                rules.insert(canonical, rule);
            }
        }

        for (canonical, rule) in rules {
            if !task_mentions_policy_id(title, &canonical)
                && !task_mentions_policy_id(body, &canonical)
            {
                continue;
            }
            let Some((field, value)) = deny_basis(&rule) else {
                continue;
            };
            return Ok(TaskEligibility::Deferred(PolicyDenial {
                item_id: rule.id,
                field,
                value: value.to_owned(),
                source_ref: rule.source_ref,
                source_path: rule.source_path,
                source_commit: rule.source_commit,
                source_blob: rule.source_blob,
            }));
        }
        Ok(TaskEligibility::Allowed)
    }

'''
if policy.count(method_marker) != 1:
    raise SystemExit("policy.rs: base_branch marker mismatch")
policy = policy.replace(method_marker, method + method_marker, 1)

helpers_marker = "fn append_document(output: &mut String, kind: &str, document: &PolicyDocument) {\n"
helpers = r'''fn parse_roadmap_task_rules(document: &PolicyDocument) -> Result<Vec<RoadmapTaskRule>, String> {
    let mut in_roadmap = false;
    let mut current: Option<RoadmapTaskRule> = None;
    let mut rules = Vec::new();
    let mut ids = BTreeSet::new();

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_roadmap {
            if raw_line == "roadmap:" {
                in_roadmap = true;
            }
            continue;
        }

        if raw_line.starts_with('\t') {
            return Err(format!(
                "tab indentation is not allowed in recognized roadmap policy origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            break;
        }

        if indent == 2 && trimmed.starts_with("- id:") {
            if let Some(rule) = current.take() {
                finish_roadmap_rule(rule, &mut ids, &mut rules)?;
            }
            let id = parse_policy_scalar(trimmed[5..].trim(), "id")?;
            canonical_policy_id(&id)?;
            current = Some(RoadmapTaskRule {
                id,
                status: None,
                agent_policy: None,
                execution_policy: None,
                source_ref: document.ref_name.clone(),
                source_path: document.path.clone(),
                source_commit: document.commit_sha.clone(),
                source_blob: document.blob_sha.clone(),
            });
            continue;
        }
        if indent == 2 && trimmed.starts_with("- ") {
            return Err(format!(
                "recognized roadmap contains a top-level list item without an id in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        if indent != 4 {
            continue;
        }

        let Some((key, raw_value)) = trimmed.split_once(':') else {
            continue;
        };
        if !matches!(key, "status" | "agent_policy" | "execution_policy") {
            continue;
        }
        let rule = current.as_mut().ok_or_else(|| {
            format!(
                "roadmap field {key} appears before an id in origin/{}:{}",
                document.ref_name, document.path
            )
        })?;
        let value = parse_policy_scalar(raw_value.trim(), key)?;
        let slot = match key {
            "status" => &mut rule.status,
            "agent_policy" => &mut rule.agent_policy,
            "execution_policy" => &mut rule.execution_policy,
            _ => unreachable!(),
        };
        if slot.replace(value).is_some() {
            return Err(format!(
                "duplicate roadmap field {key} for id {} in origin/{}:{}",
                rule.id, document.ref_name, document.path
            ));
        }
    }

    if let Some(rule) = current {
        finish_roadmap_rule(rule, &mut ids, &mut rules)?;
    }
    Ok(rules)
}

fn finish_roadmap_rule(
    rule: RoadmapTaskRule,
    ids: &mut BTreeSet<String>,
    rules: &mut Vec<RoadmapTaskRule>,
) -> Result<(), String> {
    let canonical = canonical_policy_id(&rule.id)?;
    if !ids.insert(canonical) {
        return Err(format!("duplicate roadmap id {} in one mandatory policy document", rule.id));
    }
    rules.push(rule);
    Ok(())
}

fn parse_policy_scalar(value: &str, field: &str) -> Result<String, String> {
    if value.is_empty() || matches!(value, ">" | "|") || value.len() > 256 {
        return Err(format!("invalid scalar value for roadmap field {field}"));
    }
    let value = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(format!("invalid scalar value for roadmap field {field}"));
    }
    Ok(value.to_owned())
}

fn canonical_policy_id(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 128 {
        return Err(format!("invalid roadmap id: {value:?}"));
    }
    let mut canonical = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            canonical.push(character.to_ascii_uppercase());
        } else if !matches!(character, '_' | '-' | '.') {
            return Err(format!("invalid roadmap id: {value:?}"));
        }
    }
    if canonical.is_empty() {
        return Err(format!("invalid roadmap id: {value:?}"));
    }
    Ok(canonical)
}

fn task_mentions_policy_id(text: &str, canonical_id: &str) -> bool {
    text.split(|character: char| {
        !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    })
    .filter(|token| !token.is_empty())
    .any(|token| canonical_policy_id(token).is_ok_and(|candidate| candidate == canonical_id))
}

fn deny_basis(rule: &RoadmapTaskRule) -> Option<(&'static str, &str)> {
    if rule
        .agent_policy
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("forbidden_to_initiate"))
    {
        return Some(("agent_policy", rule.agent_policy.as_deref().unwrap_or_default()));
    }
    if rule
        .status
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("human_only_blocked"))
    {
        return Some(("status", rule.status.as_deref().unwrap_or_default()));
    }
    if rule
        .execution_policy
        .as_deref()
        .is_some_and(|value| value.to_ascii_lowercase().contains("human_only"))
    {
        return Some((
            "execution_policy",
            rule.execution_policy.as_deref().unwrap_or_default(),
        ));
    }
    None
}

fn rule_is_denied(rule: &RoadmapTaskRule) -> bool {
    deny_basis(rule).is_some()
}

'''
if policy.count(helpers_marker) != 1:
    raise SystemExit("policy.rs: append_document marker mismatch")
policy = policy.replace(helpers_marker, helpers + helpers_marker, 1)

# Add pure eligibility tests before the existing same-second evidence test.
test_marker = '''    #[test]
    fn base_branch_validation_accepts_master_and_nonstandard_defaults() {
'''
tests = r'''    fn snapshot_with_policy_documents(contents: &[&str]) -> PolicySnapshot {
        PolicySnapshot {
            repository: "Memorithm/Test".to_owned(),
            base_branch: "main".to_owned(),
            base_sha: "0".repeat(40),
            bootstrap: None,
            documents: contents
                .iter()
                .enumerate()
                .map(|(index, content)| PolicyDocument {
                    ref_name: format!("agent/policy-{index}"),
                    path: format!(".agent/POLICY-{index}.yaml"),
                    commit_sha: format!("{:040x}", index + 1),
                    blob_sha: format!("{:040x}", index + 101),
                    content: (*content).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn task_eligibility_blocks_explicit_human_only_roadmap_item() {
        let snapshot = snapshot_with_policy_documents(&[r#"schema_version: 1
roadmap:
  - id: TDI7_1
    status: complete_pending_human_confirmatory_execution
  - id: TDI7_2
    status: human_only_blocked
    execution_policy: explicit_human_only_confirmation_at_execution_time
    agent_policy: forbidden_to_initiate
  - id: TDIX
    status: planned_parallel
"#]);
        for spelling in ["TDI7_2", "TDI7.2", "TDI-7.2"] {
            let decision = snapshot
                .task_eligibility(&format!("Run {spelling} final holdout"), "")
                .unwrap();
            let TaskEligibility::Deferred(denial) = decision else {
                panic!("expected {spelling} to be denied");
            };
            assert_eq!(denial.item_id, "TDI7_2");
            assert_eq!(denial.field, "agent_policy");
        }
        assert_eq!(
            snapshot.task_eligibility("Advance TDIX evidence bridge", "").unwrap(),
            TaskEligibility::Allowed
        );
        assert_eq!(
            snapshot.task_eligibility("Audit TDI7_20 fixture", "").unwrap(),
            TaskEligibility::Allowed
        );
    }

    #[test]
    fn task_eligibility_rejects_duplicate_or_conflicting_structured_policy() {
        let duplicate = snapshot_with_policy_documents(&[r#"roadmap:
  - id: X1
    status: human_only_blocked
    status: active
"#]);
        assert!(duplicate.task_eligibility("X1", "").is_err());

        let conflicting = snapshot_with_policy_documents(&[
            "roadmap:\n  - id: X2\n    status: active\n",
            "roadmap:\n  - id: X2\n    agent_policy: forbidden_to_initiate\n",
        ]);
        assert!(conflicting.task_eligibility("X2", "").is_err());
    }

    #[test]
    fn free_text_is_not_promoted_into_task_policy() {
        let snapshot = snapshot_with_policy_documents(&[r#"schema_version: 1
notes: >-
  agent_policy: forbidden_to_initiate and human_only_blocked are words here,
  not a roadmap item.
financial_rule: agents must never authorize custody from model output
"#]);
        assert_eq!(
            snapshot
                .task_eligibility("financial custody analysis", "forbidden_to_initiate")
                .unwrap(),
            TaskEligibility::Allowed
        );
    }

'''
if policy.count(test_marker) != 1:
    raise SystemExit("policy.rs: test insertion marker mismatch")
policy = policy.replace(test_marker, tests + test_marker, 1)
policy_path.write_text(policy)

# main.rs: introduce an execution result carrying trajectory detail.
replace_once(
    "src/main.rs",
    '''impl ActionOutcome {
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
    '''impl ActionOutcome {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActionExecution {
    outcome: ActionOutcome,
    detail: String,
}

impl ActionExecution {
    fn completed(outcome: ActionOutcome) -> Self {
        Self {
            outcome,
            detail: "execution completed".to_owned(),
        }
    }

    fn deferred(reason: impl Into<String>) -> Self {
        Self {
            outcome: ActionOutcome::Deferred,
            detail: reason.into(),
        }
    }
}

#[derive(Debug)]
struct ActionFailure {
''',
)

main_path = Path("src/main.rs")
main = main_path.read_text()
issue_start = main.index("fn execute_issue(\n")
issue_end = main.index("\nfn execute_ci_fix(", issue_start)
issue = main[issue_start:issue_end]
issue = issue.replace(
    ") -> Result<ActionOutcome, ActionFailure> {",
    ") -> Result<ActionExecution, ActionFailure> {",
    1,
)
issue = re.sub(
    r"Ok\(ActionOutcome::(Progress|NoProgress|Deferred)\)",
    r"Ok(ActionExecution::completed(ActionOutcome::\1))",
    issue,
)
eligibility_anchor = '''    if !issue_revision_is_current(item, "after issue body read")
        .classified(state::FailureClass::Infrastructure)?
    {
        return Ok(ActionExecution::completed(ActionOutcome::Deferred));
    }
    if !issue_repository_is_current(repository, "before agent execution")
'''
eligibility_insert = '''    if !issue_revision_is_current(item, "after issue body read")
        .classified(state::FailureClass::Infrastructure)?
    {
        return Ok(ActionExecution::completed(ActionOutcome::Deferred));
    }
    match policy_snapshot
        .task_eligibility(&item.title, &body)
        .classified(state::FailureClass::Validation)?
    {
        policy::TaskEligibility::Allowed => {}
        policy::TaskEligibility::Deferred(denial) => {
            let reason = denial.reason(&item.repository, &policy_snapshot);
            println!("policy gate: DEFERRED {reason}");
            return Ok(ActionExecution::deferred(reason));
        }
    }
    if !issue_repository_is_current(repository, "before agent execution")
'''
if issue.count(eligibility_anchor) != 1:
    raise SystemExit(f"main.rs: eligibility anchor mismatch: {issue.count(eligibility_anchor)}")
issue = issue.replace(eligibility_anchor, eligibility_insert, 1)
main = main[:issue_start] + issue + main[issue_end:]
main_path.write_text(main)

# execute_item maps existing action outcomes into ActionExecution and preserves resource reason.
replace_once(
    "src/main.rs",
    '''fn execute_item(
    config: &RunConfig,
    snapshot: &TriageSnapshot,
    item: &WorkItem,
) -> Result<ActionOutcome, ActionFailure> {
''',
    '''fn execute_item(
    config: &RunConfig,
    snapshot: &TriageSnapshot,
    item: &WorkItem,
) -> Result<ActionExecution, ActionFailure> {
''',
)
replace_once(
    "src/main.rs",
    '''            return Ok(ActionOutcome::Deferred);
        }
    }

    match item.kind {
        WorkKind::FixCi => execute_ci_fix(config, item),
        WorkKind::PullRequest => {
            let repository = repository_by_name(&snapshot.repositories, &item.repository)
                .classified(state::FailureClass::Repository)?;
            handle_pr_attention(config, repository, item)
        }
        WorkKind::Issue => execute_issue(config, &snapshot.repositories, item),
        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::NoChecks | WorkKind::UnknownCi => {
            Ok(ActionOutcome::Deferred)
        }
    }
}
''',
    '''            return Ok(ActionExecution::deferred(format!("resource gate: {reason}")));
        }
    }

    match item.kind {
        WorkKind::FixCi => execute_ci_fix(config, item).map(ActionExecution::completed),
        WorkKind::PullRequest => {
            let repository = repository_by_name(&snapshot.repositories, &item.repository)
                .classified(state::FailureClass::Repository)?;
            handle_pr_attention(config, repository, item).map(ActionExecution::completed)
        }
        WorkKind::Issue => execute_issue(config, &snapshot.repositories, item),
        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::NoChecks | WorkKind::UnknownCi => {
            Ok(ActionExecution::deferred("work kind is not actionable"))
        }
    }
}
''',
)

# Scheduler records the detailed defer reason without turning it into a failure.
replace_once(
    "src/main.rs",
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
    '''                        match execute_item(&config, &snapshot, item) {
                            Ok(action_execution) => {
                                let finished_at = unix_timestamp();
                                if let Err(journal_error) = journal.record(
                                    trajectory::EventPhase::AttemptFinished,
                                    action_execution.outcome.as_str(),
                                    &action_execution.detail,
                                    finished_at,
                                ) {
                                    eprintln!("trajectory finalization failed: {journal_error}");
                                    return ExitCode::FAILURE;
                                }
                                match attempt_store.record_for_revision(
                                    &key,
                                    &revision,
                                    action_execution.outcome.state_outcome(),
                                    finished_at,
                                ) {
                                    Ok(attempt_state) => println!(
                                        "Scheduler recorded {}; {}#{} next eligible at unix={} detail={}",
                                        action_execution.outcome.as_str(),
                                        item.repository,
                                        item.number,
                                        attempt_state.eligible_at(),
                                        action_execution.detail
                                    ),
                                    Err(state_error) => {
                                        eprintln!(
                                            "scheduler state write failed after {}: {state_error}",
                                            action_execution.outcome.as_str()
                                        );
                                        return ExitCode::FAILURE;
                                    }
                                }
                            }
''',
)
