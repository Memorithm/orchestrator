from pathlib import Path


def replace_once(data: str, old: str, new: str, label: str) -> str:
    count = data.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return data.replace(old, new, 1)


main_path = Path("src/main.rs")
main = main_path.read_text()

old_runtime = '''    let cwd = resolve_validation_cwd(workspace, &step.cwd)?;
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
    let mut command = Command::new("bwrap");
    command
        .args([
            "--die-with-parent",
            "--unshare-user",
            "--uid",
            "0",
            "--gid",
            "0",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
            "--unshare-net",
            "--cap-drop",
            "ALL",
            "--ro-bind",
            "/",
            "/",
            "--bind",
        ])
        .arg(&workspace_root)
        .arg(&workspace_root)
        .args(["--proc", "/proc", "--dev", "/dev", "--chdir"])
        .arg(&cwd)
        .arg("--")
        .arg(executable)
        .args(step.argv.iter().skip(1))
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GITHUB_PAT")
        .env_remove("OPENAI_API_KEY")
        .env_remove("OLLAMA_HOST")
        .env_remove("OPENCODE_CONFIG_CONTENT")
        .env_remove("SSH_AUTH_SOCK");
'''
new_runtime = '''    let _cwd = resolve_validation_cwd(workspace, &step.cwd)?;
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
'''
main = replace_once(main, old_runtime, new_runtime, "portable runtime sandbox invocation")

runtime_marker = '''fn run_portable_validation_step(
    config: &RunConfig,
'''
runtime_helper = '''fn validation_sandbox_path() -> Result<PathBuf, String> {
    let path = env::var_os("ORCHESTRATOR_VALIDATION_SANDBOX")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("scripts/validation-sandbox"));
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .map_err(|error| format!("failed to resolve validation sandbox root: {error}"))?
            .join(path)
    };
    let canonical = path.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize validation sandbox {}: {error}",
            path.display()
        )
    })?;
    if !canonical.is_file() {
        return Err(format!(
            "validation sandbox is not a regular file: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn run_portable_validation_plan(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    worktree_head: &str,
) -> Result<(), String> {
    for step in &plan.steps {
        run_portable_validation_step(
            config,
            workspace,
            item,
            snapshot,
            plan,
            step,
            worktree_head,
        )?;
    }
    Ok(())
}

'''
main = replace_once(main, runtime_marker, runtime_helper + runtime_marker, "runtime helpers")

old_loop = '''    if let Some(plan) = snapshot.portable_validation_plan()? {
        let worktree_head = capture_in_dir(workspace, "git", &["rev-parse", "HEAD"])?;
        for step in &plan.steps {
            run_portable_validation_step(
                config,
                workspace,
                item,
                snapshot,
                &plan,
                step,
                &worktree_head,
            )?;
        }
        return Ok(());
    }
'''
new_loop = '''    if let Some(plan) = snapshot.portable_validation_plan()? {
        let worktree_head = capture_in_dir(workspace, "git", &["rev-parse", "HEAD"])?;
        run_portable_validation_plan(
            config,
            workspace,
            item,
            snapshot,
            &plan,
            &worktree_head,
        )?;
        return Ok(());
    }
'''
main = replace_once(main, old_loop, new_loop, "portable validation loop")

test_marker = '''    #[test]
    fn validation_cwd_rejects_symlink_escape() {
'''
tests = r'''    fn orch2_validation_test_root(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "orchestrator-orch2-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn orch2_validation_test_config(data_root: &Path) -> RunConfig {
        RunConfig {
            organization: "Memorithm".to_owned(),
            model: DEFAULT_MODEL.to_owned(),
            interval: Duration::from_secs(1),
            data_root: data_root.to_path_buf(),
            auto_merge: false,
            auto_merge_scope: merge_policy::AutoMergeScope::OrchestratorValidated,
            full_validation: false,
            max_cycles: 1,
            resource_policy: resource::ResourcePolicy {
                min_available_memory_mb: 0,
                min_free_disk_mb: 0,
                max_load_per_cpu: 0.0,
            },
            low_disk_reclaim_max_targets: 1,
            low_disk_reclaim_max_workspaces: 1,
            workspace_min_idle_secs: 1,
            trajectory_max_per_item: 1,
            retry_policy: state::RetryPolicy::default(),
        }
    }

    fn orch2_validation_test_workspace(data_root: &Path) -> PathBuf {
        let workspace = data_root.join("workspaces/Memorithm__Test");
        fs::create_dir_all(&workspace).unwrap();
        run_in_dir(&workspace, "git", &["init", "-q", "-b", "main"]).unwrap();
        workspace
    }

    fn orch2_validation_test_item() -> WorkItem {
        WorkItem {
            kind: WorkKind::Issue,
            repository: "Memorithm/Test".to_owned(),
            number: 45,
            title: "test".to_owned(),
            detail: "test".to_owned(),
            source_revision: Some("issue-v1".to_owned()),
            ci_state: None,
            draft: false,
        }
    }

    fn orch2_validation_test_plan(
        steps: Vec<policy::PortableValidationStep>,
    ) -> policy::PortableValidationPlan {
        policy::PortableValidationPlan {
            steps,
            source_ref: "agent/policy".to_owned(),
            source_path: ".agent/POLICY.yaml".to_owned(),
            source_commit: "1111111111111111111111111111111111111111".to_owned(),
            source_blob: "2222222222222222222222222222222222222222".to_owned(),
        }
    }

    fn orch2_step(id: &str, argv: &[&str], timeout_seconds: u64) -> policy::PortableValidationStep {
        policy::PortableValidationStep {
            id: id.to_owned(),
            argv: argv.iter().map(|value| (*value).to_owned()).collect(),
            cwd: ".".to_owned(),
            timeout_seconds,
        }
    }

    fn root_validation_sandbox_test_enabled() -> bool {
        capture("id", &["-u"]).is_ok_and(|uid| uid == "0") && command_available("bwrap")
    }

    #[test]
    fn portable_validation_executes_in_order_and_passes_argv_literally() {
        if !root_validation_sandbox_test_enabled() {
            return;
        }
        let root = orch2_validation_test_root("order-literal");
        let data_root = root.join("data");
        let workspace = orch2_validation_test_workspace(&data_root);
        let config = orch2_validation_test_config(&data_root);
        let item = orch2_validation_test_item();
        let snapshot = policy::test_snapshot_for_validation(
            "Memorithm/Test",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        );
        let plan = orch2_validation_test_plan(vec![
            orch2_step("mkdir", &["mkdir", "ordered"], 5),
            orch2_step("dependent", &["touch", "ordered/second"], 5),
            orch2_step("literal", &["touch", "literal;touch injected"], 5),
        ]);
        run_portable_validation_plan(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        assert!(workspace.join("ordered/second").is_file());
        assert!(workspace.join("literal;touch injected").is_file());
        assert!(!workspace.join("literal").exists());
        assert!(!workspace.join("injected").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_validation_stops_after_first_failure() {
        if !root_validation_sandbox_test_enabled() {
            return;
        }
        let root = orch2_validation_test_root("stop-failure");
        let data_root = root.join("data");
        let workspace = orch2_validation_test_workspace(&data_root);
        let config = orch2_validation_test_config(&data_root);
        let item = orch2_validation_test_item();
        let snapshot = policy::test_snapshot_for_validation(
            "Memorithm/Test",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        );
        let plan = orch2_validation_test_plan(vec![
            orch2_step("first", &["touch", "first"], 5),
            orch2_step("fail", &["false"], 5),
            orch2_step("third", &["touch", "third"], 5),
        ]);
        let error = run_portable_validation_plan(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap_err();
        assert!(error.contains("fail"));
        assert!(workspace.join("first").is_file());
        assert!(!workspace.join("third").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_validation_timeout_is_recorded_and_fails() {
        if !root_validation_sandbox_test_enabled() {
            return;
        }
        let root = orch2_validation_test_root("timeout");
        let data_root = root.join("data");
        let workspace = orch2_validation_test_workspace(&data_root);
        let config = orch2_validation_test_config(&data_root);
        let item = orch2_validation_test_item();
        let snapshot = policy::test_snapshot_for_validation(
            "Memorithm/Test",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        );
        let plan = orch2_validation_test_plan(vec![orch2_step("timeout", &["sleep", "2"], 1)]);
        let error = run_portable_validation_plan(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap_err();
        assert!(error.contains("timed out"));
        let evidence_dir = data_root
            .join("state/validation-evidence")
            .join("Memorithm__Test")
            .join("ISSUE-45");
        let records = fs::read_dir(evidence_dir)
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>();
        assert!(records.iter().any(|record| {
            record.contains("step-id=timeout\n") && record.contains("timed-out=true\n")
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_validation_cannot_mutate_git_metadata() {
        if !root_validation_sandbox_test_enabled() {
            return;
        }
        let root = orch2_validation_test_root("readonly-git");
        let data_root = root.join("data");
        let workspace = orch2_validation_test_workspace(&data_root);
        let config = orch2_validation_test_config(&data_root);
        let item = orch2_validation_test_item();
        let snapshot = policy::test_snapshot_for_validation(
            "Memorithm/Test",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        );
        let plan = orch2_validation_test_plan(vec![orch2_step(
            "git-metadata",
            &["touch", ".git/validation-must-not-write"],
            5,
        )]);
        assert!(run_portable_validation_plan(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .is_err());
        assert!(!workspace.join(".git/validation-must-not-write").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_validation_masks_host_home() {
        if !root_validation_sandbox_test_enabled() {
            return;
        }
        let root = orch2_validation_test_root("masked-home");
        let data_root = root.join("data");
        let workspace = orch2_validation_test_workspace(&data_root);
        let config = orch2_validation_test_config(&data_root);
        let item = orch2_validation_test_item();
        let snapshot = policy::test_snapshot_for_validation(
            "Memorithm/Test",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        );
        let plan = orch2_validation_test_plan(vec![orch2_step(
            "masked-home",
            &["test", "!", "-e", "/root/.ssh"],
            5,
        )]);
        run_portable_validation_plan(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validation_without_structured_plan_keeps_diff_check_fallback() {
        let root = orch2_validation_test_root("fallback");
        let data_root = root.join("data");
        let workspace = orch2_validation_test_workspace(&data_root);
        fs::write(workspace.join("tracked.txt"), "clean\n").unwrap();
        commit_changes(&workspace, "test: baseline").unwrap();
        fs::write(workspace.join("tracked.txt"), "trailing-space \n").unwrap();
        let config = orch2_validation_test_config(&data_root);
        let item = orch2_validation_test_item();
        let snapshot = policy::test_snapshot_for_validation(
            "Memorithm/Test",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        );
        assert!(validate_workspace(&config, &workspace, &item, &snapshot).is_err());
        let _ = fs::remove_dir_all(root);
    }

'''
main = replace_once(main, test_marker, tests + test_marker, "execution tests")
main_path.write_text(main)

policy_path = Path("src/policy.rs")
policy = policy_path.read_text()
policy_marker = '''    #[test]
    fn validation_plan_rejects_shell_unsafe_or_ambiguous_structure() {
'''
policy_test = r'''    #[test]
    fn validation_plan_preserves_non_shell_argument_bytes() {
        let snapshot = snapshot_with_policy_documents(&[r#"validation_plan:
  schema_version: 1
  class: portable
  steps:
    - id: literal
      argv: [touch, literal;touch injected]
"#]);
        let plan = snapshot
            .portable_validation_plan()
            .unwrap()
            .expect("portable plan");
        assert_eq!(plan.steps[0].argv, ["touch", "literal;touch injected"]);
    }

'''
policy = replace_once(policy, policy_marker, policy_test + policy_marker, "literal argv parser test")
policy_path.write_text(policy)

start_path = Path("scripts/start.sh")
start = start_path.read_text()
start = replace_once(
    start,
    'install -m 700 "$ROOT/scripts/agent-sandbox" "$WRAPPER_DIR/agent-sandbox"\n',
    'install -m 700 "$ROOT/scripts/agent-sandbox" "$WRAPPER_DIR/agent-sandbox"\ninstall -m 700 "$ROOT/scripts/validation-sandbox" "$WRAPPER_DIR/validation-sandbox"\n',
    "install validation sandbox",
)
start = replace_once(
    start,
    'export ORCHESTRATOR_AGENT_SANDBOX="$WRAPPER_DIR/agent-sandbox"\n',
    'export ORCHESTRATOR_AGENT_SANDBOX="$WRAPPER_DIR/agent-sandbox"\nexport ORCHESTRATOR_VALIDATION_SANDBOX="$WRAPPER_DIR/validation-sandbox"\n',
    "export validation sandbox",
)
start = replace_once(
    start,
    "printf 'agent_process_sandbox=bubblewrap+private-dev+readonly-git+masked-host-state\\n\\n'\n",
    "printf 'agent_process_sandbox=bubblewrap+private-dev+readonly-git+masked-host-state\\n'\nprintf 'validation_sandbox=bubblewrap+private-net+readonly-git+masked-host-state\\n\\n'\n",
    "startup validation sandbox status",
)
start_path.write_text(start)

ci_path = Path(".github/workflows/ci.yml")
ci = ci_path.read_text()
ci = replace_once(
    ci,
    "          bash -n scripts/agent-sandbox\n",
    "          bash -n scripts/agent-sandbox\n          bash -n scripts/validation-sandbox\n",
    "validation sandbox syntax check",
)
ci = replace_once(
    ci,
    "      - name: Clippy\n        run: cargo clippy --workspace --all-targets -- -D warnings\n",
    '''      - name: Portable validation sandbox integration
        run: |
          sudo env \\
            PATH="$PATH" \\
            HOME="$HOME" \\
            CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \\
            RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \\
            CARGO_TARGET_DIR="$RUNNER_TEMP/orch2-root-target" \\
            ORCHESTRATOR_VALIDATION_SANDBOX="$GITHUB_WORKSPACE/scripts/validation-sandbox" \\
            ORCHESTRATOR_REAL_BWRAP="$(command -v bwrap)" \\
            cargo test --workspace portable_validation_ -- --nocapture
      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
''',
    "root portable validation CI gate",
)
ci_path.write_text(ci)
