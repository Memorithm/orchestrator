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
    '''        assert!(run_portable_validation_plan(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .is_err());
        assert!(!workspace.join(".git/validation-must-not-write").exists());
''',
    '''        run_portable_validation_plan(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        assert!(!workspace.join(".git/validation-must-not-write").exists());
''',
    "ephemeral git metadata test",
)
main = replace_once(
    main,
    "    fn portable_validation_masks_host_home() {\n",
    "    fn portable_validation_masks_host_credentials() {\n",
    "credential mask test name",
)

fallback_marker = '''    #[test]
    fn validation_without_structured_plan_keeps_diff_check_fallback() {
'''
extra_tests = r'''    #[test]
    fn portable_validation_clears_parent_credentials() {
        if !root_validation_sandbox_test_enabled() {
            return;
        }
        let root = orch2_validation_test_root("cleared-credentials");
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
            "credential",
            &["printenv", "GITHUB_TOKEN"],
            5,
        )]);
        let error = run_portable_validation_plan(
            &config,
            &workspace,
            &item,
            &snapshot,
            &plan,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap_err();
        assert!(error.contains("credential"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_validation_runs_cargo_offline_with_private_home() {
        if !root_validation_sandbox_test_enabled() {
            return;
        }
        let root = orch2_validation_test_root("cargo-offline");
        let data_root = root.join("data");
        let workspace = orch2_validation_test_workspace(&data_root);
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"orch2_sandbox_smoke\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(workspace.join("src/lib.rs"), "pub fn smoke() {}\n").unwrap();
        let config = orch2_validation_test_config(&data_root);
        let item = orch2_validation_test_item();
        let snapshot = policy::test_snapshot_for_validation(
            "Memorithm/Test",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        );
        let plan = orch2_validation_test_plan(vec![orch2_step(
            "cargo-offline",
            &["cargo", "check", "--offline"],
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
        assert!(workspace.join("target").is_dir());
        let _ = fs::remove_dir_all(root);
    }

'''
main = replace_once(main, fallback_marker, extra_tests + fallback_marker, "v2 execution tests")
main_path.write_text(main)

start_path = Path("scripts/start.sh")
start = start_path.read_text()
start = replace_once(
    start,
    'export ORCHESTRATOR_VALIDATION_SANDBOX="$WRAPPER_DIR/validation-sandbox"\n',
    'export ORCHESTRATOR_VALIDATION_SANDBOX="$WRAPPER_DIR/validation-sandbox"\nexport ORCHESTRATOR_SOURCE_ROOT="$ROOT"\n',
    "source root export",
)
start_path.write_text(start)

ci_path = Path(".github/workflows/ci.yml")
ci = ci_path.read_text()
old_gate = r'''      - name: Portable validation sandbox integration
        run: |
          sudo env \
            PATH="$PATH" \
            HOME="$HOME" \
            CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
            RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
            CARGO_TARGET_DIR="$RUNNER_TEMP/orch2-root-target" \
            ORCHESTRATOR_VALIDATION_SANDBOX="$GITHUB_WORKSPACE/scripts/validation-sandbox" \
            ORCHESTRATOR_REAL_BWRAP="$(command -v bwrap)" \
            cargo test --workspace portable_validation_ -- --nocapture
'''
new_gate = r'''      - name: Portable validation sandbox integration
        run: |
          cleanup_validation_secret() {
            sudo rm -f /root/.ssh/orchestrator-validation-ci-secret
          }
          trap cleanup_validation_secret EXIT
          sudo mkdir -p /root/.ssh
          sudo touch /root/.ssh/orchestrator-validation-ci-secret
          sudo env \
            PATH="$PATH" \
            HOME="$HOME" \
            CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
            RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
            CARGO_TARGET_DIR="$RUNNER_TEMP/orch2-root-target" \
            GITHUB_TOKEN="must-not-cross-validation-sandbox" \
            ORCHESTRATOR_SOURCE_ROOT="$GITHUB_WORKSPACE" \
            ORCHESTRATOR_VALIDATION_SANDBOX="$GITHUB_WORKSPACE/scripts/validation-sandbox" \
            ORCHESTRATOR_REAL_BWRAP="$(command -v bwrap)" \
            cargo test --workspace portable_validation_ -- --nocapture
'''
ci = replace_once(ci, old_gate, new_gate, "credential-aware root CI gate")
ci_path.write_text(ci)
