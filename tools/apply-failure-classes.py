#!/usr/bin/env python3
from pathlib import Path

main_path = Path("src/main.rs")
state_path = Path("src/state.rs")
service_path = Path("scripts/install-systemd.sh")
main = main_path.read_text()
state = state_path.read_text()
service = service_path.read_text()


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return source.replace(old, new, 1)

# ---- Persistent retry state v3 with failure classes. ----
state = replace_once(
    state,
    'const STATE_VERSION: &str = "v2";\nconst LEGACY_STATE_VERSION: &str = "v1";\n',
    'const STATE_VERSION: &str = "v3";\nconst LEGACY_STATE_V1: &str = "v1";\nconst LEGACY_STATE_V2: &str = "v2";\n',
    "state versions",
)

attempt_outcome_impl = '''impl AttemptOutcome {
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
'''
failure_class = attempt_outcome_impl + '''
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    Agent,
    Validation,
    Publication,
    Repository,
    Infrastructure,
}

impl FailureClass {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Validation => "validation",
            Self::Publication => "publication",
            Self::Repository => "repository",
            Self::Infrastructure => "infrastructure",
        }
    }

    fn parse(value: &str) -> Result<Option<Self>, String> {
        match value {
            "" | "none" => Ok(None),
            "agent" => Ok(Some(Self::Agent)),
            "validation" => Ok(Some(Self::Validation)),
            "publication" => Ok(Some(Self::Publication)),
            "repository" => Ok(Some(Self::Repository)),
            "infrastructure" => Ok(Some(Self::Infrastructure)),
            other => Err(format!("unknown failure class: {other}")),
        }
    }

    const fn transient(self) -> bool {
        matches!(self, Self::Publication | Self::Infrastructure)
    }
}
'''
state = replace_once(state, attempt_outcome_impl, failure_class, "failure class enum")

state = replace_once(
    state,
    '''    pub(crate) in_progress_since: u64,
    pub(crate) last_outcome: Option<AttemptOutcome>,
''',
    '''    pub(crate) in_progress_since: u64,
    pub(crate) last_outcome: Option<AttemptOutcome>,
    pub(crate) last_failure_class: Option<FailureClass>,
''',
    "failure class state field",
)

state = replace_once(
    state,
    '''    pub(crate) failure_max_cooldown_secs: u64,
    pub(crate) quarantine_after_failures: u32,
''',
    '''    pub(crate) failure_max_cooldown_secs: u64,
    pub(crate) transient_failure_cooldown_secs: u64,
    pub(crate) quarantine_after_failures: u32,
''',
    "transient retry policy field",
)
state = replace_once(
    state,
    '''            failure_max_cooldown_secs: 7_200,
            quarantine_after_failures: 4,
''',
    '''            failure_max_cooldown_secs: 7_200,
            transient_failure_cooldown_secs: 180,
            quarantine_after_failures: 4,
''',
    "transient retry default",
)

old_lifecycle = '''    pub(crate) fn recover_interrupted(
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
'''
new_lifecycle = '''    pub(crate) fn recover_interrupted(
        &self,
        key: &WorkKey,
        now: u64,
    ) -> Result<Option<AttemptState>, String> {
        let mut state = self.load(key)?;
        if state.in_progress_since == 0 {
            return Ok(None);
        }
        state.in_progress_since = 0;
        self.apply_failure(&mut state, FailureClass::Infrastructure, now);
        self.save(key, &state)?;
        Ok(Some(state))
    }

    pub(crate) fn record(
        &self,
        key: &WorkKey,
        outcome: AttemptOutcome,
        now: u64,
    ) -> Result<AttemptState, String> {
        match outcome {
            AttemptOutcome::Success => {
                let mut state = self.load(key)?;
                state.in_progress_since = 0;
                self.apply_success(&mut state, now);
                self.save(key, &state)?;
                Ok(state)
            }
            AttemptOutcome::Failure => self.record_failure(key, FailureClass::Validation, now),
        }
    }

    pub(crate) fn record_failure(
        &self,
        key: &WorkKey,
        class: FailureClass,
        now: u64,
    ) -> Result<AttemptState, String> {
        let mut state = self.load(key)?;
        state.in_progress_since = 0;
        self.apply_failure(&mut state, class, now);
        self.save(key, &state)?;
        Ok(state)
    }

    fn apply_success(&self, state: &mut AttemptState, now: u64) {
        state.total_attempts = state.total_attempts.saturating_add(1);
        state.last_attempt_at = now;
        state.last_outcome = Some(AttemptOutcome::Success);
        state.last_failure_class = None;
        state.consecutive_failures = 0;
        state.quarantine_until = 0;
        state.next_eligible_at = now.saturating_add(self.policy.success_cooldown_secs);
    }

    fn apply_failure(&self, state: &mut AttemptState, class: FailureClass, now: u64) {
        state.total_attempts = state.total_attempts.saturating_add(1);
        state.last_attempt_at = now;
        state.last_outcome = Some(AttemptOutcome::Failure);
        state.last_failure_class = Some(class);

        if class.transient() {
            state.next_eligible_at = now.saturating_add(self.policy.transient_failure_cooldown_secs);
            return;
        }

        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        let cooldown = self.policy.failure_cooldown(state.consecutive_failures);
        state.next_eligible_at = now.saturating_add(cooldown);
        if state.consecutive_failures >= self.policy.quarantine_after_failures {
            state.quarantine_until = now.saturating_add(self.policy.quarantine_secs);
        }
    }
'''
state = replace_once(state, old_lifecycle, new_lifecycle, "classified attempt lifecycle")

state = replace_once(
    state,
    '''        "{STATE_VERSION}\\ntotal_attempts={}\\nconsecutive_failures={}\\nlast_attempt_at={}\\nnext_eligible_at={}\\nquarantine_until={}\\nin_progress_since={}\\nlast_outcome={}\\n",
        state.total_attempts,
        state.consecutive_failures,
        state.last_attempt_at,
        state.next_eligible_at,
        state.quarantine_until,
        state.in_progress_since,
        state.last_outcome.map_or("none", AttemptOutcome::as_str)
''',
    '''        "{STATE_VERSION}\\ntotal_attempts={}\\nconsecutive_failures={}\\nlast_attempt_at={}\\nnext_eligible_at={}\\nquarantine_until={}\\nin_progress_since={}\\nlast_outcome={}\\nlast_failure_class={}\\n",
        state.total_attempts,
        state.consecutive_failures,
        state.last_attempt_at,
        state.next_eligible_at,
        state.quarantine_until,
        state.in_progress_since,
        state.last_outcome.map_or("none", AttemptOutcome::as_str),
        state.last_failure_class.map_or("none", FailureClass::as_str)
''',
    "classified state serialization",
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
    if version != STATE_VERSION && version != LEGACY_STATE_V1 && version != LEGACY_STATE_V2 {
        return Err(format!("unsupported state version: {version}"));
    }
    let legacy_v1 = version == LEGACY_STATE_V1;
    let legacy_without_failure_class = version != STATE_VERSION;
''',
    "classified state parser version",
)
state = replace_once(state, '                if legacy {\n', '                if legacy_v1 {\n', "legacy v1 lease parser")
state = replace_once(
    state,
    '''            "last_outcome" => state.last_outcome = AttemptOutcome::parse(value)?,
            other => return Err(format!("unknown state field: {other}")),
''',
    '''            "last_outcome" => state.last_outcome = AttemptOutcome::parse(value)?,
            "last_failure_class" => {
                if legacy_without_failure_class {
                    return Err(format!("{version} state cannot contain last_failure_class"));
                }
                state.last_failure_class = FailureClass::parse(value)?;
            }
            other => return Err(format!("unknown state field: {other}")),
''',
    "failure class parser",
)

state = replace_once(
    state,
    '''            failure_max_cooldown_secs: 40,
            quarantine_after_failures: 4,
''',
    '''            failure_max_cooldown_secs: 40,
            transient_failure_cooldown_secs: 5,
            quarantine_after_failures: 4,
''',
    "test retry policy",
)
state = replace_once(
    state,
    '''        assert_eq!(recovered.consecutive_failures, 1);
        assert_eq!(recovered.last_outcome, Some(AttemptOutcome::Failure));
        assert_eq!(recovered.next_eligible_at, 130);
''',
    '''        assert_eq!(recovered.consecutive_failures, 0);
        assert_eq!(recovered.last_outcome, Some(AttemptOutcome::Failure));
        assert_eq!(recovered.last_failure_class, Some(FailureClass::Infrastructure));
        assert_eq!(recovered.next_eligible_at, 125);
''',
    "interrupted attempt semantics",
)
state = replace_once(state, 'assert!(rewritten.starts_with("v2\\n"));', 'assert!(rewritten.starts_with("v3\\n"));', "v1 migration target")
state = replace_once(
    state,
    '''    #[test]
    fn corrupt_or_future_state_fails_closed() {
''',
    '''    #[test]
    fn transient_failures_rotate_without_semantic_quarantine() {
        let root = temporary_root("transient");
        let store = AttemptStore::new(root.clone(), test_policy());
        let key = WorkKey::new("Memorithm/ADA", "ISSUE", 7);

        let first = store
            .record_failure(&key, FailureClass::Infrastructure, 100)
            .unwrap();
        assert_eq!(first.total_attempts, 1);
        assert_eq!(first.consecutive_failures, 0);
        assert_eq!(first.next_eligible_at, 105);
        assert_eq!(first.last_failure_class, Some(FailureClass::Infrastructure));

        let second = store
            .record_failure(&key, FailureClass::Publication, 200)
            .unwrap();
        assert_eq!(second.total_attempts, 2);
        assert_eq!(second.consecutive_failures, 0);
        assert_eq!(second.quarantine_until, 0);
        assert_eq!(second.next_eligible_at, 205);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn v2_state_migrates_with_unknown_legacy_failure_class() {
        let root = temporary_root("v2");
        let key = WorkKey::new("Memorithm/ADA", "ISSUE", 7);
        let path = key.state_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "v2\\ntotal_attempts=2\\nconsecutive_failures=1\\nlast_attempt_at=100\\nnext_eligible_at=110\\nquarantine_until=0\\nin_progress_since=0\\nlast_outcome=failure\\n",
        )
        .unwrap();
        let store = AttemptStore::new(root.clone(), test_policy());
        let loaded = store.load(&key).unwrap();
        assert_eq!(loaded.last_failure_class, None);
        store.record(&key, AttemptOutcome::Success, 200).unwrap();
        let rewritten = fs::read_to_string(path).unwrap();
        assert!(rewritten.starts_with("v3\\n"));
        assert!(rewritten.contains("last_failure_class=none\\n"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_or_future_state_fails_closed() {
''',
    "classified retry tests",
)

# ---- Typed execution failures in the runtime. ----
run_config_tail = '''    retry_policy: state::RetryPolicy,
}

struct InstanceLock {
'''
action_failure = '''    retry_policy: state::RetryPolicy,
}

#[derive(Debug)]
struct ActionFailure {
    class: state::FailureClass,
    message: String,
}

impl ActionFailure {
    fn new(class: state::FailureClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ActionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "[{}] {}", self.class.as_str(), self.message)
    }
}

trait ClassifiedResult<T> {
    fn classified(self, class: state::FailureClass) -> Result<T, ActionFailure>;
}

impl<T> ClassifiedResult<T> for Result<T, String> {
    fn classified(self, class: state::FailureClass) -> Result<T, ActionFailure> {
        self.map_err(|message| ActionFailure::new(class, message))
    }
}

struct InstanceLock {
'''
main = replace_once(main, run_config_tail, action_failure, "action failure type")

main = replace_once(
    main,
    '''                failure_max_cooldown_secs: env_u64("ORCHESTRATOR_FAILURE_MAX_COOLDOWN_SECS", 7_200),
                quarantine_after_failures: env::var("ORCHESTRATOR_QUARANTINE_AFTER_FAILURES")
''',
    '''                failure_max_cooldown_secs: env_u64("ORCHESTRATOR_FAILURE_MAX_COOLDOWN_SECS", 7_200),
                transient_failure_cooldown_secs: env_u64(
                    "ORCHESTRATOR_TRANSIENT_FAILURE_COOLDOWN_SECS",
                    180,
                ),
                quarantine_after_failures: env::var("ORCHESTRATOR_QUARANTINE_AFTER_FAILURES")
''',
    "transient policy env",
)

old_run_agent = '''fn run_agent(config: &RunConfig, workspace: &Path, prompt: &str) -> Result<(), String> {
    println!();
    println!("===== OPENCODE LOCAL AGENT =====");
    println!("model: {}", config.model);
    println!("workspace: {}", workspace.display());
    println!();

    let status = Command::new("opencode")
        .current_dir(workspace)
        .env("OPENCODE_CONFIG_CONTENT", OPENCODE_INLINE_CONFIG)
        .env("OPENCODE_DISABLE_AUTOUPDATE", "1")
        .args(["run", "--auto", "--model"])
        .arg(&config.model)
        .arg(prompt)
        .status()
        .map_err(|error| format!("failed to execute opencode: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("opencode exited with {status}"))
    }
}
'''
new_run_agent = '''fn run_agent(config: &RunConfig, workspace: &Path, prompt: &str) -> Result<(), ActionFailure> {
    println!();
    println!("===== OPENCODE LOCAL AGENT =====");
    println!("model: {}", config.model);
    println!("workspace: {}", workspace.display());
    println!();

    let status = Command::new("opencode")
        .current_dir(workspace)
        .env("OPENCODE_CONFIG_CONTENT", OPENCODE_INLINE_CONFIG)
        .env("OPENCODE_DISABLE_AUTOUPDATE", "1")
        .args(["run", "--auto", "--model"])
        .arg(&config.model)
        .arg(prompt)
        .status()
        .map_err(|error| {
            ActionFailure::new(
                state::FailureClass::Infrastructure,
                format!("failed to execute opencode: {error}"),
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        let class = if status.code() == Some(70) {
            state::FailureClass::Infrastructure
        } else {
            state::FailureClass::Agent
        };
        Err(ActionFailure::new(class, format!("opencode exited with {status}")))
    }
}
'''
main = replace_once(main, old_run_agent, new_run_agent, "typed agent failure")

# Replace the three execution functions as a unit, preserving helpers between them.
issue_start = main.index("fn execute_issue(\n")
ci_start = main.index("fn execute_ci_fix(", issue_start)
pr_sha_start = main.index("fn pr_head_sha(", ci_start)
pr_attention_start = main.index("fn handle_pr_attention(", pr_sha_start)
runtime_preflight_start = main.index("fn runtime_preflight(", pr_attention_start)

new_issue = r'''fn execute_issue(
    config: &RunConfig,
    repositories: &[Repository],
    item: &WorkItem,
) -> Result<(), ActionFailure> {
    let repository = repository_by_name(repositories, &item.repository)
        .classified(state::FailureClass::Repository)?;
    let default_branch = repository
        .default_branch
        .as_deref()
        .ok_or_else(|| {
            ActionFailure::new(
                state::FailureClass::Repository,
                format!("{} has no default branch", item.repository),
            )
        })?;
    let store = issue_publication_store(config);
    let key = issue_publication_key(item);

    if let Some(pending) = store
        .load(&key)
        .classified(state::FailureClass::Infrastructure)?
    {
        println!(
            "Resuming pending publication for {}#{} from {} at {}",
            item.repository, item.number, pending.branch, pending.commit
        );
        return resume_issue_publication(config, repository, item, &store, &key, pending)
            .classified(state::FailureClass::Publication);
    }

    if let Some(number) = open_pr_number(&item.repository)
        .classified(state::FailureClass::Infrastructure)?
    {
        return Err(ActionFailure::new(
            state::FailureClass::Infrastructure,
            format!(
                "repository gained open PR #{number} after triage; deferring issue work to avoid parallel mutation"
            ),
        ));
    }

    let (workspace, branch) = prepare_issue_workspace(config, repository, item.number)
        .classified(state::FailureClass::Repository)?;
    let body = github_body(item).classified(state::FailureClass::Infrastructure)?;
    run_agent(config, &workspace, &agent_prompt(item, &body))?;

    if !has_changes(&workspace).classified(state::FailureClass::Repository)? {
        println!("Agent produced no working-tree changes; nothing will be pushed.");
        return Ok(());
    }

    reject_sensitive_paths(&workspace).classified(state::FailureClass::Validation)?;
    validate_workspace(config, &workspace).classified(state::FailureClass::Validation)?;
    let message = format!("feat: progress issue #{}", item.number);
    let commit_sha = commit_changes(&workspace, &message)
        .classified(state::FailureClass::Repository)?;
    println!("Created commit {commit_sha}");

    let mut pending = publication::PendingPublication::new(
        branch.clone(),
        commit_sha,
        default_branch.to_owned(),
        publication::PublicationPhase::Prepared,
    )
    .classified(state::FailureClass::Publication)?;
    store
        .save(&key, &pending)
        .classified(state::FailureClass::Publication)?;
    println!("Publication transaction prepared for {branch}");

    run_in_dir(
        &workspace,
        "git",
        &["push", "-u", "origin", branch.as_str()],
    )
    .classified(state::FailureClass::Publication)?;
    pending.phase = publication::PublicationPhase::Pushed;
    store
        .save(&key, &pending)
        .classified(state::FailureClass::Publication)?;
    println!(
        "Publication transaction recorded pushed commit {}",
        pending.commit
    );

    create_issue_pull_request(&workspace, item, default_branch, &branch)
        .classified(state::FailureClass::Publication)?;
    store
        .clear(&key)
        .classified(state::FailureClass::Publication)?;
    Ok(())
}

'''
new_ci = r'''fn execute_ci_fix(config: &RunConfig, item: &WorkItem) -> Result<(), ActionFailure> {
    let workspace = prepare_pr_workspace(config, &item.repository, item.number)
        .classified(state::FailureClass::Repository)?;
    let body = github_body(item).classified(state::FailureClass::Infrastructure)?;
    run_agent(config, &workspace, &agent_prompt(item, &body))?;

    if !has_changes(&workspace).classified(state::FailureClass::Repository)? {
        println!("Agent produced no working-tree changes; CI may already have moved on.");
        return Ok(());
    }

    reject_sensitive_paths(&workspace).classified(state::FailureClass::Validation)?;
    validate_workspace(config, &workspace).classified(state::FailureClass::Validation)?;
    let message = format!("fix: repair CI for PR #{}", item.number);
    let commit_sha = commit_changes(&workspace, &message)
        .classified(state::FailureClass::Repository)?;
    println!("Created commit {commit_sha}");
    run_in_dir(&workspace, "git", &["push", "origin", "HEAD"])
        .classified(state::FailureClass::Publication)
}

'''
new_attention = r'''fn handle_pr_attention(config: &RunConfig, item: &WorkItem) -> Result<(), ActionFailure> {
    let ci_state = item.ci_state.unwrap_or(CiState::Unknown);
    if ci_state == CiState::NoChecks {
        println!(
            "{}#{} has no CI checks; validating the checked-out PR locally before any merge.",
            item.repository, item.number
        );
        let workspace = prepare_pr_workspace(config, &item.repository, item.number)
            .classified(state::FailureClass::Repository)?;
        validate_workspace(config, &workspace).classified(state::FailureClass::Validation)?;
    }

    if !config.auto_merge {
        println!(
            "{}#{} is ready for attention, but ORCHESTRATOR_AUTO_MERGE is disabled.",
            item.repository, item.number
        );
        return Ok(());
    }
    if !matches!(ci_state, CiState::Passing | CiState::NoChecks) {
        return Err(ActionFailure::new(
            state::FailureClass::Validation,
            format!(
                "refusing merge for {}#{} with CI state {}",
                item.repository,
                item.number,
                ci_state.as_str()
            ),
        ));
    }

    let number = item.number.to_string();
    let head_sha = pr_head_sha(&item.repository, item.number)
        .classified(state::FailureClass::Infrastructure)?;
    if item.draft {
        println!(
            "Marking {}#{} ready for review",
            item.repository, item.number
        );
        let status = Command::new("gh")
            .args(["pr", "ready"])
            .arg(&number)
            .arg("--repo")
            .arg(&item.repository)
            .status()
            .map_err(|error| {
                ActionFailure::new(
                    state::FailureClass::Publication,
                    format!("failed to execute gh pr ready: {error}"),
                )
            })?;
        if !status.success() {
            return Err(ActionFailure::new(
                state::FailureClass::Publication,
                format!("gh pr ready failed for {}#{}", item.repository, item.number),
            ));
        }
    }

    println!(
        "Merging {}#{} after validated green state",
        item.repository, item.number
    );
    let status = Command::new("gh")
        .args(["pr", "merge"])
        .arg(&number)
        .arg("--repo")
        .arg(&item.repository)
        .args(["--squash", "--match-head-commit"])
        .arg(&head_sha)
        .status()
        .map_err(|error| {
            ActionFailure::new(
                state::FailureClass::Publication,
                format!("failed to execute gh pr merge: {error}"),
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(ActionFailure::new(
            state::FailureClass::Publication,
            format!("gh pr merge failed for {}#{}", item.repository, item.number),
        ))
    }
}

'''
main = main[:issue_start] + new_issue + new_ci + main[pr_sha_start:pr_attention_start] + new_attention + main[runtime_preflight_start:]

main = replace_once(
    main,
    '''fn execute_item(
    config: &RunConfig,
    snapshot: &TriageSnapshot,
    item: &WorkItem,
) -> Result<(), String> {
''',
    '''fn execute_item(
    config: &RunConfig,
    snapshot: &TriageSnapshot,
    item: &WorkItem,
) -> Result<(), ActionFailure> {
''',
    "typed execute item",
)

main = replace_once(
    main,
    '''    println!(
        "quarantine       : after {} failures for {}s",
        config.retry_policy.quarantine_after_failures, config.retry_policy.quarantine_secs
    );
''',
    '''    println!(
        "quarantine       : after {} failures for {}s",
        config.retry_policy.quarantine_after_failures, config.retry_policy.quarantine_secs
    );
    println!(
        "transient retry  : {}s (infrastructure/publication)",
        config.retry_policy.transient_failure_cooldown_secs
    );
''',
    "transient policy log",
)

main = replace_once(
    main,
    '''                                eprintln!("cycle {cycle} action failed: {error}");
                                if let Err(journal_error) = journal.record(
                                    trajectory::EventPhase::AttemptFinished,
                                    "failure",
                                    &error,
                                    finished_at,
                                ) {
''',
    '''                                eprintln!("cycle {cycle} action failed: {error}");
                                let outcome = format!("failure:{}", error.class.as_str());
                                if let Err(journal_error) = journal.record(
                                    trajectory::EventPhase::AttemptFinished,
                                    &outcome,
                                    &error.message,
                                    finished_at,
                                ) {
''',
    "classified trajectory failure",
)
main = replace_once(
    main,
    '''                                match attempt_store.record(
                                    &key,
                                    state::AttemptOutcome::Failure,
                                    finished_at,
                                ) {
                                    Ok(attempt_state) => eprintln!(
                                        "Scheduler recorded failure {} for {}#{}; next eligible at unix={}",
                                        attempt_state.consecutive_failures,
                                        item.repository,
                                        item.number,
                                        attempt_state.eligible_at()
                                    ),
''',
    '''                                match attempt_store.record_failure(
                                    &key,
                                    error.class,
                                    finished_at,
                                ) {
                                    Ok(attempt_state) => eprintln!(
                                        "Scheduler recorded {} failure; semantic failures={} for {}#{}; next eligible at unix={}",
                                        error.class.as_str(),
                                        attempt_state.consecutive_failures,
                                        item.repository,
                                        item.number,
                                        attempt_state.eligible_at()
                                    ),
''',
    "classified state failure",
)

# Update every RetryPolicy literal in main tests with the new field.
main = main.replace(
    '''            failure_max_cooldown_secs: 1,
            quarantine_after_failures: 4,
''',
    '''            failure_max_cooldown_secs: 1,
            transient_failure_cooldown_secs: 1,
            quarantine_after_failures: 4,
''',
)

# Service policy must be explicit rather than relying on a binary default.
service = replace_once(
    service,
    'ORCHESTRATOR_FAILURE_MAX_COOLDOWN_SECS=7200\n',
    'ORCHESTRATOR_FAILURE_MAX_COOLDOWN_SECS=7200\nORCHESTRATOR_TRANSIENT_FAILURE_COOLDOWN_SECS=180\n',
    "service transient retry policy",
)

main_path.write_text(main)
state_path.write_text(state)
service_path.write_text(service)
