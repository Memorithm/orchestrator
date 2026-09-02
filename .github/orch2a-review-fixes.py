from pathlib import Path


def replace_once(data: str, old: str, new: str, label: str) -> str:
    count = data.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return data.replace(old, new, 1)


main_path = Path("src/main.rs")
main = main_path.read_text()

main = replace_once(
    main,
    '''struct ValidationStepResult<'a> {
    worktree_head: &'a str,
    started_at: u64,
    finished_at: u64,
    exit_code: Option<i32>,
    timed_out: bool,
}
''',
    '''struct ValidationStepResult<'a> {
    worktree_head: &'a str,
    worktree_tree: &'a str,
    started_at: u64,
    finished_at: u64,
    exit_code: Option<i32>,
    timed_out: bool,
}
''',
    "validation result tree identity",
)
main = replace_once(
    main,
    '''    let ValidationStepResult {
        worktree_head,
        started_at,
        finished_at,
        exit_code,
        timed_out,
    } = result;
''',
    '''    let ValidationStepResult {
        worktree_head,
        worktree_tree,
        started_at,
        finished_at,
        exit_code,
        timed_out,
    } = result;
''',
    "validation evidence destructure",
)
main = replace_once(
    main,
    '''        "validation-schema=1\\nclass=portable\\nrepository={}\\nwork-kind={}\\nwork-number={}\\nstep-id={}\\nargv-hex={}\\ncwd={}\\ntimeout-seconds={}\\nstarted-at={}\\nfinished-at={}\\nexit-code={}\\ntimed-out={}\\npolicy-identity={}\\nbase-sha={}\\nworktree-head={}\\nsource-ref=origin/{}\\nsource-path={}\\nsource-commit={}\\nsource-blob={}\\n",
''',
    '''        "validation-schema=1\\nclass=portable\\nrepository={}\\nwork-kind={}\\nwork-number={}\\nstep-id={}\\nargv-hex={}\\ncwd={}\\ntimeout-seconds={}\\nstarted-at={}\\nfinished-at={}\\nexit-code={}\\ntimed-out={}\\npolicy-identity={}\\nbase-sha={}\\nworktree-head={}\\nworktree-tree={}\\nsource-ref=origin/{}\\nsource-path={}\\nsource-commit={}\\nsource-blob={}\\n",
''',
    "validation evidence record field",
)
main = replace_once(
    main,
    '''        snapshot.base_sha(),
        worktree_head,
        plan.source_ref,
''',
    '''        snapshot.base_sha(),
        worktree_head,
        worktree_tree,
        plan.source_ref,
''',
    "validation evidence record value",
)

marker = '''fn validation_sandbox_path() -> Result<PathBuf, String> {
'''
helper = '''fn validation_worktree_tree(config: &RunConfig, workspace: &Path) -> Result<String, String> {
    let directory = config.data_root.join("state/validation-index");
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "failed to create validation index directory {}: {error}",
            directory.display()
        )
    })?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_nanos();
    let index = directory.join(format!(
        "{}-{stamp}.index",
        std::process::id()
    ));

    let result = (|| {
        let run = |args: &[&str]| -> Result<(), String> {
            let status = Command::new("git")
                .current_dir(workspace)
                .env("GIT_INDEX_FILE", &index)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .args(args)
                .status()
                .map_err(|error| {
                    format!("failed to fingerprint validation worktree with git: {error}")
                })?;
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "git {} failed while fingerprinting validation worktree with {status}",
                    args.join(" ")
                ))
            }
        };

        run(&["read-tree", "HEAD"])?;
        run(&["add", "-A", "--", "."])?;
        let output = Command::new("git")
            .current_dir(workspace)
            .env("GIT_INDEX_FILE", &index)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["write-tree"])
            .output()
            .map_err(|error| format!("failed to hash validation worktree tree: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "git write-tree failed while fingerprinting validation worktree: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let tree = String::from_utf8(output.stdout)
            .map_err(|error| format!("invalid UTF-8 from git write-tree: {error}"))?;
        let tree = tree.trim();
        if !matches!(tree.len(), 40 | 64) || !tree.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("invalid validation worktree tree id: {tree:?}"));
        }
        Ok(tree.to_owned())
    })();
    let _ = fs::remove_file(index);
    result
}

'''
main = replace_once(main, marker, helper + marker, "validation worktree tree helper")

main = replace_once(
    main,
    '''fn run_portable_validation_plan(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    worktree_head: &str,
) -> Result<(), String> {
    for step in &plan.steps {
        run_portable_validation_step(config, workspace, item, snapshot, plan, step, worktree_head)?;
    }
    Ok(())
}
''',
    '''fn run_portable_validation_plan(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    worktree_head: &str,
) -> Result<(), String> {
    let worktree_tree = validation_worktree_tree(config, workspace)?;
    for step in &plan.steps {
        run_portable_validation_step(
            config,
            workspace,
            item,
            snapshot,
            plan,
            step,
            worktree_head,
            &worktree_tree,
        )?;
    }
    Ok(())
}
''',
    "validation plan exact tree binding",
)
main = replace_once(
    main,
    '''    step: &policy::PortableValidationStep,
    worktree_head: &str,
) -> Result<(), String> {
''',
    '''    step: &policy::PortableValidationStep,
    worktree_head: &str,
    worktree_tree: &str,
) -> Result<(), String> {
''',
    "validation step tree argument",
)
main = replace_once(
    main,
    '''        &ValidationStepResult {
            worktree_head,
            started_at,
''',
    '''        &ValidationStepResult {
            worktree_head,
            worktree_tree,
            started_at,
''',
    "runtime evidence tree",
)

main = replace_once(
    main,
    '''        fs::create_dir_all(&workspace).unwrap();
        run_in_dir(&workspace, "git", &["init", "-q", "-b", "main"]).unwrap();
        workspace
''',
    '''        fs::create_dir_all(&workspace).unwrap();
        run_in_dir(&workspace, "git", &["init", "-q", "-b", "main"]).unwrap();
        fs::write(workspace.join(".gitignore"), "target/\\n").unwrap();
        commit_changes(&workspace, "test: validation baseline").unwrap();
        workspace
''',
    "validation test baseline commit",
)
main = replace_once(
    main,
    '''        let plan = orch2_validation_test_plan(vec![
            orch2_step("mkdir", &["mkdir", "ordered"], 5),
            orch2_step("dependent", &["touch", "ordered/second"], 5),
            orch2_step("literal", &["touch", "literal;touch injected"], 5),
        ]);
''',
    '''        let plan = orch2_validation_test_plan(vec![
            orch2_step("first", &["true"], 5),
            orch2_step("second", &["true"], 5),
            orch2_step(
                "literal",
                &[
                    "test",
                    "literal;touch injected",
                    "=",
                    "literal;touch injected",
                ],
                5,
            ),
        ]);
''',
    "non-mutating order literal plan",
)
main = replace_once(
    main,
    '''        .unwrap();
        assert!(workspace.join("ordered/second").is_file());
        assert!(workspace.join("literal;touch injected").is_file());
        assert!(!workspace.join("literal").exists());
        assert!(!workspace.join("injected").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_validation_stops_after_first_failure() {
''',
    '''        .unwrap();
        assert!(!workspace.join("literal").exists());
        assert!(!workspace.join("injected").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_validation_stops_after_first_failure() {
''',
    "non-mutating order assertions",
)
main = replace_once(
    main,
    '''        let plan = orch2_validation_test_plan(vec![
            orch2_step("first", &["touch", "first"], 5),
            orch2_step("fail", &["false"], 5),
            orch2_step("third", &["touch", "third"], 5),
        ]);
''',
    '''        let plan = orch2_validation_test_plan(vec![
            orch2_step("first", &["true"], 5),
            orch2_step("fail", &["false"], 5),
            orch2_step("third", &["touch", "third"], 5),
        ]);
''',
    "non-mutating pre-failure step",
)
main = replace_once(
    main,
    '''        assert!(error.contains("fail"));
        assert!(workspace.join("first").is_file());
        assert!(!workspace.join("third").exists());
''',
    '''        assert!(error.contains("fail"));
        assert!(!workspace.join("third").exists());
''',
    "stop failure assertions",
)
main = replace_once(
    main,
    '''        .unwrap();
        assert!(workspace.join("target").is_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validation_without_structured_plan_keeps_diff_check_fallback() {
''',
    '''        .unwrap();
        assert!(!workspace.join("target").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_validation_rejects_worktree_mutation() {
        if !root_validation_sandbox_test_enabled() {
            return;
        }
        let root = orch2_validation_test_root("readonly-worktree");
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
            "worktree-mutation",
            &["touch", "must-not-persist"],
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
        assert!(!workspace.join("must-not-persist").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validation_worktree_tree_binds_dirty_and_untracked_content() {
        let root = orch2_validation_test_root("tree-binding");
        let data_root = root.join("data");
        let workspace = orch2_validation_test_workspace(&data_root);
        let config = orch2_validation_test_config(&data_root);
        fs::write(workspace.join("candidate.txt"), "first\\n").unwrap();
        let first = validation_worktree_tree(&config, &workspace).unwrap();
        fs::write(workspace.join("candidate.txt"), "second\\n").unwrap();
        let second = validation_worktree_tree(&config, &workspace).unwrap();
        fs::write(workspace.join("untracked.txt"), "extra\\n").unwrap();
        let third = validation_worktree_tree(&config, &workspace).unwrap();
        assert_ne!(first, second);
        assert_ne!(second, third);
        assert!(!data_root.join("state/validation-index").read_dir().unwrap().any(|entry| {
            entry
                .ok()
                .is_some_and(|entry| entry.path().extension().is_some_and(|ext| ext == "index"))
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validation_without_structured_plan_keeps_diff_check_fallback() {
''',
    "worktree mutation and tree binding tests",
)
main = main.replace(
    '''                worktree_head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                started_at: 100,
''',
    '''                worktree_head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                worktree_tree: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                started_at: 100,
''',
)
if main.count('worktree_tree: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"') != 2:
    raise SystemExit("evidence fixtures: expected two tree identities")
main = replace_once(
    main,
    '''        assert!(contents.contains("worktree-head=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(contents.contains("policy-identity="));
''',
    '''        assert!(contents.contains("worktree-head=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(contents.contains("worktree-tree=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
        assert!(contents.contains("policy-identity="));
''',
    "evidence tree assertion",
)
main_path.write_text(main)

policy_path = Path("src/policy.rs")
policy = policy_path.read_text()
old_parser = '''fn parse_validation_argv(raw: &str) -> Result<Vec<String>, String> {
    let raw = raw.trim();
    if !raw.starts_with('[') || !raw.ends_with(']') {
        return Err("validation argv must use a bracketed scalar list".to_owned());
    }
    let inner = &raw[1..raw.len() - 1];
    if inner.trim().is_empty() {
        return Err("validation argv must not be empty".to_owned());
    }
    let mut argv = Vec::new();
    for raw_arg in inner.split(',') {
        if argv.len() >= MAX_VALIDATION_ARGV {
            return Err(format!(
                "validation argv exceeds {MAX_VALIDATION_ARGV} elements"
            ));
        }
        let arg = parse_policy_scalar(raw_arg.trim(), "validation argv")?;
        if arg.is_empty()
            || arg.chars().count() > MAX_VALIDATION_ARG_CHARS
            || arg.chars().any(char::is_control)
        {
            return Err("invalid validation argv element".to_owned());
        }
        argv.push(arg);
    }
    let executable = argv.first().expect("non-empty checked above");
    if executable.contains('/')
        || executable.contains('\\\\')
        || !executable.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '+')
        })
    {
        return Err(format!("unsafe validation executable: {executable:?}"));
    }
    if matches!(
        executable.as_str(),
        "git"
            | "gh"
            | "ssh"
            | "scp"
            | "curl"
            | "wget"
            | "bash"
            | "sh"
            | "zsh"
            | "fish"
            | "sudo"
            | "su"
            | "env"
            | "xargs"
            | "ollama"
            | "opencode"
    ) {
        return Err(format!(
            "validation executable is forbidden in portable plan v1: {executable}"
        ));
    }
    Ok(argv)
}
'''
new_parser = '''fn split_validation_argv_items(inner: &str) -> Result<Vec<&str>, String> {
    let mut items = Vec::new();
    let mut quote = None;
    let mut start = 0;
    for (index, character) in inner.char_indices() {
        match quote {
            Some(expected) if character == expected => quote = None,
            Some(_) => {}
            None if matches!(character, '\\'' | '"') => quote = Some(character),
            None if character == ',' => {
                items.push(&inner[start..index]);
                start = index + character.len_utf8();
            }
            None => {}
        }
    }
    if quote.is_some() {
        return Err("validation argv contains an unterminated quoted scalar".to_owned());
    }
    items.push(&inner[start..]);
    Ok(items)
}

fn parse_validation_argv(raw: &str) -> Result<Vec<String>, String> {
    let raw = raw.trim();
    if !raw.starts_with('[') || !raw.ends_with(']') {
        return Err("validation argv must use a bracketed scalar list".to_owned());
    }
    let inner = &raw[1..raw.len() - 1];
    if inner.trim().is_empty() {
        return Err("validation argv must not be empty".to_owned());
    }
    let mut argv = Vec::new();
    for raw_arg in split_validation_argv_items(inner)? {
        if argv.len() >= MAX_VALIDATION_ARGV {
            return Err(format!(
                "validation argv exceeds {MAX_VALIDATION_ARGV} elements"
            ));
        }
        let arg = parse_policy_scalar(raw_arg.trim(), "validation argv")?;
        if arg.is_empty()
            || arg.chars().count() > MAX_VALIDATION_ARG_CHARS
            || arg.chars().any(char::is_control)
        {
            return Err("invalid validation argv element".to_owned());
        }
        argv.push(arg);
    }
    let executable = argv.first().expect("non-empty checked above");
    if executable.contains('/')
        || executable.contains('\\\\')
        || !executable.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '+')
        })
    {
        return Err(format!("unsafe validation executable: {executable:?}"));
    }
    if matches!(
        executable.as_str(),
        "git"
            | "gh"
            | "ssh"
            | "scp"
            | "curl"
            | "wget"
            | "bash"
            | "sh"
            | "zsh"
            | "fish"
            | "sudo"
            | "su"
            | "env"
            | "xargs"
            | "ollama"
            | "opencode"
    ) {
        return Err(format!(
            "validation executable is forbidden in portable plan v1: {executable}"
        ));
    }
    Ok(argv)
}
'''
policy = replace_once(policy, old_parser, new_parser, "quote-aware validation argv parser")
policy = replace_once(
    policy,
    '''        assert_eq!(plan.steps[0].argv, ["touch", "literal;touch injected"]);
    }
''',
    '''        assert_eq!(plan.steps[0].argv, ["touch", "literal;touch injected"]);

        let quoted = snapshot_with_policy_documents(&[r#"validation_plan:
  schema_version: 1
  class: portable
  steps:
    - id: features
      argv: [cargo, test, --features, "foo,bar"]
"#]);
        let quoted_plan = quoted
            .portable_validation_plan()
            .unwrap()
            .expect("quoted portable plan");
        assert_eq!(
            quoted_plan.steps[0].argv,
            ["cargo", "test", "--features", "foo,bar"]
        );
    }
''',
    "quoted comma parser test",
)
policy_path.write_text(policy)

sandbox_path = Path("scripts/validation-sandbox")
sandbox = sandbox_path.read_text()
sandbox = replace_once(
    sandbox,
    '''  --bind "$WORKSPACE" "$internal_workspace"\n''',
    '''  --ro-bind "$WORKSPACE" "$internal_workspace"\n''',
    "read-only validation workspace",
)
sandbox = replace_once(
    sandbox,
    '''  --setenv CARGO_NET_OFFLINE true\n''',
    '''  --setenv CARGO_NET_OFFLINE true\n  --setenv CARGO_TARGET_DIR /tmp/cargo-target\n''',
    "private cargo target",
)
sandbox_path.write_text(sandbox)
