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


replace_once("mod state;\n", "mod state;\nmod trajectory;\n", "trajectory module")

replace_once(
    '''    println!("Selection");
    println!("---------");
''',
    '''    println!("Priority head (before runtime cooldown)");
    println!("---------------------------------------");
''',
    "triage selection label",
)

replace_once(
    '''    if workspace.join("Cargo.toml").is_file() {
        run_in_dir(workspace, "cargo", &["fmt", "--all", "--", "--check"])?;
        run_in_dir(workspace, "cargo", &["check", "--workspace"])?;
        if config.full_validation {
            run_in_dir(workspace, "cargo", &["test", "--workspace"])?;
        }
    }
''',
    '''    if workspace.join("Cargo.toml").is_file() {
        run_in_dir(workspace, "cargo", &["fmt", "--all", "--", "--check"])?;
        run_in_dir(workspace, "cargo", &["check", "--workspace"])?;
        run_in_dir(
            workspace,
            "cargo",
            &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
        )?;
        if config.full_validation {
            run_in_dir(workspace, "cargo", &["test", "--workspace"])?;
        }
    }
''',
    "generic Rust clippy validation",
)

replace_once(
    '''    let attempt_store = state::AttemptStore::new(
        config.data_root.join("state/work-items"),
        config.retry_policy,
    );
''',
    '''    let attempt_store = state::AttemptStore::new(
        config.data_root.join("state/work-items"),
        config.retry_policy,
    );
    let trajectory_root = config.data_root.join("trajectories");
    println!("trajectories      : {}", trajectory_root.display());
''',
    "trajectory root",
)

start_marker = '''                    Ok(Some(item)) => {
'''
end_marker = '''                    Ok(None) => println!("No runtime-eligible actionable work this cycle."),
'''
start = source.find(start_marker)
end = source.find(end_marker, start + 1)
if start < 0 or end < 0:
    raise SystemExit("runtime selected-item block boundaries not found")

replacement = '''                    Ok(Some(item)) => {
                        let key = work_key(item);
                        let mut journal = match trajectory::AttemptJournal::create(
                            &trajectory_root,
                            &item.repository,
                            item.kind.as_str(),
                            item.number,
                            &config.model,
                            selection_time,
                        ) {
                            Ok(journal) => {
                                println!("trajectory : {}", journal.path().display());
                                journal
                            }
                            Err(journal_error) => {
                                eprintln!("trajectory creation failed: {journal_error}");
                                return ExitCode::FAILURE;
                            }
                        };

                        match execute_item(&config, &snapshot, item) {
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
                                match attempt_store.record(
                                    &key,
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
                            Err(error) => {
                                let finished_at = unix_timestamp();
                                eprintln!("cycle {cycle} action failed: {error}");
                                if let Err(journal_error) = journal.record(
                                    trajectory::EventPhase::AttemptFinished,
                                    "failure",
                                    &error,
                                    finished_at,
                                ) {
                                    eprintln!("trajectory finalization failed: {journal_error}");
                                    return ExitCode::FAILURE;
                                }
                                match attempt_store.record(
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
'''
source = source[:start] + replacement + source[end:]

path.write_text(source)
