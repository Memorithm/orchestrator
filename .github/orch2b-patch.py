from pathlib import Path

path = Path("src/main.rs")
text = path.read_text()

old = "mod trajectory;\nmod workspace_gc;"
new = "mod trajectory;\nmod validation_state;\nmod workspace_gc;"
assert text.count(old) == 1, text.count(old)
text = text.replace(old, new, 1)

start = text.index("struct ValidationStepResult<'a> {")
end = text.index("fn resolve_validation_cwd", start)
new_evidence = r'''struct ValidationStepResult<'a> {
    plan_attempt_id: &'a str,
    worktree_head: &'a str,
    worktree_tree: &'a str,
    started_at: u64,
    finished_at: u64,
    exit_code: Option<i32>,
    timed_out: bool,
}

fn persist_validation_step_evidence(
    config: &RunConfig,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    step: &policy::PortableValidationStep,
    result: &ValidationStepResult<'_>,
) -> Result<PathBuf, String> {
    let ValidationStepResult {
        plan_attempt_id,
        worktree_head,
        worktree_tree,
        started_at,
        finished_at,
        exit_code,
        timed_out,
    } = result;
    let directory = config
        .data_root
        .join("state/validation-evidence")
        .join(item.repository.replace('/', "__"))
        .join(format!("{}-{}", item.kind.as_str(), item.number));
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "failed to create validation evidence directory {}: {error}",
            directory.display()
        )
    })?;
    let argv_hex = step
        .argv
        .iter()
        .map(|arg| hex_bytes(arg))
        .collect::<Vec<_>>()
        .join(",");
    let record = format!(
        "validation-schema=1\nclass=portable\nrepository={}\nwork-kind={}\nwork-number={}\nplan-attempt-id={}\nstep-id={}\nargv-hex={}\ncwd={}\ntimeout-seconds={}\nstarted-at={}\nfinished-at={}\nexit-code={}\ntimed-out={}\npolicy-identity={}\nbase-sha={}\nworktree-head={}\nworktree-tree={}\nsource-ref=origin/{}\nsource-path={}\nsource-commit={}\nsource-blob={}\n",
        item.repository,
        item.kind.as_str(),
        item.number,
        plan_attempt_id,
        step.id,
        argv_hex,
        step.cwd,
        step.timeout_seconds,
        started_at,
        finished_at,
        exit_code.map_or_else(|| "none".to_owned(), |code| code.to_string()),
        timed_out,
        snapshot.identity_token(),
        snapshot.base_sha(),
        worktree_head,
        worktree_tree,
        plan.source_ref,
        plan.source_path,
        plan.source_commit,
        plan.source_blob
    );
    for sequence in 0..1_024_u16 {
        let path = directory.join(format!(
            "{}-{}-{}-{}.txt",
            started_at,
            std::process::id(),
            validation_safe_component(&step.id),
            sequence
        ));
        let mut file = match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create validation evidence {}: {error}",
                    path.display()
                ));
            }
        };
        file.write_all(record.as_bytes()).map_err(|error| {
            format!(
                "failed to write validation evidence {}: {error}",
                path.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync validation evidence {}: {error}",
                path.display()
            )
        })?;
        return Ok(path);
    }
    Err("validation evidence sequence exhausted for one step".to_owned())
}

'''
text = text[:start] + new_evidence + text[end:]

start = text.index("struct ValidationWorktreeIdentity<'a> {")
end = text.index("fn ensure_git_identity", start)
new_validation = r'''fn git_hash_validation_payload(workspace: &Path, payload: &str) -> Result<String, String> {
    let mut child = Command::new("git")
        .current_dir(workspace)
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to hash portable validation identity: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open git hash-object stdin".to_owned())?;
    stdin
        .write_all(payload.as_bytes())
        .map_err(|error| format!("failed to write portable validation identity: {error}"))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to wait for git hash-object: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git hash-object failed while binding portable validation: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let identity = String::from_utf8(output.stdout)
        .map_err(|error| format!("invalid UTF-8 from git hash-object: {error}"))?;
    let identity = identity.trim();
    if !matches!(identity.len(), 40 | 64)
        || !identity.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "invalid portable validation hash identity: {identity:?}"
        ));
    }
    Ok(identity.to_owned())
}

fn portable_validation_plan_identity(
    workspace: &Path,
    plan: &policy::PortableValidationPlan,
) -> Result<String, String> {
    let mut record = format!("portable-validation-plan-v1\nsteps={}\n", plan.steps.len());
    for (step_index, step) in plan.steps.iter().enumerate() {
        record.push_str(&format!(
            "step-index={step_index}\nstep-id-hex={}\ncwd-hex={}\ntimeout-seconds={}\nargc={}\n",
            hex_bytes(&step.id),
            hex_bytes(&step.cwd),
            step.timeout_seconds,
            step.argv.len()
        ));
        for (arg_index, arg) in step.argv.iter().enumerate() {
            record.push_str(&format!(
                "arg-index={arg_index}\narg-hex={}\n",
                hex_bytes(arg)
            ));
        }
    }
    git_hash_validation_payload(workspace, &record)
}

struct ValidationWorktreeIdentity<'a> {
    head: &'a str,
    tree: &'a str,
}

fn portable_validation_binding(
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    identity: &ValidationWorktreeIdentity<'_>,
) -> Result<validation_state::PlanBinding, String> {
    let plan_identity = portable_validation_plan_identity(workspace, plan)?;
    let policy_identity = snapshot.identity_token();
    let binding_record = format!(
        "portable-validation-binding-v1\nrepository-hex={}\nwork-kind={}\nwork-number={}\nplan-identity={}\npolicy-identity={}\nbase-sha={}\nworktree-head={}\nworktree-tree={}\nsource-ref-hex={}\nsource-path-hex={}\nsource-commit={}\nsource-blob={}\ndeclared-steps={}\n",
        hex_bytes(&item.repository),
        item.kind.as_str(),
        item.number,
        plan_identity,
        policy_identity,
        snapshot.base_sha(),
        identity.head,
        identity.tree,
        hex_bytes(&plan.source_ref),
        hex_bytes(&plan.source_path),
        plan.source_commit,
        plan.source_blob,
        plan.steps.len()
    );
    let binding_identity = git_hash_validation_payload(workspace, &binding_record)?;
    validation_state::PlanBinding::new(
        item.repository.clone(),
        item.kind.as_str().to_owned(),
        item.number,
        binding_identity,
        plan_identity,
        policy_identity,
        snapshot.base_sha().to_owned(),
        identity.head.to_owned(),
        identity.tree.to_owned(),
        plan.source_ref.clone(),
        plan.source_path.clone(),
        plan.source_commit.clone(),
        plan.source_blob.clone(),
        plan.steps.len(),
    )
}

fn portable_validation_attempt_id(
    binding: &validation_state::PlanBinding,
) -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_nanos();
    Ok(format!(
        "{}-{nanos}-{}",
        binding.binding_identity(),
        std::process::id()
    ))
}

#[derive(Debug)]
enum PortableStepOutcome {
    Passed,
    Failed(String),
    TimedOut(String),
}

fn run_portable_validation_plan(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    worktree_head: &str,
) -> Result<(), String> {
    run_portable_validation_plan_with_reuse(
        config,
        workspace,
        item,
        snapshot,
        plan,
        worktree_head,
        false,
    )
}

fn run_portable_validation_plan_with_reuse(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    worktree_head: &str,
    allow_reuse: bool,
) -> Result<(), String> {
    let worktree_tree = validation_worktree_tree(config, workspace)?;
    let identity = ValidationWorktreeIdentity {
        head: worktree_head,
        tree: &worktree_tree,
    };
    let binding = portable_validation_binding(workspace, item, snapshot, plan, &identity)?;
    let store = validation_state::ValidationPlanStore::new(
        config.data_root.join("state/validation-plans"),
    );
    if allow_reuse && store.reusable_passed(&binding)? {
        println!(
            "Reusing exact portable validation pass binding={}",
            binding.binding_identity()
        );
        return Ok(());
    }

    let attempt_id = portable_validation_attempt_id(&binding)?;
    let mut attempt = store.begin(binding, attempt_id, unix_timestamp())?;
    for (index, step) in plan.steps.iter().enumerate() {
        let completed_steps = index + 1;
        let outcome = run_portable_validation_step(
            config,
            workspace,
            item,
            snapshot,
            plan,
            step,
            &identity,
            attempt.attempt_id(),
        )?;
        store.update_progress(&mut attempt, completed_steps)?;
        match outcome {
            PortableStepOutcome::Passed => {}
            PortableStepOutcome::Failed(message) => {
                let history = store.finish(
                    &mut attempt,
                    validation_state::TerminalStatus::Failed,
                    completed_steps,
                    unix_timestamp(),
                )?;
                println!("validation plan terminal evidence: {}", history.display());
                return Err(message);
            }
            PortableStepOutcome::TimedOut(message) => {
                let history = store.finish(
                    &mut attempt,
                    validation_state::TerminalStatus::TimedOut,
                    completed_steps,
                    unix_timestamp(),
                )?;
                println!("validation plan terminal evidence: {}", history.display());
                return Err(message);
            }
        }
    }
    let declared_steps = attempt.binding.declared_steps();
    let history = store.finish(
        &mut attempt,
        validation_state::TerminalStatus::Passed,
        declared_steps,
        unix_timestamp(),
    )?;
    println!("validation plan terminal evidence: {}", history.display());
    Ok(())
}

fn run_portable_validation_step(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    step: &policy::PortableValidationStep,
    identity: &ValidationWorktreeIdentity<'_>,
    plan_attempt_id: &str,
) -> Result<PortableStepOutcome, String> {
    let _cwd = resolve_validation_cwd(workspace, &step.cwd)?;
    let workspace_root = workspace.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize workspace {}: {error}",
            workspace.display()
        )
    })?;
    let executable = step
        .argv
        .first()
        .ok_or_else(|| format!("validation step {} has empty argv", step.id))?;
    let sandbox = validation_sandbox_path()?;
    let mut command = Command::new(&sandbox);
    command
        .current_dir(&workspace_root)
        .arg("--cwd")
        .arg(&step.cwd)
        .arg("--")
        .arg(executable)
        .args(step.argv.iter().skip(1))
        .env("ORCHESTRATOR_DATA_ROOT", &config.data_root);
    println!("$ [portable:{}] {}", step.id, step.argv.join(" "));
    let started_at = unix_timestamp();
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn portable validation {}: {error}", step.id))?;
    let timeout = Duration::from_secs(step.timeout_seconds);
    let (status, timed_out) = loop {
        match child
            .try_wait()
            .map_err(|error| format!("failed to poll validation {}: {error}", step.id))?
        {
            Some(status) => break (status, false),
            None if started.elapsed() >= timeout => {
                child.kill().map_err(|error| {
                    format!("failed to kill timed-out validation {}: {error}", step.id)
                })?;
                let status = child.wait().map_err(|error| {
                    format!("failed to reap timed-out validation {}: {error}", step.id)
                })?;
                break (status, true);
            }
            None => thread::sleep(Duration::from_millis(50)),
        }
    };
    let finished_at = unix_timestamp();
    let evidence = persist_validation_step_evidence(
        config,
        item,
        snapshot,
        plan,
        step,
        &ValidationStepResult {
            plan_attempt_id,
            worktree_head: identity.head,
            worktree_tree: identity.tree,
            started_at,
            finished_at,
            exit_code: status.code(),
            timed_out,
        },
    )?;
    println!("validation evidence: {}", evidence.display());
    if timed_out {
        return Ok(PortableStepOutcome::TimedOut(format!(
            "portable validation step {} timed out after {} seconds",
            step.id, step.timeout_seconds
        )));
    }
    if !status.success() {
        return Ok(PortableStepOutcome::Failed(format!(
            "portable validation step {} failed with {status}",
            step.id
        )));
    }
    Ok(PortableStepOutcome::Passed)
}

fn validate_workspace_internal(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    allow_portable_reuse: bool,
) -> Result<(), String> {
    println!();
    println!("===== ORCHESTRATOR VALIDATION =====");
    run_in_dir(workspace, "git", &["diff", "--check"])?;
    if let Some(plan) = snapshot.portable_validation_plan()? {
        let worktree_head = capture_in_dir(workspace, "git", &["rev-parse", "HEAD"])?;
        run_portable_validation_plan_with_reuse(
            config,
            workspace,
            item,
            snapshot,
            &plan,
            &worktree_head,
            allow_portable_reuse,
        )?;
        return Ok(());
    }
    if workspace.join("Cargo.toml").is_file() {
        run_in_dir(workspace, "cargo", &["fmt", "--all", "--", "--check"])?;
        run_in_dir(workspace, "cargo", &["check", "--workspace"])?;
        run_in_dir(
            workspace,
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        )?;
        if config.full_validation {
            run_in_dir(workspace, "cargo", &["test", "--workspace"])?;
        }
    }
    Ok(())
}

fn validate_workspace(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
) -> Result<(), String> {
    validate_workspace_internal(config, workspace, item, snapshot, false)
}

fn validate_workspace_reusing_passed(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
) -> Result<(), String> {
    validate_workspace_internal(config, workspace, item, snapshot, true)
}

'''
text = text[:start] + new_validation + text[end:]

old = '''    reject_sensitive_committed_paths(workspace, &base_ref)?;
    validate_workspace(config, workspace, item, snapshot)
}'''
new = '''    reject_sensitive_committed_paths(workspace, &base_ref)?;
    validate_workspace_reusing_passed(config, workspace, item, snapshot)
}'''
assert text.count(old) == 1, text.count(old)
text = text.replace(old, new, 1)

anchor = '''    #[test]
    fn portable_validation_stops_after_first_failure() {'''
assert text.count(anchor) == 1, text.count(anchor)
new_test = r'''    #[test]
    fn exact_portable_validation_pass_is_reused_without_new_step_evidence() {
        if !root_validation_sandbox_test_enabled() {
            return;
        }
        let root = orch2_validation_test_root("reuse-pass");
        let data_root = root.join("data");
        let workspace = orch2_validation_test_workspace(&data_root);
        let config = orch2_validation_test_config(&data_root);
        let item = orch2_validation_test_item();
        let snapshot = policy::test_snapshot_for_validation(
            "Memorithm/Test",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        );
        let plan = orch2_validation_test_plan(vec![orch2_step("pass", &["true"], 5)]);
        let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        run_portable_validation_plan_with_reuse(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            head,
            false,
        )
        .unwrap();
        let evidence_dir = data_root
            .join("state/validation-evidence")
            .join("Memorithm__Test")
            .join("ISSUE-45");
        let before = fs::read_dir(&evidence_dir).unwrap().count();
        assert_eq!(before, 1);
        run_portable_validation_plan_with_reuse(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            head,
            true,
        )
        .unwrap();
        let after = fs::read_dir(&evidence_dir).unwrap().count();
        assert_eq!(after, before);
        let _ = fs::remove_dir_all(root);
    }

'''
text = text.replace(anchor, new_test + anchor, 1)

path.write_text(text)
