from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one anchor, got {count}")
    return text.replace(old, new, 1)


# --- typed hardware evidence defer reasons ---
path = Path("src/hardware_evidence.rs")
text = path.read_text()
old = '''#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HardwareEvidenceStatus {
    Satisfied {
        evidence_path: PathBuf,
        hardware_class: String,
        device_fingerprint: String,
    },
    Deferred(String),
}
'''
new = '''#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HardwareEvidenceDeferReason {
    MissingTrust,
    MissingEvidence,
    VerifierUnavailable,
    AttestationNotVerified,
}

impl HardwareEvidenceDeferReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MissingTrust => "missing_trust",
            Self::MissingEvidence => "missing_evidence",
            Self::VerifierUnavailable => "verifier_unavailable",
            Self::AttestationNotVerified => "attestation_not_verified",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HardwareEvidenceDeferred {
    pub(crate) reason: HardwareEvidenceDeferReason,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HardwareEvidenceStatus {
    Satisfied {
        evidence_path: PathBuf,
        hardware_class: String,
        device_fingerprint: String,
    },
    Deferred(HardwareEvidenceDeferred),
}
'''
text = replace_once(text, old, new, "hardware evidence status")

repls = [
('''            return Ok(HardwareEvidenceStatus::Deferred(format!(
                "hardware evidence requirement {} has no local trust root",
                request.requirement_id
            )));''', '''            return Ok(HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                reason: HardwareEvidenceDeferReason::MissingTrust,
                detail: format!(
                    "hardware evidence requirement {} has no local trust root",
                    request.requirement_id
                ),
            }));''', "missing trust defer"),
('''                return Ok(HardwareEvidenceStatus::Deferred(format!(
                    "hardware evidence requirement {} has no evidence for {}#{} head={}",
                    request.requirement_id, request.repository, request.pr_number, request.head_sha
                )));''', '''                return Ok(HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                    reason: HardwareEvidenceDeferReason::MissingEvidence,
                    detail: format!(
                        "hardware evidence requirement {} has no evidence for {}#{} head={}",
                        request.requirement_id,
                        request.repository,
                        request.pr_number,
                        request.head_sha
                    ),
                }));''', "missing evidence defer"),
('''            return Ok(HardwareEvidenceStatus::Deferred(format!(
                "hardware evidence verifier unavailable for requirement {}: {}",
                request.requirement_id,
                bounded_diagnostic(&error.to_string())
            )));''', '''            return Ok(HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                reason: HardwareEvidenceDeferReason::VerifierUnavailable,
                detail: format!(
                    "hardware evidence verifier unavailable for requirement {}: {}",
                    request.requirement_id,
                    bounded_diagnostic(&error.to_string())
                ),
            }));''', "verifier unavailable defer"),
('''        return Ok(HardwareEvidenceStatus::Deferred(format!(
            "hardware evidence attestation did not verify for requirement {}: {}",
            request.requirement_id,
            bounded_diagnostic(&stderr)
        )));''', '''        return Ok(HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
            reason: HardwareEvidenceDeferReason::AttestationNotVerified,
            detail: format!(
                "hardware evidence attestation did not verify for requirement {}: {}",
                request.requirement_id,
                bounded_diagnostic(&stderr)
            ),
        }));''', "attestation defer"),
]
for old, new, label in repls:
    text = replace_once(text, old, new, label)

old_test = '''        assert!(
            matches!(status, HardwareEvidenceStatus::Deferred(reason) if reason.contains("no local trust root"))
        );

        write_trust(&root);
        let status =
            verify_with_program(&req, OsStr::new("definitely-not-a-real-verifier")).unwrap();
        assert!(
            matches!(status, HardwareEvidenceStatus::Deferred(reason) if reason.contains("has no evidence"))
        );'''
new_test = '''        assert!(matches!(
            status,
            HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                reason: HardwareEvidenceDeferReason::MissingTrust,
                detail,
            }) if detail.contains("no local trust root")
        ));

        write_trust(&root);
        let status =
            verify_with_program(&req, OsStr::new("definitely-not-a-real-verifier")).unwrap();
        assert!(matches!(
            status,
            HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                reason: HardwareEvidenceDeferReason::MissingEvidence,
                detail,
            }) if detail.contains("has no evidence")
        ));'''
text = replace_once(text, old_test, new_test, "typed missing tests")
old_test = '''        assert!(
            matches!(status, HardwareEvidenceStatus::Deferred(reason) if reason.contains("did not verify"))
        );'''
new_test = '''        assert!(matches!(
            status,
            HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                reason: HardwareEvidenceDeferReason::AttestationNotVerified,
                detail,
            }) if detail.contains("did not verify")
        ));'''
text = replace_once(text, old_test, new_test, "typed verifier test")

# Add explicit verifier-unavailable typed test after failed verifier test.
insert = '''

    #[test]
    fn typed_verifier_unavailable_reason_is_not_missing_evidence() {
        let root = temp_root("verifier-unavailable");
        write_trust(&root);
        write_evidence(&root);
        let status = verify_with_program(
            &request(&root),
            OsStr::new("definitely-not-a-real-verifier"),
        )
        .unwrap();
        assert!(matches!(
            status,
            HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                reason: HardwareEvidenceDeferReason::VerifierUnavailable,
                ..
            })
        ));
        fs::remove_dir_all(root).unwrap();
    }
'''
anchor = '''    #[cfg(unix)]
    #[test]
    fn failed_cryptographic_verifier_never_satisfies_evidence() {'''
pos = text.index(anchor)
# Put typed verifier-unavailable test before cfg unix failed verifier test.
text = text[:pos] + insert + text[pos:]
path.write_text(text)


# --- harden and complete dispatch module ---
path = Path("src/hardware_dispatch.rs")
text = path.read_text()
text = replace_once(
    text,
    'use std::process::Command;\n',
    'use std::process::{Command, Stdio};\n',
    "dispatch process import",
)

old = '''pub(crate) fn ensure_dispatched(
    request: &HardwareEvidenceRequest<'_>,
    dispatch_token: &str,
) -> Result<HardwareDispatchOutcome, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_secs();
    ensure_dispatched_with_program(request, dispatch_token, now, OsStr::new("gh"))
}
'''
new = '''pub(crate) fn binding_token(request: &HardwareEvidenceRequest<'_>) -> Result<String, String> {
    binding_token_with_program(request, OsStr::new("git"))
}

fn binding_token_with_program(
    request: &HardwareEvidenceRequest<'_>,
    program: &OsStr,
) -> Result<String, String> {
    validate_request(request)?;
    let payload = format!(
        "hardware-dispatch-binding-v1\\nrepository-hex={}\\npr-number={}\\nhead-sha={}\\nbase-sha={}\\npolicy-identity={}\\nrequirement-id={}\\n",
        hex_component(request.repository),
        request.pr_number,
        request.head_sha,
        request.base_sha,
        request.policy_identity,
        request.requirement_id,
    );
    let mut child = Command::new(program)
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start hardware dispatch binding hasher: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "failed to open hardware dispatch binding hasher stdin".to_owned())?
        .write_all(payload.as_bytes())
        .map_err(|error| format!("failed to write hardware dispatch binding payload: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to wait for hardware dispatch binding hasher: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "hardware dispatch binding hasher failed: {}",
            bounded_diagnostic(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    let token = String::from_utf8(output.stdout)
        .map_err(|error| format!("hardware dispatch binding hash is not UTF-8: {error}"))?;
    let token = token.trim().to_owned();
    validate_git_digest("dispatch token", &token)?;
    Ok(token)
}

pub(crate) fn ensure_dispatched(
    request: &HardwareEvidenceRequest<'_>,
) -> Result<HardwareDispatchOutcome, String> {
    let dispatch_token = binding_token(request)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_secs();
    ensure_dispatched_with_program(request, &dispatch_token, now, OsStr::new("gh"))
}
'''
text = replace_once(text, old, new, "dispatch public API and binding token")

start = text.index("fn ensure_managed_directory(")
end = text.index("fn read_regular_bounded(", start)
new_dir = r'''fn ensure_managed_directory(
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
            "hardware dispatch state path is outside orchestrator data root: {}",
            directory.display()
        ));
    }

    if state_root.exists() {
        let metadata = fs::symlink_metadata(state_root).map_err(|error| {
            format!(
                "failed to inspect hardware dispatch state root {}: {error}",
                state_root.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "hardware dispatch state root must be a non-symlink directory: {}",
                state_root.display()
            ));
        }
    } else {
        let parent = state_root
            .parent()
            .ok_or_else(|| "hardware dispatch state root has no parent".to_owned())?;
        let canonical_parent = fs::canonicalize(parent).map_err(|error| {
            format!(
                "failed to canonicalize hardware dispatch state parent {}: {error}",
                parent.display()
            )
        })?;
        if !canonical_parent.starts_with(&canonical_data) {
            return Err("hardware dispatch state parent escapes orchestrator data root".to_owned());
        }
        fs::create_dir(state_root).map_err(|error| {
            format!(
                "failed to create hardware dispatch state root {}: {error}",
                state_root.display()
            )
        })?;
    }

    let canonical_root = fs::canonicalize(state_root).map_err(|error| {
        format!(
            "failed to canonicalize hardware dispatch state root {}: {error}",
            state_root.display()
        )
    })?;
    if !canonical_root.starts_with(&canonical_data) {
        return Err("hardware dispatch state root escapes orchestrator data root".to_owned());
    }

    let relative = directory.strip_prefix(state_root).map_err(|_| {
        format!(
            "hardware dispatch directory is outside state root: {}",
            directory.display()
        )
    })?;
    let mut current = state_root.to_path_buf();
    for component in relative.components() {
        use std::path::Component;
        let Component::Normal(component) = component else {
            return Err("hardware dispatch directory contains a non-normal component".to_owned());
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "hardware dispatch directory component must be a non-symlink directory: {}",
                        current.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    format!(
                        "failed to create hardware dispatch directory component {}: {error}",
                        current.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect hardware dispatch directory component {}: {error}",
                    current.display()
                ));
            }
        }
        let canonical_current = fs::canonicalize(&current).map_err(|error| {
            format!(
                "failed to canonicalize hardware dispatch directory component {}: {error}",
                current.display()
            )
        })?;
        if !canonical_current.starts_with(&canonical_root) {
            return Err(format!(
                "hardware dispatch directory component escapes managed root: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

'''
text = text[:start] + new_dir + text[end:]

# Add token sensitivity and symlink hardening tests before final changed-binding test.
anchor = '''    #[test]
    fn changed_binding_or_config_cannot_reuse_existing_dispatch_state() {'''
insert = r'''    #[test]
    fn binding_token_changes_with_exact_candidate_identity() {
        let root = temp_root("binding-token");
        let first = binding_token(&request(&root)).unwrap();
        let changed = HardwareEvidenceRequest {
            data_root: &root,
            repository: "Memorithm/Test",
            pr_number: 51,
            head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base_sha: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            policy_identity: "abcd1234",
            requirement_id: "jetson-thor-real-device",
        };
        let second = binding_token(&changed).unwrap();
        assert_ne!(first, second);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_state_component_fails_closed_before_dispatch() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = temp_root("symlink-state");
        write_config(&root);
        let outside = temp_root("symlink-outside");
        let state_root = root.join("state/hardware-dispatch");
        fs::create_dir_all(&state_root).unwrap();
        symlink(&outside, state_root.join(hex_component("Memorithm/Test"))).unwrap();

        let marker = root.join("must-not-run");
        let fake = root.join("fake-gh");
        fs::write(
            &fake,
            format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake, permissions).unwrap();
        let error = ensure_dispatched_with_program(
            &request(&root),
            "cccccccccccccccccccccccccccccccccccccccc",
            100,
            fake.as_os_str(),
        )
        .unwrap_err();
        assert!(error.contains("non-symlink directory"));
        assert!(!marker.exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

'''
text = text.replace(anchor, insert + anchor, 1)
path.write_text(text)


# --- merge gate integration: only typed missing_evidence may dispatch ---
path = Path("src/main.rs")
text = path.read_text()
text = replace_once(
    text,
    "mod evidence;\nmod hardware_evidence;\n",
    "mod evidence;\nmod hardware_dispatch;\nmod hardware_evidence;\n",
    "dispatch module declaration",
)
old = '''                hardware_evidence::HardwareEvidenceStatus::Deferred(reason) => {
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
'''
new = '''                hardware_evidence::HardwareEvidenceStatus::Deferred(deferred) => {
                    let mut dispatch_detail = String::new();
                    if deferred.reason
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
                    let detail = format!(
                        "hardware evidence deferred for {}#{} requirement={} head={} base={} reason={}: {}{}",
                        item.repository,
                        item.number,
                        requirement.requirement_id(),
                        head_sha,
                        base_sha,
                        deferred.reason.as_str(),
                        deferred.detail,
                        dispatch_detail
                    );
                    println!("{detail}");
                    Ok(Some(ActionExecution::deferred(detail)))
                }
'''
text = replace_once(text, old, new, "missing evidence dispatch integration")
path.write_text(text)

print("ORCH6b integration transform applied")
