from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one anchor, got {count}")
    return text.replace(old, new, 1)


policy_path = Path("src/policy.rs")
policy = policy_path.read_text()

old_defs = '''#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MergeEvidenceEligibility {
    Inherit,
    PortableCi,
    Deferred(PolicyDenial),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeEvidenceClass {
'''
new_defs = '''#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MergeEvidenceEligibility {
    Inherit,
    PortableCi,
    HardwareRequired(HardwareEvidenceRequirement),
    Deferred(PolicyDenial),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HardwareEvidenceRequirement {
    requirement_id: String,
}

impl HardwareEvidenceRequirement {
    pub(crate) fn requirement_id(&self) -> &str {
        &self.requirement_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeEvidenceClass {
'''
policy = replace_once(policy, old_defs, new_defs, "merge evidence eligibility definitions")

old_rule = '''#[derive(Debug, Clone, PartialEq, Eq)]
struct MergeEvidenceRule {
    required: MergeEvidenceClass,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
}
'''
new_rule = '''#[derive(Debug, Clone, PartialEq, Eq)]
struct MergeEvidenceRule {
    schema_version: u8,
    required: MergeEvidenceClass,
    requirement_id: Option<String>,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
}
'''
policy = replace_once(policy, old_rule, new_rule, "merge evidence rule")

old_method = '''    pub(crate) fn merge_evidence_eligibility(&self) -> Result<MergeEvidenceEligibility, String> {
        let mut selected: Option<MergeEvidenceRule> = None;
        for document in &self.documents {
            let Some(rule) = parse_merge_evidence_policy(document)? else {
                continue;
            };
            if selected.replace(rule).is_some() {
                return Err(
                    "duplicate merge evidence policy across mandatory policy documents".to_owned(),
                );
            }
        }
        let Some(rule) = selected else {
            return Ok(MergeEvidenceEligibility::Inherit);
        };
        if rule.required == MergeEvidenceClass::PortableCi {
            return Ok(MergeEvidenceEligibility::PortableCi);
        }
        Ok(MergeEvidenceEligibility::Deferred(PolicyDenial {
            item_id: format!("evidence:{}", rule.required.as_str()),
            field: "merge_evidence_policy",
            value: rule.required.as_str().to_owned(),
            source_ref: rule.source_ref,
            source_path: rule.source_path,
            source_commit: rule.source_commit,
            source_blob: rule.source_blob,
        }))
    }
'''
new_method = '''    pub(crate) fn merge_evidence_eligibility(&self) -> Result<MergeEvidenceEligibility, String> {
        let mut selected: Option<MergeEvidenceRule> = None;
        for document in &self.documents {
            let Some(rule) = parse_merge_evidence_policy(document)? else {
                continue;
            };
            if selected.replace(rule).is_some() {
                return Err(
                    "duplicate merge evidence policy across mandatory policy documents".to_owned(),
                );
            }
        }
        let Some(rule) = selected else {
            return Ok(MergeEvidenceEligibility::Inherit);
        };
        if rule.schema_version == 2 {
            if rule.required != MergeEvidenceClass::HardwareRequired {
                return Err("merge evidence schema v2 is reserved for hardware_required".to_owned());
            }
            let requirement_id = rule.requirement_id.ok_or_else(|| {
                "merge evidence schema v2 hardware_required is missing requirement_id".to_owned()
            })?;
            return Ok(MergeEvidenceEligibility::HardwareRequired(
                HardwareEvidenceRequirement { requirement_id },
            ));
        }
        if rule.required == MergeEvidenceClass::PortableCi {
            return Ok(MergeEvidenceEligibility::PortableCi);
        }
        Ok(MergeEvidenceEligibility::Deferred(PolicyDenial {
            item_id: format!("evidence:{}", rule.required.as_str()),
            field: "merge_evidence_policy",
            value: rule.required.as_str().to_owned(),
            source_ref: rule.source_ref,
            source_path: rule.source_path,
            source_commit: rule.source_commit,
            source_blob: rule.source_blob,
        }))
    }
'''
policy = replace_once(policy, old_method, new_method, "merge evidence eligibility method")

start = policy.index("fn parse_merge_evidence_policy(\n")
end = policy.index("fn parse_merge_policy(", start)
new_parser = r'''fn validate_hardware_requirement_id(value: &str) -> Result<(), String> {
    const MAX_CHARS: usize = 96;
    if value.is_empty() || value.len() > MAX_CHARS {
        return Err("invalid hardware requirement_id".to_owned());
    }
    let mut bytes = value.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid hardware requirement_id".to_owned());
    }
    Ok(())
}

fn parse_merge_evidence_policy(
    document: &PolicyDocument,
) -> Result<Option<MergeEvidenceRule>, String> {
    let mut in_policy = false;
    let mut saw_section = false;
    let mut schema_version = None;
    let mut required = None;
    let mut requirement_id = None;

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_policy {
            if raw_line == "merge_evidence_policy:" {
                if saw_section {
                    return Err(format!(
                        "duplicate merge_evidence_policy section in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
                in_policy = true;
                saw_section = true;
            }
            continue;
        }
        if raw_line.starts_with('\t') {
            return Err(format!(
                "tab indentation is not allowed in merge evidence policy origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            in_policy = false;
            if raw_line == "merge_evidence_policy:" {
                return Err(format!(
                    "duplicate merge_evidence_policy section in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            continue;
        }
        if indent != 2 {
            return Err(format!(
                "merge_evidence_policy only accepts scalar fields at indentation 2 in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let (key, raw_value) = trimmed.split_once(':').ok_or_else(|| {
            format!(
                "malformed merge evidence policy field in origin/{}:{}",
                document.ref_name, document.path
            )
        })?;
        let value = parse_policy_scalar(raw_value.trim(), key)?;
        match key {
            "schema_version" => {
                let parsed = match value.as_str() {
                    "1" => 1_u8,
                    "2" => 2_u8,
                    other => {
                        return Err(format!(
                            "unsupported merge evidence policy schema_version {other} in origin/{}:{}",
                            document.ref_name, document.path
                        ));
                    }
                };
                if schema_version.replace(parsed).is_some() {
                    return Err(format!(
                        "duplicate merge evidence schema_version in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            "required" => {
                let parsed = MergeEvidenceClass::parse(&value)?;
                if required.replace(parsed).is_some() {
                    return Err(format!(
                        "duplicate merge evidence requirement in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            "requirement_id" => {
                validate_hardware_requirement_id(&value)?;
                if requirement_id.replace(value).is_some() {
                    return Err(format!(
                        "duplicate hardware requirement_id in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            other => {
                return Err(format!(
                    "unknown merge evidence policy field {other} in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
        }
    }

    if !saw_section {
        return Ok(None);
    }
    let schema_version = schema_version.ok_or_else(|| {
        format!(
            "merge_evidence_policy requires schema_version in origin/{}:{}",
            document.ref_name, document.path
        )
    })?;
    let required = required.ok_or_else(|| {
        format!(
            "merge_evidence_policy requires required in origin/{}:{}",
            document.ref_name, document.path
        )
    })?;
    match schema_version {
        1 => {
            if requirement_id.is_some() {
                return Err(format!(
                    "merge_evidence_policy schema v1 does not accept requirement_id in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
        }
        2 => {
            if required != MergeEvidenceClass::HardwareRequired {
                return Err(format!(
                    "merge_evidence_policy schema v2 only supports hardware_required in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            if requirement_id.is_none() {
                return Err(format!(
                    "merge_evidence_policy schema v2 hardware_required requires requirement_id in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
        }
        _ => unreachable!(),
    }
    Ok(Some(MergeEvidenceRule {
        schema_version,
        required,
        requirement_id,
        source_ref: document.ref_name.clone(),
        source_path: document.path.clone(),
        source_commit: document.commit_sha.clone(),
        source_blob: document.blob_sha.clone(),
    }))
}

'''
policy = policy[:start] + new_parser + policy[end:]

start = policy.index("    #[test]\n    fn merge_evidence_policy_distinguishes_portable_hardware_and_human()")
end = policy.index("    #[test]\n    fn merge_policy_is_explicit_versioned_and_source_bound()", start)
new_tests = r'''    #[test]
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

'''
policy = policy[:start] + new_tests + policy[end:]
policy_path.write_text(policy)

main_path = Path("src/main.rs")
main = main_path.read_text()
main = replace_once(main, "mod evidence;\nmod health;\n", "mod evidence;\nmod hardware_evidence;\nmod health;\n", "hardware module declaration")

helper_anchor = '''fn pr_head_sha(repository: &str, number: u64) -> Result<String, String> {
    live_pr_identity(repository, number).map(|identity| identity.head_sha)
}

fn handle_pr_attention(
'''
helper = '''fn pr_head_sha(repository: &str, number: u64) -> Result<String, String> {
    live_pr_identity(repository, number).map(|identity| identity.head_sha)
}

fn enforce_merge_evidence_gate(
    config: &RunConfig,
    item: &WorkItem,
    policy_snapshot: &policy::PolicySnapshot,
    head_sha: &str,
    base_sha: &str,
) -> Result<Option<ActionExecution>, ActionFailure> {
    match policy_snapshot
        .merge_evidence_eligibility()
        .classified(state::FailureClass::Validation)?
    {
        policy::MergeEvidenceEligibility::Inherit
        | policy::MergeEvidenceEligibility::PortableCi => Ok(None),
        policy::MergeEvidenceEligibility::HardwareRequired(requirement) => {
            let policy_identity = policy_snapshot.identity_token();
            let request = hardware_evidence::HardwareEvidenceRequest {
                data_root: &config.data_root,
                repository: &item.repository,
                pr_number: item.number,
                head_sha,
                base_sha,
                policy_identity: &policy_identity,
                requirement_id: requirement.requirement_id(),
            };
            match hardware_evidence::verify(&request)
                .classified(state::FailureClass::Validation)?
            {
                hardware_evidence::HardwareEvidenceStatus::Satisfied {
                    evidence_path,
                    hardware_class,
                    device_fingerprint,
                } => {
                    println!(
                        "Authoritative hardware evidence verified for {}#{} requirement={} class={} device={} artifact={}",
                        item.repository,
                        item.number,
                        requirement.requirement_id(),
                        hardware_class,
                        device_fingerprint,
                        evidence_path.display()
                    );
                    Ok(None)
                }
                hardware_evidence::HardwareEvidenceStatus::Deferred(reason) => {
                    let detail = format!(
                        "hardware evidence deferred for {}#{} requirement={} head={} base={}: {reason}",
                        item.repository,
                        item.number,
                        requirement.requirement_id(),
                        head_sha,
                        base_sha
                    );
                    println!("{detail}");
                    Ok(Some(ActionExecution::deferred(detail)))
                }
            }
        }
        policy::MergeEvidenceEligibility::Deferred(denial) => {
            let reason = denial.merge_reason(&item.repository, policy_snapshot);
            println!("Autonomous merge deferred by repository evidence policy: {reason}");
            Ok(Some(ActionExecution::deferred(reason)))
        }
    }
}

fn handle_pr_attention(
'''
main = replace_once(main, helper_anchor, helper, "merge evidence helper anchor")

old_gate = '''    match policy_snapshot
        .merge_evidence_eligibility()
        .classified(state::FailureClass::Validation)?
    {
        policy::MergeEvidenceEligibility::Inherit
        | policy::MergeEvidenceEligibility::PortableCi => {}
        policy::MergeEvidenceEligibility::Deferred(denial) => {
            let reason = denial.merge_reason(&item.repository, &policy_snapshot);
            println!("Autonomous merge deferred by repository evidence policy: {reason}");
            return Ok(ActionExecution::deferred(reason));
        }
    }
'''
new_gate = '''    if let Some(deferred) = enforce_merge_evidence_gate(
        config,
        item,
        &policy_snapshot,
        &metadata.head_sha,
        &validated_base_sha,
    )? {
        return Ok(deferred);
    }
'''
main = replace_once(main, old_gate, new_gate, "initial merge evidence gate")

base_sync_anchor = '''                if !policy_snapshot_is_current(
                    &workspace,
                    &policy_snapshot,
                    "before base-sync push",
                )? {
                    return Ok(ActionExecution::completed(ActionOutcome::Deferred));
                }
                let refspec = format!("HEAD:refs/heads/{}", metadata.head_branch);
'''
base_sync_new = '''                if !policy_snapshot_is_current(
                    &workspace,
                    &policy_snapshot,
                    "before base-sync push",
                )? {
                    return Ok(ActionExecution::completed(ActionOutcome::Deferred));
                }
                if let Some(deferred) = enforce_merge_evidence_gate(
                    config,
                    item,
                    &policy_snapshot,
                    &metadata.head_sha,
                    &validated_base_sha,
                )? {
                    return Ok(deferred);
                }
                let refspec = format!("HEAD:refs/heads/{}", metadata.head_branch);
'''
main = replace_once(main, base_sync_anchor, base_sync_new, "base sync hardware recheck")

draft_anchor = '''    let number = item.number.to_string();
    if item.draft {
        println!(
            "Marking {}#{} ready only after exact-head local validation",
'''
draft_new = '''    let number = item.number.to_string();
    if item.draft {
        if let Some(deferred) = enforce_merge_evidence_gate(
            config,
            item,
            &policy_snapshot,
            &metadata.head_sha,
            &validated_base_sha,
        )? {
            return Ok(deferred);
        }
        println!(
            "Marking {}#{} ready only after exact-head local validation",
'''
main = replace_once(main, draft_anchor, draft_new, "ready hardware recheck")

merge_anchor = '''    if !policy_snapshot_is_current(&workspace, &policy_snapshot, "immediately before merge")? {
        return Ok(ActionExecution::completed(ActionOutcome::Deferred));
    }

    println!(
        "Merging {}#{} at exact validated head {} on unchanged base {}",
'''
merge_new = '''    if !policy_snapshot_is_current(&workspace, &policy_snapshot, "immediately before merge")? {
        return Ok(ActionExecution::completed(ActionOutcome::Deferred));
    }
    if let Some(deferred) = enforce_merge_evidence_gate(
        config,
        item,
        &policy_snapshot,
        &metadata.head_sha,
        &validated_base_sha,
    )? {
        return Ok(deferred);
    }

    println!(
        "Merging {}#{} at exact validated head {} on unchanged base {}",
'''
main = replace_once(main, merge_anchor, merge_new, "final merge hardware recheck")
main_path.write_text(main)

hardware_path = Path("src/hardware_evidence.rs")
hardware = hardware_path.read_text()
hardware = hardware.replace("    started_at: u64,\n    finished_at: u64,\n", "")
hardware = hardware.replace("        .arg(request.head_sha)\n        .output()", "        .arg(request.head_sha)\n        .arg(\"--predicate-type\")\n        .arg(\"https://slsa.dev/provenance/v1\")\n        .output()")
hardware = hardware.replace("    if contents.as_bytes().len() as u64 > max_bytes {", "    if contents.len() as u64 > max_bytes {")
hardware = hardware.replace("        device_fingerprint,\n        started_at,\n        finished_at,\n    })", "        device_fingerprint,\n    })")
hardware = hardware.replace("test \"${{11}}\" = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\\nexit 0\\n", "test \"${{11}}\" = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\\ntest \"${{12}}\" = --predicate-type\\ntest \"${{13}}\" = https://slsa.dev/provenance/v1\\nexit 0\\n")
hardware_path.write_text(hardware)

print("ORCH6a staging transformation applied")
