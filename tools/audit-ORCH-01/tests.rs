// Included in main.rs's test module so these regressions use the same private
// validation entrypoints and fixtures as the existing portable-plan tests.

#[test]
fn portable_validation_missing_plan_never_builds_or_reuses_cargo() {
    let root = orch2_validation_test_root("missing-plan");
    let data_root = root.join("data");
    let workspace = orch2_validation_test_workspace(&data_root);
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
    let snapshot = policy::test_snapshot_for_validation(
        "Memorithm/Test",
        "main",
        "0123456789abcdef0123456789abcdef01234567",
    );
    assert!(snapshot.portable_validation_plan().unwrap().is_none());
    let item = orch2_validation_test_item();
    let mut config = orch2_validation_test_config(&data_root);
    for full in [false, true] {
        config.full_validation = full;
        for reuse in [false, true] {
            let error = validate_workspace_internal(
                &config, &workspace, &item, &snapshot, reuse,
            )
            .unwrap_err();
            assert!(error.contains("portable validation plan required"), "{error}");
            assert!(!marker.exists());
            assert!(!workspace.join("target").exists());
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn portable_validation_missing_plan_rejects_manifest_directory() {
    let root = orch2_validation_test_root("manifest-directory");
    let data_root = root.join("data");
    let workspace = orch2_validation_test_workspace(&data_root);
    fs::create_dir(workspace.join("Cargo.toml")).unwrap();
    let config = orch2_validation_test_config(&data_root);
    let item = orch2_validation_test_item();
    let snapshot = policy::test_snapshot_for_validation(
        "Memorithm/Test",
        "main",
        "0123456789abcdef0123456789abcdef01234567",
    );
    let error = validate_workspace(&config, &workspace, &item, &snapshot).unwrap_err();
    assert!(error.contains("portable validation plan required"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn portable_validation_missing_plan_rejects_dangling_manifest() {
    let root = orch2_validation_test_root("dangling-manifest");
    let data_root = root.join("data");
    let workspace = orch2_validation_test_workspace(&data_root);
    std::os::unix::fs::symlink("absent-manifest", workspace.join("Cargo.toml")).unwrap();
    let config = orch2_validation_test_config(&data_root);
    let item = orch2_validation_test_item();
    let snapshot = policy::test_snapshot_for_validation(
        "Memorithm/Test",
        "main",
        "0123456789abcdef0123456789abcdef01234567",
    );
    let error = validate_workspace_reusing_passed(&config, &workspace, &item, &snapshot)
        .unwrap_err();
    assert!(error.contains("portable validation plan required"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn portable_validation_documentation_only_keeps_diff_check() {
    let root = orch2_validation_test_root("documentation-only");
    let data_root = root.join("data");
    let workspace = orch2_validation_test_workspace(&data_root);
    let config = orch2_validation_test_config(&data_root);
    let item = orch2_validation_test_item();
    let snapshot = policy::test_snapshot_for_validation(
        "Memorithm/Test",
        "main",
        "0123456789abcdef0123456789abcdef01234567",
    );
    validate_workspace(&config, &workspace, &item, &snapshot).unwrap();
    validate_workspace_reusing_passed(&config, &workspace, &item, &snapshot).unwrap();
    fs::write(workspace.join("tracked.md"), "baseline\n").unwrap();
    commit_changes(&workspace, "test: tracked document").unwrap();
    fs::write(workspace.join("tracked.md"), "trailing whitespace \n").unwrap();
    assert!(validate_workspace(&config, &workspace, &item, &snapshot).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn portable_validation_build_script_blocks_host_secret_and_host_network() {
    if !root_validation_sandbox_test_enabled() {
        return;
    }
    let root = orch2_validation_test_root("build-script-boundary");
    let data_root = root.join("data");
    let workspace = orch2_validation_test_workspace(&data_root);
    fs::create_dir_all(workspace.join("src")).unwrap();
    let sentinel = data_root.join("host-only-sentinel");
    fs::write(&sentinel, "public-test-canary-not-a-real-secret").unwrap();
    assert_eq!(
        fs::read_to_string(&sentinel).unwrap(),
        "public-test-canary-not-a-real-secret"
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(std::net::TcpStream::connect(address).unwrap());
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"isolated_build_probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n",
    )
    .unwrap();
    // Successful compilation requires the build script to execute and emit
    // this include only after every isolation assertion succeeds.
    fs::write(
        workspace.join("src/lib.rs"),
        "include!(concat!(env!(\"OUT_DIR\"), \"verified.rs\"));\n",
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
    run_in_dir(&workspace, "cargo", &["generate-lockfile", "--offline"]).unwrap();
    let config = orch2_validation_test_config(&data_root);
    let item = orch2_validation_test_item();
    let snapshot = policy::test_snapshot_for_validation(
        "Memorithm/Test",
        "main",
        "0123456789abcdef0123456789abcdef01234567",
    );
    let plan = orch2_validation_test_plan(vec![orch2_step(
        "build-script-boundary",
        &["cargo", "check", "--offline", "--locked"],
        30,
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
    assert!(!workspace.join("target").exists());
    assert_eq!(
        fs::read_to_string(&sentinel).unwrap(),
        "public-test-canary-not-a-real-secret"
    );
    drop(listener);
    fs::remove_dir_all(root).unwrap();
}
