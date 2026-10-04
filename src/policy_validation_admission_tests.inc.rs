// These tests share the existing policy fixtures, but exercise the actual
// orchestrator validation entrypoints rather than a mock admission result.

fn admission_config(data_root: &Path) -> crate::RunConfig {
    crate::RunConfig {
        organization: "Memorithm".to_owned(),
        model: crate::DEFAULT_MODEL.to_owned(),
        interval: std::time::Duration::from_secs(1),
        data_root: data_root.to_path_buf(),
        auto_merge: false,
        auto_merge_scope: crate::merge_policy::AutoMergeScope::OrchestratorValidated,
        full_validation: false,
        max_cycles: 1,
        resource_policy: crate::resource::ResourcePolicy {
            min_available_memory_mb: 0,
            min_free_disk_mb: 0,
            max_load_per_cpu: 0.0,
        },
        low_disk_reclaim_max_targets: 1,
        low_disk_reclaim_max_workspaces: 1,
        workspace_min_idle_secs: 1,
        trajectory_max_per_item: 1,
        retry_policy: crate::state::RetryPolicy::default(),
    }
}

fn admission_item() -> crate::WorkItem {
    crate::WorkItem {
        kind: crate::WorkKind::Issue,
        repository: "Memorithm/Test".to_owned(),
        number: 45,
        title: "validation admission".to_owned(),
        detail: "audit ORCH-01".to_owned(),
        source_revision: Some("issue-v1".to_owned()),
        ci_state: None,
        draft: false,
    }
}

fn admission_workspace(data_root: &Path) -> PathBuf {
    let workspace = data_root.join("workspaces/Memorithm__Test");
    fs::create_dir_all(&workspace).unwrap();
    git(&workspace, &["init", "-q", "-b", "main"]);
    fs::write(workspace.join("README.md"), "validation fixture\n").unwrap();
    fs::write(workspace.join(".gitignore"), "target/\n").unwrap();
    commit(&workspace, "test: validation baseline");
    workspace
}

fn assert_unplanned_validation_refused(workspace: &Path, data_root: &Path) {
    let mut config = admission_config(data_root);
    let item = admission_item();
    let snapshot = snapshot_with_policy_documents(&[]);
    for full in [false, true] {
        config.full_validation = full;
        for reuse in [false, true] {
            let error = crate::validate_workspace_internal(
                &config, workspace, &item, &snapshot, reuse,
            )
            .unwrap_err();
            assert!(error.contains("portable validation plan required"), "{error}");
        }
    }
}

#[test]
fn portable_validation_no_plan_never_builds_or_reuses_cargo() {
    let root = temporary_root("no-plan-cargo");
    let data_root = root.join("data");
    let workspace = admission_workspace(&data_root);
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"unplanned_candidate\"\nversion = \"0.0.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n",
    )
    .unwrap();
    fs::write(workspace.join("src/lib.rs"), "pub fn candidate() {}\n").unwrap();
    let marker = root.join("unplanned-build-executed");
    fs::write(
        workspace.join("build.rs"),
        format!(
            "fn main() {{\n    std::fs::write({:?}, b\"unexpected\").unwrap();\n}}\n",
            marker.to_str().unwrap()
        ),
    )
    .unwrap();
    assert_unplanned_validation_refused(&workspace, &data_root);
    assert!(!marker.exists());
    assert!(!workspace.join("target").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn portable_validation_no_plan_refuses_documentation_only_candidate() {
    let root = temporary_root("no-plan-docs");
    let data_root = root.join("data");
    let workspace = admission_workspace(&data_root);
    let snapshot = snapshot_with_policy_documents(&[]);
    // Read-only policy inspection still works. Candidate validation is a
    // different operation and cannot infer an executable plan from absence.
    assert!(snapshot.prompt_context().contains("bootstrap: AGENTS.md absent"));
    assert_unplanned_validation_refused(&workspace, &data_root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn portable_validation_no_plan_refuses_manifest_directory() {
    let root = temporary_root("no-plan-directory");
    let data_root = root.join("data");
    let workspace = admission_workspace(&data_root);
    fs::create_dir(workspace.join("Cargo.toml")).unwrap();
    assert_unplanned_validation_refused(&workspace, &data_root);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn portable_validation_no_plan_refuses_dangling_manifest() {
    let root = temporary_root("no-plan-symlink");
    let data_root = root.join("data");
    let workspace = admission_workspace(&data_root);
    std::os::unix::fs::symlink("absent-manifest", workspace.join("Cargo.toml")).unwrap();
    assert_unplanned_validation_refused(&workspace, &data_root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn portable_validation_keeps_diff_check_before_plan_selection() {
    let root = temporary_root("diff-before-plan");
    let data_root = root.join("data");
    let workspace = admission_workspace(&data_root);
    fs::write(workspace.join("README.md"), "trailing whitespace \n").unwrap();
    let error = crate::validate_workspace(
        &admission_config(&data_root),
        &workspace,
        &admission_item(),
        &snapshot_with_policy_documents(&[]),
    )
    .unwrap_err();
    assert!(error.contains("git diff --check failed"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn portable_validation_real_build_script_cannot_read_host_secret_or_connect() {
    if !crate::capture("id", &["-u"]).is_ok_and(|uid| uid == "0")
        || !crate::command_available("bwrap")
    {
        return;
    }
    // The existing root CI step supplies a public synthetic GITHUB_TOKEN.
    // This precondition prevents claiming environment stripping vacuously.
    assert!(std::env::var_os("GITHUB_TOKEN").is_some());
    let root = temporary_root("build-script-isolation");
    let data_root = root.join("data");
    let workspace = admission_workspace(&data_root);
    fs::create_dir_all(workspace.join("src")).unwrap();
    let sentinel = data_root.join("host-only-sentinel");
    let canary = "public-test-canary-not-a-real-secret";
    fs::write(&sentinel, canary).unwrap();
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), canary);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(std::net::TcpStream::connect(address).unwrap());
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"isolated_build_probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n",
    )
    .unwrap();
    fs::write(
        workspace.join("src/lib.rs"),
        "include!(concat!(env!(\"OUT_DIR\"), \"/verified.rs\"));\n",
    )
    .unwrap();
    fs::write(
        workspace.join("build.rs"),
        format!(
            r#"fn main() {{
    assert!(std::fs::read({sentinel:?}).is_err(), "host sentinel visible");
    assert!(std::env::var_os("GITHUB_TOKEN").is_none(), "parent token inherited");
    let address: std::net::SocketAddr = {address:?}.parse().unwrap();
    assert!(std::net::TcpStream::connect_timeout(
        &address, std::time::Duration::from_millis(250)
    ).is_err(), "host loopback listener reachable");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("verified.rs"),
        "pub const ISOLATION_PROBED: bool = true;\n").unwrap();
}}
"#,
            sentinel = sentinel.to_str().unwrap(),
            address = address.to_string(),
        ),
    )
    .unwrap();
    crate::run_in_dir(&workspace, "cargo", &["generate-lockfile", "--offline"]).unwrap();
    let snapshot = snapshot_with_policy_documents(&[r#"validation_plan:
  schema_version: 1
  class: portable
  steps:
    - id: build-script-boundary
      argv: [cargo, check, --offline, --locked]
      cwd: .
      timeout_seconds: 30
"#]);
    crate::validate_workspace(
        &admission_config(&data_root),
        &workspace,
        &admission_item(),
        &snapshot,
    )
    .unwrap();
    assert!(!workspace.join("target").exists());
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), canary);
    drop(listener);
    fs::remove_dir_all(root).unwrap();
}
