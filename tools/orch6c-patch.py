from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one anchor, got {count}")
    return text.replace(old, new, 1)


# ---- hardware_dispatch: expose only the already trusted local source ----
path = Path("src/hardware_dispatch.rs")
text = path.read_text()
old = '''#[derive(Debug, Clone, PartialEq, Eq)]
struct DispatchConfig {
    repository: String,
    workflow: String,
    ref_name: String,
}
'''
new = '''#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HardwareDispatchSource {
    pub(crate) repository: String,
    pub(crate) workflow: String,
    pub(crate) ref_name: String,
}

type DispatchConfig = HardwareDispatchSource;
'''
text = replace_once(text, old, new, "dispatch source type")
anchor = '''pub(crate) fn binding_token(request: &HardwareEvidenceRequest<'_>) -> Result<String, String> {
    binding_token_with_program(request, OsStr::new("git"))
}
'''
insert = anchor + '''
pub(crate) fn discovery_source(
    request: &HardwareEvidenceRequest<'_>,
) -> Result<Option<HardwareDispatchSource>, String> {
    validate_request(request)?;
    let config_root = request.data_root.join("config/hardware-dispatch");
    let config_path = config_root.join(format!("{}.state", request.requirement_id));
    let Some(contents) = read_regular_bounded(
        request.data_root,
        &config_root,
        &config_path,
        MAX_CONFIG_BYTES,
        "hardware dispatch config",
    )? else {
        return Ok(None);
    };
    parse_config(&contents).map(Some)
}
'''
text = replace_once(text, anchor, insert, "dispatch discovery source function")
path.write_text(text)


# ---- hardware_evidence: factor exact candidate verification and no-clobber promotion ----
path = Path("src/hardware_evidence.rs")
text = path.read_text()
text = replace_once(
    text,
    "use std::fs;\nuse std::path::{Path, PathBuf};\nuse std::process::Command;\n",
    "use std::fs::{self, OpenOptions};\nuse std::io::Write;\nuse std::path::{Path, PathBuf};\nuse std::process::Command;\nuse std::time::{SystemTime, UNIX_EPOCH};\n",
    "hardware evidence imports",
)
start = text.index("pub(crate) fn verify(\n")
end = text.index("fn validate_request(", start)
block = r'''pub(crate) fn verify(
    request: &HardwareEvidenceRequest<'_>,
) -> Result<HardwareEvidenceStatus, String> {
    verify_with_program(request, OsStr::new("gh"))
}

fn verify_with_program(
    request: &HardwareEvidenceRequest<'_>,
    verifier: &OsStr,
) -> Result<HardwareEvidenceStatus, String> {
    validate_request(request)?;
    let evidence_root = request.data_root.join("state/hardware-evidence");
    let evidence_path = canonical_evidence_path(request)?;
    verify_path_with_program(request, &evidence_root, &evidence_path, verifier)
}

pub(crate) fn promote_candidate_with_program(
    request: &HardwareEvidenceRequest<'_>,
    candidate_path: &Path,
    verifier: &OsStr,
) -> Result<HardwareEvidenceStatus, String> {
    validate_request(request)?;
    let candidate_status = verify_path_with_program(
        request,
        request.data_root,
        candidate_path,
        verifier,
    )?;
    let HardwareEvidenceStatus::Satisfied { .. } = candidate_status else {
        return Ok(candidate_status);
    };

    let canonical_path = canonical_evidence_path(request)?;
    if fs::symlink_metadata(&canonical_path).is_ok() {
        return verify_with_program(request, verifier);
    }
    let contents = read_regular_bounded(request.data_root, candidate_path, MAX_EVIDENCE_BYTES)?
        .ok_or_else(|| "verified hardware evidence candidate disappeared before promotion".to_owned())?;
    let evidence_root = request.data_root.join("state/hardware-evidence");
    let parent = canonical_path
        .parent()
        .ok_or_else(|| "canonical hardware evidence path has no parent".to_owned())?;
    ensure_managed_directory(request.data_root, &evidence_root, parent)?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_nanos();
    let temporary = parent.join(format!(
        ".hardware-evidence.{}.{}.tmp",
        std::process::id(),
        stamp
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| {
                format!(
                    "failed to create hardware evidence promotion transaction {}: {error}",
                    temporary.display()
                )
            })?;
        file.write_all(contents.as_bytes()).map_err(|error| {
            format!(
                "failed to write hardware evidence promotion transaction {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync hardware evidence promotion transaction {}: {error}",
                temporary.display()
            )
        })?;
        match fs::hard_link(&temporary, &canonical_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(format!(
                "failed to publish verified hardware evidence without clobbering {}: {error}",
                canonical_path.display()
            )),
        }
    })();
    let _ = fs::remove_file(&temporary);
    result?;

    verify_with_program(request, verifier)
}

fn canonical_evidence_path(request: &HardwareEvidenceRequest<'_>) -> Result<PathBuf, String> {
    validate_request(request)?;
    Ok(request
        .data_root
        .join("state/hardware-evidence")
        .join(hex_component(request.repository))
        .join(format!("pr-{}", request.pr_number))
        .join(request.head_sha)
        .join(format!("{}.evidence", request.requirement_id)))
}

fn load_trust(request: &HardwareEvidenceRequest<'_>) -> Result<Option<HardwareTrust>, String> {
    let trust_root = request.data_root.join("config/hardware-trust");
    let trust_path = trust_root.join(format!("{}.state", request.requirement_id));
    let Some(contents) = read_regular_bounded(&trust_root, &trust_path, MAX_TRUST_BYTES)? else {
        return Ok(None);
    };
    parse_trust(&contents).map(Some)
}

fn verify_path_with_program(
    request: &HardwareEvidenceRequest<'_>,
    managed_root: &Path,
    evidence_path: &Path,
    verifier: &OsStr,
) -> Result<HardwareEvidenceStatus, String> {
    let Some(trust) = load_trust(request)? else {
        return Ok(HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
            reason: HardwareEvidenceDeferReason::MissingTrust,
            detail: format!(
                "hardware evidence requirement {} has no local trust root",
                request.requirement_id
            ),
        }));
    };
    let evidence_contents = match read_regular_bounded(managed_root, evidence_path, MAX_EVIDENCE_BYTES)? {
        Some(contents) => contents,
        None => {
            return Ok(HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                reason: HardwareEvidenceDeferReason::MissingEvidence,
                detail: format!(
                    "hardware evidence requirement {} has no evidence for {}#{} head={}",
                    request.requirement_id,
                    request.repository,
                    request.pr_number,
                    request.head_sha
                ),
            }));
        }
    };
    let manifest = parse_manifest(&evidence_contents)?;
    validate_manifest_binding(request, &manifest)?;

    let output = match Command::new(verifier)
        .arg("attestation")
        .arg("verify")
        .arg(evidence_path)
        .arg("--repo")
        .arg(request.repository)
        .arg("--signer-workflow")
        .arg(&trust.signer_workflow)
        .arg("--signer-digest")
        .arg(&trust.signer_digest)
        .arg("--source-digest")
        .arg(request.head_sha)
        .arg("--predicate-type")
        .arg("https://slsa.dev/provenance/v1")
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return Ok(HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                reason: HardwareEvidenceDeferReason::VerifierUnavailable,
                detail: format!(
                    "hardware evidence verifier unavailable for requirement {}: {}",
                    request.requirement_id,
                    bounded_diagnostic(&error.to_string())
                ),
            }));
        }
    };
    if !output.status.success() {
        return Ok(HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
            reason: HardwareEvidenceDeferReason::AttestationNotVerified,
            detail: format!(
                "hardware evidence attestation did not verify for requirement {}: {}",
                request.requirement_id,
                bounded_diagnostic(&String::from_utf8_lossy(&output.stderr))
            ),
        }));
    }
    Ok(HardwareEvidenceStatus::Satisfied {
        evidence_path: evidence_path.to_path_buf(),
        hardware_class: manifest.hardware_class,
        device_fingerprint: manifest.device_fingerprint,
    })
}

fn ensure_managed_directory(
    data_root: &Path,
    state_root: &Path,
    directory: &Path,
) -> Result<(), String> {
    let canonical_data = fs::canonicalize(data_root).map_err(|error| {
        format!(
            "failed to canonicalize orchestrator data root {}: {error}",
            data_root.display()
        )
    })?;
    if !state_root.starts_with(data_root) || !directory.starts_with(state_root) {
        return Err(format!(
            "hardware evidence promotion path is outside orchestrator data root: {}",
            directory.display()
        ));
    }
    let relative = directory.strip_prefix(data_root).map_err(|_| {
        "hardware evidence promotion directory is outside orchestrator data root".to_owned()
    })?;
    let mut current = data_root.to_path_buf();
    for component in relative.components() {
        use std::path::Component;
        let Component::Normal(component) = component else {
            return Err("hardware evidence promotion path contains a non-normal component".to_owned());
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "hardware evidence promotion directory component must be a non-symlink directory: {}",
                        current.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    format!(
                        "failed to create hardware evidence promotion directory {}: {error}",
                        current.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect hardware evidence promotion directory {}: {error}",
                    current.display()
                ));
            }
        }
        let canonical_current = fs::canonicalize(&current).map_err(|error| {
            format!(
                "failed to canonicalize hardware evidence promotion directory {}: {error}",
                current.display()
            )
        })?;
        if !canonical_current.starts_with(&canonical_data) {
            return Err(format!(
                "hardware evidence promotion directory escapes data root: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

'''
text = text[:start] + block + text[end:]

# Add promotion test before existing failed verifier test.
anchor = '''    #[cfg(unix)]
    #[test]
    fn failed_cryptographic_verifier_never_satisfies_evidence() {'''
insert = r'''    #[cfg(unix)]
    #[test]
    fn verified_candidate_is_promoted_without_clobber_and_reverified() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("promote-candidate");
        write_trust(&root);
        let candidate_dir = root.join("state/hardware-ingest/candidate");
        fs::create_dir_all(&candidate_dir).unwrap();
        let candidate = candidate_dir.join("hardware.evidence");
        let canonical = evidence_path(&root);
        let canonical_parent = canonical.parent().unwrap();
        let source = "v1\nrepository=Memorithm/Test\npr_number=49\nhead_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbase_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\npolicy_identity=abcd1234\nrequirement_id=jetson-thor-real-device\nresult=passed\nhardware_class=jetson-thor\ndevice_fingerprint=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\nstarted_at=100\nfinished_at=101\n";
        fs::write(&candidate, source).unwrap();
        let verifier = root.join("fake-gh-promote");
        let marker = root.join("verify-count");
        fs::write(
            &verifier,
            format!(
                "#!/bin/sh\nset -eu\ntest \"$1\" = attestation\ntest \"$2\" = verify\ntest \"$4\" = --repo\ntest \"$5\" = Memorithm/Test\ntest \"$6\" = --signer-workflow\ntest \"$8\" = --signer-digest\ntest \"${{10}}\" = --source-digest\ntest \"${{11}}\" = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\ntest \"${{12}}\" = --predicate-type\ntest \"${{13}}\" = https://slsa.dev/provenance/v1\necho verify >> '{}'\nexit 0\n",
                marker.display()
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&verifier).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&verifier, permissions).unwrap();

        let status = promote_candidate_with_program(
            &request(&root),
            &candidate,
            verifier.as_os_str(),
        )
        .unwrap();
        assert!(matches!(status, HardwareEvidenceStatus::Satisfied { .. }));
        assert_eq!(fs::read_to_string(&canonical).unwrap(), source);
        assert_eq!(fs::read_to_string(&marker).unwrap().lines().count(), 2);
        assert!(canonical_parent.is_dir());

        fs::write(&candidate, source.replace("hardware_class=jetson-thor", "hardware_class=other")).unwrap();
        let status = promote_candidate_with_program(
            &request(&root),
            &candidate,
            verifier.as_os_str(),
        )
        .unwrap();
        assert!(matches!(status, HardwareEvidenceStatus::Satisfied { .. }));
        assert_eq!(fs::read_to_string(&canonical).unwrap(), source);
        fs::remove_dir_all(root).unwrap();
    }

'''
if text.count(anchor) != 1:
    raise SystemExit("promotion test anchor mismatch")
text = text.replace(anchor, insert + anchor, 1)
path.write_text(text)


# ---- main: ingest only after exact missing-evidence dispatch transaction ----
path = Path("src/main.rs")
text = path.read_text()
text = replace_once(
    text,
    "mod hardware_dispatch;\nmod hardware_evidence;\n",
    "mod hardware_dispatch;\nmod hardware_evidence;\nmod hardware_ingest;\n",
    "hardware ingest module declaration",
)
old = '''                    if deferred.reason
                        == hardware_evidence::HardwareEvidenceDeferReason::MissingEvidence
                    {
                        match hardware_dispatch::ensure_dispatched(&request)
                            .classified(state::FailureClass::Validation)?
                        {
                            hardware_dispatch::HardwareDispatchOutcome::Dispatched { token } => {
                                dispatch_detail = format!(
                                    "; hardware evidence workflow dispatched for exact binding token={token}; dispatch is not evidence"
                                );
                            }
                            hardware_dispatch::HardwareDispatchOutcome::AlreadyDispatched {
                                token,
                            } => {
                                dispatch_detail = format!(
                                    "; exact hardware evidence workflow was already dispatched token={token}; waiting for authoritative evidence"
                                );
                            }
                            hardware_dispatch::HardwareDispatchOutcome::Deferred(reason) => {
                                dispatch_detail = format!("; dispatch deferred: {reason}");
                            }
                        }
                    }
'''
new = '''                    if deferred.reason
                        == hardware_evidence::HardwareEvidenceDeferReason::MissingEvidence
                    {
                        match hardware_dispatch::ensure_dispatched(&request)
                            .classified(state::FailureClass::Validation)?
                        {
                            hardware_dispatch::HardwareDispatchOutcome::Dispatched { token } => {
                                dispatch_detail = format!(
                                    "; hardware evidence workflow dispatched for exact binding token={token}; dispatch is not evidence"
                                );
                                match hardware_ingest::discover_and_ingest(&request, &token)
                                    .classified(state::FailureClass::Validation)?
                                {
                                    hardware_ingest::HardwareIngestOutcome::Imported {
                                        evidence_path,
                                        artifact_id,
                                        run_id,
                                    } => {
                                        println!(
                                            "Authoritative remote hardware evidence imported and canonically reverified for {}#{} requirement={} artifact_id={} run_id={} path={}",
                                            item.repository,
                                            item.number,
                                            requirement.requirement_id(),
                                            artifact_id,
                                            run_id,
                                            evidence_path.display()
                                        );
                                        return Ok(None);
                                    }
                                    hardware_ingest::HardwareIngestOutcome::Deferred(reason) => {
                                        dispatch_detail.push_str(&format!(
                                            "; remote evidence ingestion deferred: {reason}"
                                        ));
                                    }
                                }
                            }
                            hardware_dispatch::HardwareDispatchOutcome::AlreadyDispatched {
                                token,
                            } => {
                                dispatch_detail = format!(
                                    "; exact hardware evidence workflow was already dispatched token={token}; dispatch is not evidence"
                                );
                                match hardware_ingest::discover_and_ingest(&request, &token)
                                    .classified(state::FailureClass::Validation)?
                                {
                                    hardware_ingest::HardwareIngestOutcome::Imported {
                                        evidence_path,
                                        artifact_id,
                                        run_id,
                                    } => {
                                        println!(
                                            "Authoritative remote hardware evidence imported and canonically reverified for {}#{} requirement={} artifact_id={} run_id={} path={}",
                                            item.repository,
                                            item.number,
                                            requirement.requirement_id(),
                                            artifact_id,
                                            run_id,
                                            evidence_path.display()
                                        );
                                        return Ok(None);
                                    }
                                    hardware_ingest::HardwareIngestOutcome::Deferred(reason) => {
                                        dispatch_detail.push_str(&format!(
                                            "; remote evidence ingestion deferred: {reason}"
                                        ));
                                    }
                                }
                            }
                            hardware_dispatch::HardwareDispatchOutcome::Deferred(reason) => {
                                dispatch_detail = format!("; dispatch deferred: {reason}");
                            }
                        }
                    }
'''
text = replace_once(text, old, new, "hardware ingest merge integration")
path.write_text(text)
print("ORCH6c integration transform applied")
