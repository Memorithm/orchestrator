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


replace_once(
    "use std::time::{Duration, SystemTime, UNIX_EPOCH};\n\n",
    "use std::time::{Duration, SystemTime, UNIX_EPOCH};\n\nmod state;\n\n",
    "module declaration",
)

replace_once(
    'const DEFAULT_MODEL: &str = "ollama/muse-glimmer:latest";',
    'const DEFAULT_MODEL: &str = "ollama/qwen3.8:latest";',
    "Qwen-only default",
)

replace_once(
    """#[derive(Debug, Clone)]
struct RunConfig {
    organization: String,
    model: String,
    interval: Duration,
    data_root: PathBuf,
    auto_merge: bool,
    full_validation: bool,
    max_cycles: u64,
}
""",
    """#[derive(Debug, Clone)]
struct RunConfig {
    organization: String,
    model: String,
    interval: Duration,
    data_root: PathBuf,
    auto_merge: bool,
    full_validation: bool,
    max_cycles: u64,
    retry_policy: state::RetryPolicy,
}
""",
    "RunConfig retry policy",
)

replace_once(
    """            auto_merge: env_flag("ORCHESTRATOR_AUTO_MERGE", false),
            full_validation: env_flag("ORCHESTRATOR_FULL_VALIDATION", false),
            max_cycles: max_cycles_override
                .unwrap_or_else(|| env_u64("ORCHESTRATOR_MAX_CYCLES", 0)),
        })
""",
    """            auto_merge: env_flag("ORCHESTRATOR_AUTO_MERGE", false),
            full_validation: env_flag("ORCHESTRATOR_FULL_VALIDATION", false),
            max_cycles: max_cycles_override
                .unwrap_or_else(|| env_u64("ORCHESTRATOR_MAX_CYCLES", 0)),
            retry_policy: state::RetryPolicy {
                success_cooldown_secs: env_u64("ORCHESTRATOR_SUCCESS_COOLDOWN_SECS", 900),
                failure_base_cooldown_secs: env_u64(
                    "ORCHESTRATOR_FAILURE_BASE_COOLDOWN_SECS",
                    300,
                ),
                failure_max_cooldown_secs: env_u64(
                    "ORCHESTRATOR_FAILURE_MAX_COOLDOWN_SECS",
                    7_200,
                ),
                quarantine_after_failures: env::var("ORCHESTRATOR_QUARANTINE_AFTER_FAILURES")
                    .ok()
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(4),
                quarantine_secs: env_u64("ORCHESTRATOR_QUARANTINE_SECS", 21_600),
            },
        })
""",
    "RunConfig env policy",
)

old_execute = """fn execute_selected(config: &RunConfig, snapshot: &TriageSnapshot) -> Result<(), String> {
    let Some(item) = snapshot.selected_for_run(config.auto_merge) else {
        println!("No actionable work this cycle.");
        return Ok(());
    };

    println!();
    println!("===== SELECTED WORK =====");
    println!("kind       : {}", item.kind.as_str());
    println!("repository : {}", item.repository);
    println!("reference  : #{}", item.number);
    println!("title      : {}", item.title);

    match item.kind {
        WorkKind::FixCi => execute_ci_fix(config, item),
        WorkKind::PullRequest => handle_pr_attention(config, item),
        WorkKind::Issue => execute_issue(config, &snapshot.repositories, item),
        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => Ok(()),
    }
}
"""

new_execute = """fn work_key(item: &WorkItem) -> state::WorkKey {
    state::WorkKey::new(&item.repository, item.kind.as_str(), item.number)
}

fn work_item_runnable(item: &WorkItem, auto_merge: bool) -> bool {
    match item.kind {
        WorkKind::FixCi | WorkKind::Issue => true,
        WorkKind::PullRequest => auto_merge,
        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => false,
    }
}

fn selected_for_run_with_state<'a>(
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

fn execute_item(
    config: &RunConfig,
    snapshot: &TriageSnapshot,
    item: &WorkItem,
) -> Result<(), String> {
    println!();
    println!("===== SELECTED WORK =====");
    println!("kind       : {}", item.kind.as_str());
    println!("repository : {}", item.repository);
    println!("reference  : #{}", item.number);
    println!("title      : {}", item.title);

    match item.kind {
        WorkKind::FixCi => execute_ci_fix(config, item),
        WorkKind::PullRequest => handle_pr_attention(config, item),
        WorkKind::Issue => execute_issue(config, &snapshot.repositories, item),
        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => Ok(()),
    }
}
"""
replace_once(old_execute, new_execute, "state-aware execution")

replace_once(
    '    println!("paid LLM APIs    : DISABLED");\n\n    let mut cycle = 0_u64;',
    '''    println!("paid LLM APIs    : DISABLED");
    println!(
        "success cooldown : {}s",
        config.retry_policy.success_cooldown_secs
    );
    println!(
        "failure cooldown : {}s..={}s",
        config.retry_policy.failure_base_cooldown_secs,
        config.retry_policy.failure_max_cooldown_secs
    );
    println!(
        "quarantine       : after {} failures for {}s",
        config.retry_policy.quarantine_after_failures,
        config.retry_policy.quarantine_secs
    );

    let attempt_store = state::AttemptStore::new(
        config.data_root.join("state/work-items"),
        config.retry_policy,
    );
    let mut cycle = 0_u64;''',
    "attempt store initialization",
)

replace_once(
    """            Ok(snapshot) => {
                print_triage(&snapshot, &config.organization);
                if let Err(error) = execute_selected(&config, &snapshot) {
                    eprintln!("cycle {cycle} action failed: {error}");
                }
            }
""",
    """            Ok(snapshot) => {
                print_triage(&snapshot, &config.organization);
                let selection_time = unix_timestamp();
                match selected_for_run_with_state(
                    &snapshot,
                    config.auto_merge,
                    &attempt_store,
                    selection_time,
                ) {
                    Ok(Some(item)) => {
                        let key = work_key(item);
                        match execute_item(&config, &snapshot, item) {
                            Ok(()) => {
                                match attempt_store.record(
                                    &key,
                                    state::AttemptOutcome::Success,
                                    unix_timestamp(),
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
                            Err(error) => {
                                eprintln!("cycle {cycle} action failed: {error}");
                                match attempt_store.record(
                                    &key,
                                    state::AttemptOutcome::Failure,
                                    unix_timestamp(),
                                ) {
                                    Ok(attempt_state) => eprintln!(
                                        "Scheduler recorded failure {} for {}#{}; next eligible at unix={}",
                                        attempt_state.consecutive_failures,
                                        item.repository,
                                        item.number,
                                        attempt_state.eligible_at()
                                    ),
                                    Err(state_error) => {
                                        eprintln!(
                                            "scheduler state write failed after action failure: {state_error}"
                                        );
                                        return ExitCode::FAILURE;
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => println!("No runtime-eligible actionable work this cycle."),
                    Err(error) => eprintln!("cycle {cycle} scheduler state failed: {error}"),
                }
            }
""",
    "run loop state integration",
)

# Add a focused integration test for scheduler rotation without modifying the
# existing tests. The state module itself owns persistence/backoff unit tests.
closing = "\n}\n"
if not source.endswith(closing):
    raise SystemExit("test module closing brace not found")
source = source[: -len(closing)] + r'''

    #[test]
    fn state_aware_selection_skips_cooling_priority_item() {
        let root = std::env::temp_dir().join(format!(
            "orchestrator-selection-test-{}-{}",
            std::process::id(),
            unix_timestamp()
        ));
        let policy = state::RetryPolicy {
            success_cooldown_secs: 30,
            failure_base_cooldown_secs: 60,
            failure_max_cooldown_secs: 60,
            quarantine_after_failures: 4,
            quarantine_secs: 600,
        };
        let store = state::AttemptStore::new(root.clone(), policy);
        let first = WorkItem {
            kind: WorkKind::Issue,
            repository: "Memorithm/AAA".to_owned(),
            number: 1,
            title: "first".to_owned(),
            detail: "open issue".to_owned(),
            ci_state: None,
            draft: false,
        };
        let second = WorkItem {
            kind: WorkKind::Issue,
            repository: "Memorithm/BBB".to_owned(),
            number: 2,
            title: "second".to_owned(),
            detail: "open issue".to_owned(),
            ci_state: None,
            draft: false,
        };
        let snapshot = TriageSnapshot {
            repositories: Vec::new(),
            items: vec![first.clone(), second.clone()],
            eligible_count: 2,
            repositories_with_open_pr: 0,
        };

        store
            .record(&work_key(&first), state::AttemptOutcome::Failure, 100)
            .unwrap();
        let selected = selected_for_run_with_state(&snapshot, false, &store, 101)
            .unwrap()
            .unwrap();
        assert_eq!(selected.repository, second.repository);
        assert_eq!(selected.number, second.number);

        let _ = std::fs::remove_dir_all(root);
    }
}
'''

path.write_text(source)
