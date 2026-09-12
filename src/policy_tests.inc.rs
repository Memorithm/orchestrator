fn temporary_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "orchestrator-policy-test-{name}-{}-{stamp}",
        std::process::id()
    ))
}

fn git(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn commit(directory: &Path, message: &str) -> String {
    git(directory, &["add", "-A"]);
    git(
        directory,
        &[
            "-c",
            "user.name=policy-test",
            "-c",
            "user.email=policy-test@example.invalid",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
    git(directory, &["rev-parse", "HEAD"])
}

#[test]
fn pointer_parser_deduplicates_and_rejects_unsafe_values() {
    let bootstrap = "read `origin/agent/roadmap:.agent/ROADMAP.yaml` then git show origin/agent/roadmap:.agent/ROADMAP.yaml";
    let pointers = extract_policy_pointers(bootstrap).unwrap();
    assert_eq!(pointers.len(), 1);
    assert_eq!(pointers[0].ref_name, "agent/roadmap");
    assert_eq!(pointers[0].path, ".agent/ROADMAP.yaml");

    assert!(extract_policy_pointers("origin/../bad:.agent/x.yaml").is_err());
    assert!(extract_policy_pointers("origin/agent/ok:../secret").is_err());
    assert!(extract_policy_pointers("origin/agent/*:.agent/x.yaml").is_err());
}

#[test]
fn snapshot_loads_exact_bootstrap_and_referenced_identity() {
    let root = temporary_root("load");
    let origin = root.join("origin.git");
    let work = root.join("work");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "--bare", origin.to_str().unwrap()]);
    git(&root, &["init", "-q", "-b", "main", work.to_str().unwrap()]);
    git(
        &work,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );

    fs::write(work.join("base.txt"), "base\n").unwrap();
    commit(&work, "base");
    git(&work, &["push", "-q", "-u", "origin", "main"]);

    git(&work, &["checkout", "-q", "-b", "agent/roadmap"]);
    fs::create_dir_all(work.join(".agent")).unwrap();
    fs::write(work.join(".agent/ROADMAP.yaml"), "status: active\n").unwrap();
    let roadmap_commit = commit(&work, "roadmap");
    git(&work, &["push", "-q", "-u", "origin", "agent/roadmap"]);

    git(&work, &["checkout", "-q", "main"]);
    fs::write(
        work.join("AGENTS.md"),
        "Mandatory: `origin/agent/roadmap:.agent/ROADMAP.yaml`\n",
    )
    .unwrap();
    let base_sha = commit(&work, "bootstrap");
    git(&work, &["push", "-q", "origin", "main"]);
    git(&work, &["fetch", "-q", "origin", "--prune"]);

    let snapshot = load_snapshot(&work, "Memorithm/Test", "main", &base_sha).unwrap();
    assert_eq!(snapshot.base_sha(), base_sha);
    assert_eq!(snapshot.base_branch(), "main");
    assert_eq!(snapshot.documents.len(), 1);
    assert_eq!(snapshot.documents[0].commit_sha, roadmap_commit);
    assert!(snapshot.prompt_context().contains("status: active"));
    assert!(remote_identity_is_current(&work, &snapshot).unwrap());

    git(&work, &["checkout", "-q", "agent/roadmap"]);
    fs::write(work.join(".agent/ROADMAP.yaml"), "status: changed\n").unwrap();
    commit(&work, "advance roadmap");
    git(&work, &["push", "-q", "origin", "agent/roadmap"]);
    assert!(!remote_identity_is_current(&work, &snapshot).unwrap());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repository_without_bootstrap_is_allowed() {
    let root = temporary_root("no-bootstrap");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    fs::write(root.join("README.md"), "test\n").unwrap();
    let base_sha = commit(&root, "base");
    let snapshot = load_snapshot(&root, "Memorithm/Test", "main", &base_sha).unwrap();
    assert!(snapshot.bootstrap.is_none());
    assert!(snapshot.documents.is_empty());
    let _ = fs::remove_dir_all(root);
}

fn snapshot_with_policy_documents(contents: &[&str]) -> PolicySnapshot {
    PolicySnapshot {
        repository: "Memorithm/Test".to_owned(),
        base_branch: "main".to_owned(),
        base_sha: "0".repeat(40),
        bootstrap: None,
        documents: contents
            .iter()
            .enumerate()
            .map(|(index, content)| PolicyDocument {
                ref_name: format!("agent/policy-{index}"),
                path: format!(".agent/POLICY-{index}.yaml"),
                commit_sha: format!("{:040x}", index + 1),
                blob_sha: format!("{:040x}", index + 101),
                content: (*content).to_owned(),
            })
            .collect(),
    }
}

#[test]
fn task_eligibility_blocks_explicit_human_only_roadmap_item() {
    let snapshot = snapshot_with_policy_documents(&[r#"schema_version: 1
roadmap:
  - id: TDI7_1
    status: complete_pending_human_confirmatory_execution
  - id: TDI7_2
    status: human_only_blocked
    execution_policy: explicit_human_only_confirmation_at_execution_time
    agent_policy: forbidden_to_initiate
  - id: TDIX
    status: planned_parallel
"#]);
    for spelling in ["TDI7_2", "TDI7.2", "TDI-7.2"] {
        let decision = snapshot
            .task_eligibility(&format!("Run {spelling} final holdout"), "")
            .unwrap();
        let TaskEligibility::Deferred(denial) = decision else {
            panic!("expected {spelling} to be denied");
        };
        assert_eq!(denial.item_id, "TDI7_2");
        assert_eq!(denial.field, "agent_policy");
    }
    assert_eq!(
        snapshot
            .task_eligibility("Advance TDIX evidence bridge", "")
            .unwrap(),
        TaskEligibility::Allowed
    );
    assert_eq!(
        snapshot
            .task_eligibility("Audit TDI7_20 fixture", "")
            .unwrap(),
        TaskEligibility::Allowed
    );
}

#[test]
fn contextual_body_mentions_do_not_target_a_prohibited_item() {
    let snapshot = snapshot_with_policy_documents(&[r#"roadmap:
  - id: TDI7_1
    status: active
  - id: TDI7_2
    status: human_only_blocked
    agent_policy: forbidden_to_initiate
"#]);
    let broad_issue_body = r#"# Active research programme — TDI-7.x
## TDI-7.1 — deterministic evaluator
CI proves normal tests cannot produce TDI-7.2 results.
TDI-7.1 must stop before final holdout execution.
## TDI-7.2 — confirmatory result
The final TDI-7.2 holdout remains blocked.
Current development target:
- TDI-7.1 deterministic evaluator.
"#;
    assert_eq!(
        snapshot
            .task_eligibility(
                "TDI-7.x — dynamic recovery diagnostics for attention",
                broad_issue_body,
            )
            .unwrap(),
        TaskEligibility::Allowed
    );

    let targeted = snapshot
        .task_eligibility(
            "Run confirmatory evaluation",
            "Target: TDI-7.2 final holdout",
        )
        .unwrap();
    assert!(matches!(targeted, TaskEligibility::Deferred(_)));
}

#[test]
fn task_eligibility_rejects_duplicate_or_conflicting_structured_policy() {
    let duplicate = snapshot_with_policy_documents(&[r#"roadmap:
  - id: X1
    status: human_only_blocked
    status: active
"#]);
    assert!(duplicate.task_eligibility("X1", "").is_err());

    let conflicting = snapshot_with_policy_documents(&[
        "roadmap:\n  - id: X2\n    status: active\n",
        "roadmap:\n  - id: X2\n    agent_policy: forbidden_to_initiate\n",
    ]);
    assert!(conflicting.task_eligibility("X2", "").is_err());
}

#[test]
fn global_action_policy_denies_only_explicit_matching_category() {
    let snapshot = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 1
  financial_execution: deny
  custody_mutation: allow
"#]);
    let denied = snapshot
        .task_eligibility(
            "Implement payout adapter",
            "Action category: financial_execution",
        )
        .unwrap();
    let TaskEligibility::Deferred(denial) = denied else {
        panic!("expected global financial deny");
    };
    assert_eq!(denial.item_id, "global:financial_execution");
    assert_eq!(denial.field, "autonomous_action_policy");
    assert!(
        denial
            .reason("Memorithm/Test", &snapshot)
            .contains("origin/agent/policy-0:.agent/POLICY-0.yaml")
    );

    assert_eq!(
        snapshot
            .task_eligibility("Update custody docs", "Autonomous action: custody_mutation",)
            .unwrap(),
        TaskEligibility::Allowed
    );
    assert_eq!(
        snapshot
            .task_eligibility("Analyze payout design", "")
            .unwrap(),
        TaskEligibility::Allowed
    );
}

#[test]
fn task_scoped_deny_precedes_global_allow() {
    let snapshot = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 1
  financial_execution: allow
roadmap:
  - id: FIN1
    agent_policy: forbidden_to_initiate
"#]);
    assert!(matches!(
        snapshot
            .task_eligibility("Run FIN1 payout", "Action category: financial_execution",)
            .unwrap(),
        TaskEligibility::Deferred(_)
    ));
}

#[test]
fn global_action_policy_is_strict_and_does_not_promote_free_text() {
    let free_text = snapshot_with_policy_documents(&[r#"notes: >-
  autonomous_action_policy: financial_execution deny
  custody and wallet changes are risky words only
"#]);
    assert_eq!(
        free_text
            .task_eligibility("Finance analysis", "Action category: financial_execution",)
            .unwrap(),
        TaskEligibility::Allowed
    );

    let unknown = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 1
  arbitrary_new_category: deny
"#]);
    assert!(
        unknown
            .task_eligibility("Work", "Action category: financial_execution")
            .is_err()
    );

    let future = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 2
  financial_execution: deny
"#]);
    assert!(
        future
            .task_eligibility("Work", "Action category: financial_execution")
            .is_err()
    );

    let conflicting = snapshot_with_policy_documents(&[
        "autonomous_action_policy:\n  schema_version: 1\n  financial_execution: deny\n",
        "autonomous_action_policy:\n  schema_version: 1\n  financial_execution: allow\n",
    ]);
    assert!(
        conflicting
            .task_eligibility("Work", "Action category: financial_execution")
            .is_err()
    );

    let duplicate_task = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 1
  financial_execution: deny
"#]);
    assert!(
        duplicate_task
            .task_eligibility(
                "Work",
                "Action category: financial_execution\nAutonomous action: custody_mutation",
            )
            .is_err()
    );
}

#[test]
fn portable_validation_plan_is_structured_and_bounded() {
    let snapshot = snapshot_with_policy_documents(&[r#"validation_plan:
  schema_version: 1
  class: portable
  steps:
    - id: fmt
      argv: [cargo, fmt, --all, --, --check]
      cwd: .
      timeout_seconds: 120
    - id: tests
      argv: [cargo, test, --workspace]
      cwd: crates/core
      timeout_seconds: 300
"#]);
    let plan = snapshot
        .portable_validation_plan()
        .unwrap()
        .expect("portable plan");
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[0].id, "fmt");
    assert_eq!(plan.steps[0].argv[0], "cargo");
    assert_eq!(plan.steps[1].cwd, "crates/core");
    assert_eq!(plan.steps[1].timeout_seconds, 300);
    assert_eq!(plan.source_ref, "agent/policy-0");
}

#[test]
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

#[test]
fn validation_plan_rejects_shell_unsafe_or_ambiguous_structure() {
    for content in [
        "validation_plan:\n  schema_version: 2\n  class: portable\n  steps:\n    - id: x\n      argv: [cargo, check]\n",
        "validation_plan:\n  schema_version: 1\n  class: hardware\n  steps:\n    - id: x\n      argv: [cargo, check]\n",
        "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [bash, -c, echo bad]\n",
        "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [../tool]\n",
        "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [cargo, check]\n      cwd: ../outside\n",
        "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [cargo, check]\n    - id: x\n      argv: [cargo, test]\n",
        "validation_plan:\n  schema_version: 1\n  class: portable\n  unknown: value\n  steps:\n    - id: x\n      argv: [cargo, check]\n",
    ] {
        assert!(
            snapshot_with_policy_documents(&[content])
                .portable_validation_plan()
                .is_err()
        );
    }
    let duplicate = snapshot_with_policy_documents(&[
        "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [cargo, check]\n",
        "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: y\n      argv: [cargo, test]\n",
    ]);
    assert!(duplicate.portable_validation_plan().is_err());
}

#[test]
fn validation_plan_is_not_inferred_from_free_text() {
    let snapshot = snapshot_with_policy_documents(&[r#"notes: >-
  validation_plan: cargo test --workspace
"#]);
    assert!(snapshot.portable_validation_plan().unwrap().is_none());
}

#[test]
fn merge_evidence_policy_distinguishes_portable_hardware_and_human() {
    let portable = snapshot_with_policy_documents(&[r#"merge_evidence_policy:
  schema_version: 1
  required: portable_ci
"#]);
    assert_eq!(
        portable.merge_evidence_eligibility().unwrap(),
        MergeEvidenceEligibility::PortableCi
    );
    assert_eq!(
        snapshot_with_policy_documents(&[])
            .merge_evidence_eligibility()
            .unwrap(),
        MergeEvidenceEligibility::Inherit
    );

    for required in ["hardware_required", "human_required"] {
        let content =
            format!("merge_evidence_policy:\n  schema_version: 1\n  required: {required}\n");
        let snapshot = snapshot_with_policy_documents(&[&content]);
        let MergeEvidenceEligibility::Deferred(denial) =
            snapshot.merge_evidence_eligibility().unwrap()
        else {
            panic!("expected v1 {required} to defer merge");
        };
        assert_eq!(denial.value, required);
        let reason = denial.merge_reason("Memorithm/Test", &snapshot);
        assert!(reason.contains(required));
        assert!(reason.contains("source=origin/agent/policy-0:.agent/POLICY-0.yaml"));
    }

    let hardware = snapshot_with_policy_documents(&[r#"merge_evidence_policy:
  schema_version: 2
  required: hardware_required
  requirement_id: jetson-thor-real-device
"#]);
    let MergeEvidenceEligibility::HardwareRequired(requirement) =
        hardware.merge_evidence_eligibility().unwrap()
    else {
        panic!("expected schema v2 hardware requirement");
    };
    assert_eq!(requirement.requirement_id(), "jetson-thor-real-device");
}

#[test]
fn merge_evidence_policy_is_strict_and_not_inferred_from_prose() {
    let prose = snapshot_with_policy_documents(&[r#"notes: >-
  merge_evidence_policy hardware_required
  physical GPU evidence is required in prose only
"#]);
    assert_eq!(
        prose.merge_evidence_eligibility().unwrap(),
        MergeEvidenceEligibility::Inherit
    );

    let duplicate = snapshot_with_policy_documents(&[
        "merge_evidence_policy:\n  schema_version: 1\n  required: portable_ci\n",
        "merge_evidence_policy:\n  schema_version: 2\n  required: hardware_required\n  requirement_id: gpu-a\n",
    ]);
    assert!(duplicate.merge_evidence_eligibility().is_err());

    for content in [
        "merge_evidence_policy:\n  schema_version: 3\n  required: hardware_required\n  requirement_id: gpu-a\n",
        "merge_evidence_policy:\n  schema_version: 2\n  required: hardware_required\n",
        "merge_evidence_policy:\n  schema_version: 2\n  required: portable_ci\n  requirement_id: gpu-a\n",
        "merge_evidence_policy:\n  schema_version: 2\n  required: human_required\n  requirement_id: gpu-a\n",
        "merge_evidence_policy:\n  schema_version: 1\n  required: hardware_required\n  requirement_id: gpu-a\n",
        "merge_evidence_policy:\n  schema_version: 2\n  required: hardware_required\n  requirement_id: ../gpu\n",
        "merge_evidence_policy:\n  schema_version: 2\n  required: hardware_required\n  requirement_id: gpu-a\n  signer_workflow: Memorithm/repo/.github/workflows/gpu.yml\n",
        "merge_evidence_policy:\n  schema_version: 1\n  required: self_reported_gpu\n",
        "merge_evidence_policy:\n  schema_version: 1\n  unknown: hardware_required\n",
        "merge_evidence_policy:\n  schema_version: 1\n  required: portable_ci\n  required: human_required\n",
        "merge_evidence_policy:\n  schema_version: 2\n  required: hardware_required\n  requirement_id: gpu-a\n  requirement_id: gpu-b\n",
    ] {
        assert!(
            snapshot_with_policy_documents(&[content])
                .merge_evidence_eligibility()
                .is_err()
        );
    }
}

#[test]
fn merge_policy_is_explicit_versioned_and_source_bound() {
    let denied = snapshot_with_policy_documents(&[r#"autonomous_merge_policy:
  schema_version: 1
  decision: deny
"#]);
    let MergeEligibility::Deferred(denial) = denied.merge_eligibility().unwrap() else {
        panic!("expected explicit merge deny");
    };
    assert_eq!(denial.item_id, "global:auto_merge");
    assert_eq!(denial.field, "autonomous_merge_policy");
    let reason = denial.merge_reason("Memorithm/Test", &denied);
    assert!(reason.contains("source=origin/agent/policy-0:.agent/POLICY-0.yaml"));
    assert!(reason.contains("policy_identity="));

    let allowed = snapshot_with_policy_documents(&[r#"autonomous_merge_policy:
  schema_version: 1
  decision: allow
"#]);
    assert_eq!(
        allowed.merge_eligibility().unwrap(),
        MergeEligibility::Allowed
    );
    assert_eq!(
        snapshot_with_policy_documents(&[])
            .merge_eligibility()
            .unwrap(),
        MergeEligibility::Inherit
    );
}

#[test]
fn merge_policy_rejects_ambiguous_or_unknown_structure() {
    let duplicate = snapshot_with_policy_documents(&[
        "autonomous_merge_policy:\n  schema_version: 1\n  decision: deny\n",
        "autonomous_merge_policy:\n  schema_version: 1\n  decision: allow\n",
    ]);
    assert!(duplicate.merge_eligibility().is_err());

    for content in [
        "autonomous_merge_policy:\n  schema_version: 2\n  decision: deny\n",
        "autonomous_merge_policy:\n  schema_version: 1\n  decision: maybe\n",
        "autonomous_merge_policy:\n  schema_version: 1\n  unknown: deny\n",
        "autonomous_merge_policy:\n  schema_version: 1\n  decision: deny\n  decision: allow\n",
    ] {
        assert!(
            snapshot_with_policy_documents(&[content])
                .merge_eligibility()
                .is_err()
        );
    }
}

#[test]
fn merge_policy_is_not_inferred_from_free_text() {
    let snapshot = snapshot_with_policy_documents(&[r#"notes: >-
  autonomous_merge_policy: decision deny
  do not auto merge financial work
"#]);
    assert_eq!(
        snapshot.merge_eligibility().unwrap(),
        MergeEligibility::Inherit
    );
}

#[test]
fn free_text_is_not_promoted_into_task_policy() {
    let snapshot = snapshot_with_policy_documents(&[r#"schema_version: 1
notes: >-
  agent_policy: forbidden_to_initiate and human_only_blocked are words here,
  not a roadmap item.
financial_rule: agents must never authorize custody from model output
"#]);
    assert_eq!(
        snapshot
            .task_eligibility("financial custody analysis", "forbidden_to_initiate")
            .unwrap(),
        TaskEligibility::Allowed
    );
}

#[test]
fn base_branch_validation_accepts_master_and_nonstandard_defaults() {
    for branch in ["master", "release/stable"] {
        let root = temporary_root(&branch.replace('/', "-"));
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", branch]);
        fs::write(root.join("README.md"), "test\n").unwrap();
        let base_sha = commit(&root, "base");
        let snapshot = load_snapshot(&root, "Memorithm/Test", branch, &base_sha).unwrap();
        assert_eq!(snapshot.base_branch(), branch);
        assert_eq!(snapshot.base_sha(), base_sha);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn identity_evidence_is_append_only_per_attempt() {
    let root = temporary_root("persist");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    fs::write(root.join("README.md"), "test\n").unwrap();
    let base_sha = commit(&root, "base");
    let snapshot = load_snapshot(&root, "Memorithm/Test", "main", &base_sha).unwrap();
    let first = persist_identity(&root, "Memorithm/Test", "ISSUE", 7, 100, &snapshot).unwrap();
    let second = persist_identity(&root, "Memorithm/Test", "ISSUE", 7, 100, &snapshot).unwrap();
    assert_ne!(first, second);
    for evidence in [first, second] {
        let contents = fs::read_to_string(&evidence).unwrap();
        assert!(contents.contains(&format!("base-sha={base_sha}")));
    }
    let _ = fs::remove_dir_all(root);
}
