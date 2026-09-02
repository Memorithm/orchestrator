from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, got {count}")
    return text.replace(old, new, 1)


# Restrict caller-supplied hardware candidate verification to the dedicated ingest root.
path = Path("src/hardware_evidence.rs")
text = path.read_text()
text = replace_once(
    text,
    '''    let candidate_status =
        verify_path_with_program(request, request.data_root, candidate_path, verifier)?;
''',
    '''    let ingest_root = request.data_root.join("state/hardware-ingest");
    let candidate_status =
        verify_path_with_program(request, &ingest_root, candidate_path, verifier)?;
''',
    "candidate managed root",
)
anchor = '''    #[cfg(unix)]
    #[test]
    fn verified_candidate_is_promoted_without_clobber_and_reverified() {'''
insert = '''    #[cfg(unix)]
    #[test]
    fn candidate_outside_hardware_ingest_root_fails_closed() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("candidate-outside-ingest");
        write_trust(&root);
        fs::create_dir_all(root.join("state/hardware-ingest")).unwrap();
        let outside_dir = root.join("state/other");
        fs::create_dir_all(&outside_dir).unwrap();
        let outside = outside_dir.join("hardware.evidence");
        fs::write(
            &outside,
            "v1\\nrepository=Memorithm/Test\\npr_number=49\\nhead_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\\nbase_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\\npolicy_identity=abcd1234\\nrequirement_id=jetson-thor-real-device\\nresult=passed\\nhardware_class=jetson-thor\\ndevice_fingerprint=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\\nstarted_at=100\\nfinished_at=101\\n",
        )
        .unwrap();
        let verifier = root.join("must-not-run");
        fs::write(&verifier, "#!/bin/sh\\nexit 99\\n").unwrap();
        let mut permissions = fs::metadata(&verifier).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&verifier, permissions).unwrap();

        let error = promote_candidate_with_program(&request(&root), &outside, verifier.as_os_str())
            .unwrap_err();
        assert!(error.contains("escapes managed root"));
        assert!(!evidence_path(&root).exists());
        fs::remove_dir_all(root).unwrap();
    }

'''
if text.count(anchor) != 1:
    raise SystemExit("candidate boundary test anchor missing")
text = text.replace(anchor, insert + anchor, 1)
path.write_text(text)


# Record a digest of the downloaded candidate plus a distinct download-complete timestamp.
path = Path("src/hardware_ingest.rs")
text = path.read_text()
text = replace_once(
    text,
    '''    artifact_size_bytes: u64,
    discovered_at: u64,
    finished_at: u64,
    phase: IngestPhase,
''',
    '''    artifact_size_bytes: u64,
    candidate_digest: String,
    discovered_at: u64,
    downloaded_at: u64,
    finished_at: u64,
    phase: IngestPhase,
''',
    "ingest record provenance fields",
)
text = replace_once(
    text,
    '''        artifact_size_bytes: candidate.size_bytes,
        discovered_at,
        finished_at: discovered_at,
        phase: IngestPhase::Deferred,
''',
    '''        artifact_size_bytes: candidate.size_bytes,
        candidate_digest: "none".to_owned(),
        discovered_at,
        downloaded_at: 0,
        finished_at: discovered_at,
        phase: IngestPhase::Deferred,
''',
    "initial ingest record",
)
text = replace_once(
    text,
    '''    let promoted = match hardware_evidence::promote_candidate_with_program(
        request,
        &candidate_path,
        gh_program,
    ) {
''',
    '''    let downloaded_at = unix_timestamp()?;
    let mut candidate_record = expected.clone();
    candidate_record.candidate_digest = candidate_digest(&candidate_path)?;
    candidate_record.downloaded_at = downloaded_at;

    let promoted = match hardware_evidence::promote_candidate_with_program(
        request,
        &candidate_path,
        gh_program,
    ) {
''',
    "candidate provenance before verification",
)
text = replace_once(
    text,
    '''        Err(error) => {
            let mut record = expected.clone();
            record.finished_at = unix_timestamp()?;
            record.phase = IngestPhase::Rejected;
            atomic_replace(&state_path, &serialize_record(&record))?;
            remove_managed_tree(request.data_root, &state_root, &attempt)?;
            return Err(error);
        }
    };

    let outcome = match promoted {
        HardwareEvidenceStatus::Satisfied { evidence_path, .. } => {
            let mut record = expected;
''',
    '''        Err(error) => {
            let mut record = candidate_record.clone();
            record.finished_at = unix_timestamp()?;
            record.phase = IngestPhase::Rejected;
            atomic_replace(&state_path, &serialize_record(&record))?;
            remove_managed_tree(request.data_root, &state_root, &attempt)?;
            return Err(error);
        }
    };

    let outcome = match promoted {
        HardwareEvidenceStatus::Satisfied { evidence_path, .. } => {
            let mut record = candidate_record.clone();
''',
    "candidate record on verifier error/import",
)
text = replace_once(
    text,
    '''        HardwareEvidenceStatus::Deferred(deferred) => {
            let mut record = expected;
''',
    '''        HardwareEvidenceStatus::Deferred(deferred) => {
            let mut record = candidate_record;
''',
    "candidate record on deferred verification",
)
text = replace_once(
    text,
    '''        "{STATE_VERSION}\\nrepository={}\\npr_number={}\\nhead_sha={}\\nbase_sha={}\\npolicy_identity={}\\nrequirement_id={}\\ndispatch_token={}\\ndispatch_repository={}\\ndispatch_workflow={}\\ndispatch_ref={}\\nartifact_id={}\\nrun_id={}\\nartifact_name={}\\nartifact_size_bytes={}\\ndiscovered_at={}\\nfinished_at={}\\nstatus={}\\n",
''',
    '''        "{STATE_VERSION}\\nrepository={}\\npr_number={}\\nhead_sha={}\\nbase_sha={}\\npolicy_identity={}\\nrequirement_id={}\\ndispatch_token={}\\ndispatch_repository={}\\ndispatch_workflow={}\\ndispatch_ref={}\\nartifact_id={}\\nrun_id={}\\nartifact_name={}\\nartifact_size_bytes={}\\ncandidate_digest={}\\ndiscovered_at={}\\ndownloaded_at={}\\nfinished_at={}\\nstatus={}\\n",
''',
    "serialize format",
)
text = replace_once(
    text,
    '''        record.artifact_name,
        record.artifact_size_bytes,
        record.discovered_at,
        record.finished_at,
        record.phase.as_str(),
''',
    '''        record.artifact_name,
        record.artifact_size_bytes,
        record.candidate_digest,
        record.discovered_at,
        record.downloaded_at,
        record.finished_at,
        record.phase.as_str(),
''',
    "serialize arguments",
)
text = replace_once(
    text,
    '''        "artifact_size_bytes",
        "discovered_at",
        "finished_at",
        "status",
''',
    '''        "artifact_size_bytes",
        "candidate_digest",
        "discovered_at",
        "downloaded_at",
        "finished_at",
        "status",
''',
    "parse allowed fields",
)
text = replace_once(
    text,
    '''    let discovered_at = parse_nonzero_u64("discovered_at", required(&fields, "discovered_at")?)?;
    let finished_at = parse_nonzero_u64("finished_at", required(&fields, "finished_at")?)?;
    if finished_at < discovered_at {
        return Err("hardware ingest state finished_at precedes discovered_at".to_owned());
    }
''',
    '''    let discovered_at = parse_nonzero_u64("discovered_at", required(&fields, "discovered_at")?)?;
    let downloaded_at = parse_u64("downloaded_at", required(&fields, "downloaded_at")?)?;
    let finished_at = parse_nonzero_u64("finished_at", required(&fields, "finished_at")?)?;
    if finished_at < discovered_at
        || (downloaded_at != 0 && (downloaded_at < discovered_at || downloaded_at > finished_at))
    {
        return Err("hardware ingest state timestamps are not ordered".to_owned());
    }
    let candidate_digest = required(&fields, "candidate_digest")?.to_owned();
    validate_candidate_digest(&candidate_digest)?;
''',
    "parse timestamps and digest",
)
text = replace_once(
    text,
    '''        artifact_size_bytes: parse_nonzero_u64(
            "artifact_size_bytes",
            required(&fields, "artifact_size_bytes")?,
        )?,
        discovered_at,
        finished_at,
        phase: IngestPhase::parse(required(&fields, "status")?)?,
''',
    '''        artifact_size_bytes: parse_nonzero_u64(
            "artifact_size_bytes",
            required(&fields, "artifact_size_bytes")?,
        )?,
        candidate_digest,
        discovered_at,
        downloaded_at,
        finished_at,
        phase: IngestPhase::parse(required(&fields, "status")?)?,
''',
    "parsed provenance fields",
)
# Add helpers before parse_nonzero_u64.
anchor = '''fn parse_nonzero_u64(label: &str, value: &str) -> Result<u64, String> {'''
helpers = '''fn candidate_digest(path: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("hash-object")
        .arg(path)
        .output()
        .map_err(|error| format!("failed to hash hardware evidence candidate: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "hardware evidence candidate hash failed: {}",
            bounded_diagnostic(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    let digest = String::from_utf8(output.stdout)
        .map_err(|error| format!("hardware evidence candidate hash is not UTF-8: {error}"))?;
    let digest = digest.trim().to_owned();
    validate_git_digest("candidate digest", &digest)?;
    Ok(digest)
}

fn validate_candidate_digest(value: &str) -> Result<(), String> {
    if value == "none" {
        return Ok(());
    }
    validate_git_digest("candidate digest", value)
}

fn parse_u64(label: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|error| format!("invalid hardware ingest {label} {value:?}: {error}"))
}

'''
if text.count(anchor) != 1:
    raise SystemExit("digest helper anchor missing")
text = text.replace(anchor, helpers + anchor, 1)
# End-to-end test must prove the durable record contains concrete candidate provenance.
anchor = '''        let actions = fs::read_to_string(&marker).unwrap();
        assert_eq!(
            actions.lines().collect::<Vec<_>>(),
            ["api", "run", "attestation", "attestation"]
        );
'''
replacement = '''        let actions = fs::read_to_string(&marker).unwrap();
        assert_eq!(
            actions.lines().collect::<Vec<_>>(),
            ["api", "run", "attestation", "attestation"]
        );
        let state_path = root
            .join("state/hardware-ingest")
            .join(hex_component("Memorithm/Test"))
            .join("pr-53")
            .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .join("jetson-thor-real-device")
            .join(&token)
            .join("artifact-7.state");
        let state = fs::read_to_string(state_path).unwrap();
        let digest = state
            .lines()
            .find_map(|line| line.strip_prefix("candidate_digest="))
            .unwrap();
        assert_ne!(digest, "none");
        assert!(matches!(digest.len(), 40 | 64));
        let downloaded_at = state
            .lines()
            .find_map(|line| line.strip_prefix("downloaded_at="))
            .unwrap()
            .parse::<u64>()
            .unwrap();
        assert!(downloaded_at > 0);
'''
text = replace_once(text, anchor, replacement, "end-to-end provenance assertions")
path.write_text(text)
print("ORCH6c hardening applied")
