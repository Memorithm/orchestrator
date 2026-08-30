from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if text.count(old) != 1:
        raise SystemExit(f"expected exactly one match in {path}: {old!r}")
    file.write_text(text.replace(old, new, 1))


path = Path("src/main.rs")
text = path.read_text()

replace_once(
    "src/main.rs",
    '''    if !check_ollama() {
        return Err("Ollama is not reachable".to_owned());
    }
    if !is_local_ollama_model(&config.model) || !check_model_available(&config.model) {''',
    '''    if !check_ollama() {
        return Err("Ollama is not reachable".to_owned());
    }
    if !check_bwrap_sandbox() {
        return Err("bubblewrap process sandbox is installed but unusable".to_owned());
    }
    if !is_local_ollama_model(&config.model) || !check_model_available(&config.model) {''',
)

anchor = '''fn selected_for_run_with_state<'a>(
    snapshot: &'a TriageSnapshot,
    auto_merge: bool,
    attempt_store: &state::AttemptStore,
    now: u64,
) -> Result<Option<&'a WorkItem>, String> {'''
if text.count(anchor) != 1:
    raise SystemExit("selected_for_run_with_state anchor changed")

preflight_selector = '''fn selected_for_preflight_with_state<'a>(
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
        let revision = work_revision(item)?;
        let attempt_state = attempt_store.load_for_revision(&key, &revision)?;
        if attempt_state.in_progress_since != 0 {
            println!(
                "Preflight recovery pending: {}#{} {} has interrupted lease since unix={}; no state mutated",
                item.repository,
                item.number,
                item.kind.as_str(),
                attempt_state.in_progress_since
            );
            continue;
        }
        if attempt_state.is_eligible(now) {
            eligible.push((item, attempt_state.last_attempt_at));
        } else {
            println!(
                "Preflight cooldown: {}#{} {} deferred until unix={} (failures={})",
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
text = text.replace(anchor, preflight_selector + anchor, 1)

run_loop_anchor = '''fn run_loop(config: RunConfig) -> ExitCode {
    if let Err(error) = runtime_preflight(&config) {'''
if text.count(run_loop_anchor) != 1:
    raise SystemExit("run_loop anchor changed")

preflight_fn = '''fn preflight(config: RunConfig) -> ExitCode {
    println!("Memorithm Orchestrator preflight");
    println!("==============================");
    println!("organization     : {}", config.organization);
    println!("model            : {}", config.model);
    println!("data root        : {}", config.data_root.display());
    println!("auto merge       : {}", config.auto_merge);
    println!("full validation  : {}", config.full_validation);
    println!("agent execution  : DISABLED");
    println!("repo mutation    : DISABLED");
    println!();

    if let Err(error) = runtime_preflight(&config) {
        eprintln!("runtime preflight: FAIL: {error}");
        return ExitCode::FAILURE;
    }
    println!("runtime preflight: PASS");

    let _lock = match acquire_instance_lock(&config.data_root) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("instance isolation: FAIL: {error}");
            return ExitCode::FAILURE;
        }
    };
    println!("instance isolation: PASS (exclusive preflight lock acquired)");

    let health_report = health::inspect(&config.data_root, unix_timestamp());
    println!();
    println!("{}", health_report.text);
    if health_report.degraded {
        eprintln!("persistent state: FAIL: health is degraded");
        return ExitCode::FAILURE;
    }
    println!("persistent state: PASS");

    let resource_snapshot = match resource::sample_linux(&config.data_root) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("resource sample: FAIL: {error}");
            return ExitCode::FAILURE;
        }
    };
    match config.resource_policy.evaluate(resource_snapshot) {
        resource::Admission::Admitted(snapshot) => println!(
            "resource admission: PASS memory={}MiB disk={}MiB load1={:.2} load/cpu={:.2} cpus={}",
            snapshot.available_memory_mb,
            snapshot.free_disk_mb,
            snapshot.load_one,
            snapshot.load_per_cpu(),
            snapshot.cpu_count
        ),
        resource::Admission::Deferred {
            snapshot, reason, ..
        } => {
            eprintln!(
                "resource admission: FAIL memory={}MiB disk={}MiB load1={:.2} load/cpu={:.2} cpus={} reason={reason}",
                snapshot.available_memory_mb,
                snapshot.free_disk_mb,
                snapshot.load_one,
                snapshot.load_per_cpu(),
                snapshot.cpu_count
            );
            return ExitCode::FAILURE;
        }
    }

    let snapshot = match build_triage(&config.organization) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("GitHub triage: FAIL: {error}");
            return ExitCode::FAILURE;
        }
    };
    println!();
    print_triage(&snapshot, &config.organization);

    let attempt_store = state::AttemptStore::new(
        config.data_root.join("state/work-items"),
        config.retry_policy,
    );
    let now = unix_timestamp();
    let selected = match selected_for_preflight_with_state(
        &snapshot,
        config.auto_merge,
        &attempt_store,
        now,
    ) {
        Ok(selected) => selected,
        Err(error) => {
            eprintln!("scheduler preview: FAIL: {error}");
            return ExitCode::FAILURE;
        }
    };

    println!();
    println!("Runtime scheduler preview");
    println!("-------------------------");
    if let Some(item) = selected {
        println!("Kind       : {}", item.kind.as_str());
        println!("Repository : {}", item.repository);
        println!("Reference  : #{}", item.number);
        println!("Title      : {}", item.title);
    } else {
        println!("No runtime-eligible actionable work at this instant.");
    }
    println!();
    println!("PREFLIGHT RESULT: PASS");
    println!("No agent was launched and no managed repository was mutated.");
    ExitCode::SUCCESS
}

'''
text = text.replace(run_loop_anchor, preflight_fn + run_loop_anchor, 1)
path.write_text(text)

replace_once(
    "src/main.rs",
    '''    eprintln!("  {program} triage [organization]");
    eprintln!("  {program} run [organization]");''',
    '''    eprintln!("  {program} triage [organization]");
    eprintln!("  {program} preflight [organization]");
    eprintln!("  {program} run [organization]");''',
)

replace_once(
    "src/main.rs",
    '''        Some("triage") => triage(&organization_arg(&mut args)),
        Some("run") => match RunConfig::from_env(organization_arg(&mut args), None) {''',
    '''        Some("triage") => triage(&organization_arg(&mut args)),
        Some("preflight") => match RunConfig::from_env(organization_arg(&mut args), Some(1)) {
            Ok(config) => preflight(config),
            Err(error) => {
                eprintln!("configuration error: {error}");
                ExitCode::FAILURE
            }
        },
        Some("run") => match RunConfig::from_env(organization_arg(&mut args), None) {''',
)

replace_once(
    "README.md",
    '''orchestrator triage [organization]
orchestrator run [organization]''',
    '''orchestrator triage [organization]
orchestrator preflight [organization]
orchestrator run [organization]''',
)

replace_once(
    "README.md",
    '''`run` continuously repeats triage and executes one actionable unit per cycle.''',
    '''`preflight` acquires the exclusive Orchestrator instance lock, validates the local tool/model/sandbox runtime, inspects persistent state and host resource admission, performs live read-only GitHub triage, and previews the runtime scheduler without recovering interrupted leases, launching an agent, or mutating a managed repository.

`run` continuously repeats triage and executes one actionable unit per cycle.''',
)

replace_once(
    "src/main.rs",
    '''    #[test]
    fn required_runtime_contains_local_agent_stack() {''',
    '''    #[test]
    fn preflight_selector_uses_scheduler_order_without_writes() {
        let root = env::temp_dir().join(format!(
            "orchestrator-preflight-selection-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = state::AttemptStore::new(root.join("work-items"), state::RetryPolicy::default());
        let snapshot = TriageSnapshot {
            repositories: Vec::new(),
            items: vec![
                WorkItem {
                    kind: WorkKind::Issue,
                    repository: "Memorithm/Beta".to_owned(),
                    number: 2,
                    title: "beta".to_owned(),
                    detail: "open issue".to_owned(),
                    ci_state: None,
                    draft: false,
                },
                WorkItem {
                    kind: WorkKind::Issue,
                    repository: "Memorithm/Alpha".to_owned(),
                    number: 9,
                    title: "alpha".to_owned(),
                    detail: "open issue".to_owned(),
                    ci_state: None,
                    draft: false,
                },
            ],
            eligible_count: 2,
            repositories_with_open_pr: 0,
        };

        let selected = selected_for_preflight_with_state(&snapshot, false, &store, 100)
            .unwrap()
            .unwrap();
        assert_eq!(selected.repository, "Memorithm/Alpha");
        assert!(!root.exists(), "read-only preview unexpectedly created state");
    }

    #[test]
    fn preflight_selector_does_not_recover_interrupted_attempt() {
        let root = env::temp_dir().join(format!(
            "orchestrator-preflight-interrupted-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = state::AttemptStore::new(root.join("work-items"), state::RetryPolicy::default());
        let item = WorkItem {
            kind: WorkKind::Issue,
            repository: "Memorithm/Alpha".to_owned(),
            number: 1,
            title: "alpha".to_owned(),
            detail: "open issue".to_owned(),
            ci_state: None,
            draft: false,
        };
        let key = work_key(&item);
        store.begin_for_revision(&key, "issue-v1", 100).unwrap();
        let snapshot = TriageSnapshot {
            repositories: Vec::new(),
            items: vec![item],
            eligible_count: 1,
            repositories_with_open_pr: 0,
        };

        let selected = selected_for_preflight_with_state(&snapshot, false, &store, 200).unwrap();
        assert!(selected.is_none());
        let state = store.load_for_revision(&key, "issue-v1").unwrap();
        assert_eq!(state.in_progress_since, 100);
        assert_eq!(state.consecutive_failures, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn required_runtime_contains_local_agent_stack() {''',
)
