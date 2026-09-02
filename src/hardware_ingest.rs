use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::hardware_dispatch::{self, HardwareDispatchSource};
use crate::hardware_evidence::{
    self, HardwareEvidenceDeferReason, HardwareEvidenceRequest, HardwareEvidenceStatus,
};

const STATE_VERSION: &str = "v1";
const MAX_STATE_BYTES: u64 = 96 * 1024;
const MAX_METADATA_BYTES: usize = 32 * 1024;
const MAX_METADATA_LINES: usize = 10;
const MAX_REMOTE_ARTIFACT_BYTES: u64 = 1024 * 1024;
const MAX_CANDIDATE_BYTES: u64 = 128 * 1024;
const CANDIDATE_FILENAME: &str = "hardware.evidence";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HardwareIngestOutcome {
    Imported {
        evidence_path: PathBuf,
        artifact_id: u64,
        run_id: u64,
    },
    Deferred(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtifactCandidate {
    artifact_id: u64,
    run_id: u64,
    name: String,
    size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IngestPhase {
    Deferred,
    Rejected,
    VerifiedImported,
}

impl IngestPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Deferred => "deferred",
            Self::Rejected => "rejected",
            Self::VerifiedImported => "verified_imported",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "deferred" => Ok(Self::Deferred),
            "rejected" => Ok(Self::Rejected),
            "verified_imported" => Ok(Self::VerifiedImported),
            other => Err(format!("unknown hardware ingest status: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IngestRecord {
    repository: String,
    pr_number: u64,
    head_sha: String,
    base_sha: String,
    policy_identity: String,
    requirement_id: String,
    dispatch_token: String,
    dispatch_repository: String,
    dispatch_workflow: String,
    dispatch_ref: String,
    artifact_id: u64,
    run_id: u64,
    artifact_name: String,
    artifact_size_bytes: u64,
    candidate_digest: String,
    discovered_at: u64,
    downloaded_at: u64,
    finished_at: u64,
    phase: IngestPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DiscoveryStatus {
    Found(ArtifactCandidate),
    Deferred(String),
}

pub(crate) fn discover_and_ingest(
    request: &HardwareEvidenceRequest<'_>,
    dispatch_token: &str,
) -> Result<HardwareIngestOutcome, String> {
    discover_and_ingest_with_program(request, dispatch_token, OsStr::new("gh"))
}

fn discover_and_ingest_with_program(
    request: &HardwareEvidenceRequest<'_>,
    dispatch_token: &str,
    gh_program: &OsStr,
) -> Result<HardwareIngestOutcome, String> {
    validate_git_digest("dispatch token", dispatch_token)?;
    let Some(source) = hardware_dispatch::discovery_source(request)? else {
        return Ok(HardwareIngestOutcome::Deferred(format!(
            "hardware requirement {} has no local dispatch source for remote evidence discovery",
            request.requirement_id
        )));
    };
    let artifact_name = format!("hardware-evidence-{dispatch_token}");
    let candidate = match discover_candidate(&source, &artifact_name, gh_program)? {
        DiscoveryStatus::Found(candidate) => candidate,
        DiscoveryStatus::Deferred(reason) => return Ok(HardwareIngestOutcome::Deferred(reason)),
    };

    let state_root = request.data_root.join("state/hardware-ingest");
    let binding_root = state_root
        .join(hex_component(request.repository))
        .join(format!("pr-{}", request.pr_number))
        .join(request.head_sha)
        .join(request.requirement_id)
        .join(dispatch_token);
    ensure_managed_directory(request.data_root, &state_root, &binding_root)?;
    let state_path = binding_root.join(format!("artifact-{}.state", candidate.artifact_id));
    let discovered_at = unix_timestamp()?;
    let expected = IngestRecord {
        repository: request.repository.to_owned(),
        pr_number: request.pr_number,
        head_sha: request.head_sha.to_owned(),
        base_sha: request.base_sha.to_owned(),
        policy_identity: request.policy_identity.to_owned(),
        requirement_id: request.requirement_id.to_owned(),
        dispatch_token: dispatch_token.to_owned(),
        dispatch_repository: source.repository.clone(),
        dispatch_workflow: source.workflow.clone(),
        dispatch_ref: source.ref_name.clone(),
        artifact_id: candidate.artifact_id,
        run_id: candidate.run_id,
        artifact_name: candidate.name.clone(),
        artifact_size_bytes: candidate.size_bytes,
        candidate_digest: "none".to_owned(),
        discovered_at,
        downloaded_at: 0,
        finished_at: discovered_at,
        phase: IngestPhase::Deferred,
    };
    if let Some(existing) = read_state(request.data_root, &state_root, &state_path)? {
        let parsed = parse_record(&existing)?;
        validate_record_binding(&expected, &parsed)?;
        if parsed.phase == IngestPhase::VerifiedImported {
            return Err(format!(
                "hardware ingest state {} says verified_imported but canonical evidence was missing when discovery began",
                state_path.display()
            ));
        }
    }

    let attempt = binding_root.join(format!(
        "artifact-{}-attempt-{}-{}",
        candidate.artifact_id,
        discovered_at,
        std::process::id()
    ));
    if fs::symlink_metadata(&attempt).is_ok() {
        remove_managed_tree(request.data_root, &state_root, &attempt)?;
    }
    ensure_managed_directory(request.data_root, &state_root, &attempt)?;

    let download = Command::new(gh_program)
        .arg("run")
        .arg("download")
        .arg(candidate.run_id.to_string())
        .arg("--repo")
        .arg(&source.repository)
        .arg("--name")
        .arg(&artifact_name)
        .arg("--dir")
        .arg(&attempt)
        .output();
    let output = match download {
        Ok(output) => output,
        Err(error) => {
            let mut record = expected.clone();
            record.finished_at = unix_timestamp()?;
            record.phase = IngestPhase::Deferred;
            atomic_replace(&state_path, &serialize_record(&record))?;
            remove_managed_tree(request.data_root, &state_root, &attempt)?;
            return Ok(HardwareIngestOutcome::Deferred(format!(
                "hardware evidence download unavailable for artifact {}: {}",
                candidate.artifact_id,
                bounded_diagnostic(&error.to_string())
            )));
        }
    };
    if !output.status.success() {
        let mut record = expected.clone();
        record.finished_at = unix_timestamp()?;
        record.phase = IngestPhase::Deferred;
        atomic_replace(&state_path, &serialize_record(&record))?;
        let diagnostic = bounded_diagnostic(&String::from_utf8_lossy(&output.stderr));
        remove_managed_tree(request.data_root, &state_root, &attempt)?;
        return Ok(HardwareIngestOutcome::Deferred(format!(
            "hardware evidence download failed for artifact {}: {diagnostic}",
            candidate.artifact_id
        )));
    }

    let candidate_path = match inspect_candidate(&attempt) {
        Ok(path) => path,
        Err(error) => {
            let mut record = expected.clone();
            record.finished_at = unix_timestamp()?;
            record.phase = IngestPhase::Rejected;
            atomic_replace(&state_path, &serialize_record(&record))?;
            remove_managed_tree(request.data_root, &state_root, &attempt)?;
            return Err(error);
        }
    };

    let downloaded_at = unix_timestamp()?;
    let mut candidate_record = expected.clone();
    candidate_record.candidate_digest = candidate_digest(&candidate_path)?;
    candidate_record.downloaded_at = downloaded_at;

    let promoted = match hardware_evidence::promote_candidate_with_program(
        request,
        &candidate_path,
        gh_program,
    ) {
        Ok(status) => status,
        Err(error) => {
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
            record.finished_at = unix_timestamp()?;
            record.phase = IngestPhase::VerifiedImported;
            atomic_replace(&state_path, &serialize_record(&record))?;
            HardwareIngestOutcome::Imported {
                evidence_path,
                artifact_id: candidate.artifact_id,
                run_id: candidate.run_id,
            }
        }
        HardwareEvidenceStatus::Deferred(deferred) => {
            let mut record = candidate_record;
            record.finished_at = unix_timestamp()?;
            record.phase = match deferred.reason {
                HardwareEvidenceDeferReason::AttestationNotVerified => IngestPhase::Rejected,
                _ => IngestPhase::Deferred,
            };
            atomic_replace(&state_path, &serialize_record(&record))?;
            HardwareIngestOutcome::Deferred(format!(
                "remote hardware evidence candidate {} did not become authoritative reason={}: {}",
                candidate.artifact_id,
                deferred.reason.as_str(),
                deferred.detail
            ))
        }
    };
    remove_managed_tree(request.data_root, &state_root, &attempt)?;
    Ok(outcome)
}

fn discover_candidate(
    source: &HardwareDispatchSource,
    artifact_name: &str,
    program: &OsStr,
) -> Result<DiscoveryStatus, String> {
    validate_artifact_name(artifact_name)?;
    let endpoint = format!(
        "repos/{}/actions/artifacts?name={artifact_name}&per_page={MAX_METADATA_LINES}",
        source.repository
    );
    let output = match Command::new(program)
        .arg("api")
        .arg(&endpoint)
        .arg("--jq")
        .arg(".artifacts[] | [.id, .workflow_run.id, .name, (.expired | tostring), .size_in_bytes] | @tsv")
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return Ok(DiscoveryStatus::Deferred(format!(
                "hardware artifact discovery unavailable: {}",
                bounded_diagnostic(&error.to_string())
            )));
        }
    };
    if !output.status.success() {
        return Ok(DiscoveryStatus::Deferred(format!(
            "hardware artifact discovery failed: {}",
            bounded_diagnostic(&String::from_utf8_lossy(&output.stderr))
        )));
    }
    if output.stdout.len() > MAX_METADATA_BYTES {
        return Err("hardware artifact discovery output exceeds bound".to_owned());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("hardware artifact discovery output is not UTF-8: {error}"))?;
    parse_candidate_lines(&stdout, artifact_name)
}

fn parse_candidate_lines(stdout: &str, artifact_name: &str) -> Result<DiscoveryStatus, String> {
    let mut active = Vec::new();
    let mut saw_expired = false;
    let mut line_count = 0usize;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        line_count += 1;
        if line_count > MAX_METADATA_LINES {
            return Err("hardware artifact discovery returned too many candidates".to_owned());
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 5 {
            return Err(format!("malformed hardware artifact metadata: {line:?}"));
        }
        let artifact_id = parse_nonzero_u64("artifact id", fields[0])?;
        let run_id = parse_nonzero_u64("workflow run id", fields[1])?;
        if fields[2] != artifact_name {
            return Err(format!(
                "hardware artifact metadata name mismatch: expected {artifact_name:?}, got {:?}",
                fields[2]
            ));
        }
        let expired = match fields[3] {
            "true" => true,
            "false" => false,
            other => return Err(format!("invalid hardware artifact expired flag: {other}")),
        };
        let size_bytes = fields[4]
            .parse::<u64>()
            .map_err(|error| format!("invalid hardware artifact size {:?}: {error}", fields[4]))?;
        if size_bytes == 0 || size_bytes > MAX_REMOTE_ARTIFACT_BYTES {
            return Err(format!(
                "hardware artifact {artifact_id} has unsafe size {size_bytes}"
            ));
        }
        if expired {
            saw_expired = true;
            continue;
        }
        active.push(ArtifactCandidate {
            artifact_id,
            run_id,
            name: fields[2].to_owned(),
            size_bytes,
        });
    }
    match active.len() {
        0 if saw_expired => Ok(DiscoveryStatus::Deferred(
            "matching hardware evidence artifact exists only in expired form".to_owned(),
        )),
        0 => Ok(DiscoveryStatus::Deferred(
            "no matching remote hardware evidence artifact is available yet".to_owned(),
        )),
        1 => Ok(DiscoveryStatus::Found(active.remove(0))),
        count => Err(format!(
            "ambiguous hardware evidence discovery returned {count} active exact-name artifacts"
        )),
    }
}

fn inspect_candidate(directory: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        format!(
            "failed to inspect hardware ingest directory {}: {error}",
            directory.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "hardware ingest download target must be a non-symlink directory: {}",
            directory.display()
        ));
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read hardware ingest directory: {error}"))?;
    let first = entries
        .next()
        .transpose()
        .map_err(|error| format!("failed to inspect hardware ingest payload: {error}"))?
        .ok_or_else(|| "hardware evidence artifact extracted no payload".to_owned())?;
    if entries
        .next()
        .transpose()
        .map_err(|error| format!("failed to inspect additional hardware ingest payload: {error}"))?
        .is_some()
    {
        return Err("hardware evidence artifact must contain exactly one payload".to_owned());
    }
    if first.file_name() != CANDIDATE_FILENAME {
        return Err(format!(
            "hardware evidence artifact payload must be named {CANDIDATE_FILENAME}"
        ));
    }
    let path = first.path();
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("failed to inspect hardware evidence candidate: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("hardware evidence candidate must be a regular non-symlink file".to_owned());
    }
    if metadata.len() == 0 || metadata.len() > MAX_CANDIDATE_BYTES {
        return Err(format!(
            "hardware evidence candidate has unsafe size {}",
            metadata.len()
        ));
    }
    let canonical_dir = fs::canonicalize(directory)
        .map_err(|error| format!("failed to canonicalize hardware ingest directory: {error}"))?;
    let canonical_path = fs::canonicalize(&path)
        .map_err(|error| format!("failed to canonicalize hardware evidence candidate: {error}"))?;
    if !canonical_path.starts_with(&canonical_dir) {
        return Err("hardware evidence candidate escapes managed download directory".to_owned());
    }
    Ok(path)
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
            "hardware ingest path is outside orchestrator data root: {}",
            directory.display()
        ));
    }
    let relative = directory
        .strip_prefix(data_root)
        .map_err(|_| "hardware ingest directory is outside orchestrator data root".to_owned())?;
    let mut current = data_root.to_path_buf();
    for component in relative.components() {
        use std::path::Component;
        let Component::Normal(component) = component else {
            return Err("hardware ingest directory contains a non-normal component".to_owned());
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "hardware ingest directory component must be a non-symlink directory: {}",
                        current.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    format!(
                        "failed to create hardware ingest directory {}: {error}",
                        current.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect hardware ingest directory {}: {error}",
                    current.display()
                ));
            }
        }
        let canonical_current = fs::canonicalize(&current).map_err(|error| {
            format!("failed to canonicalize hardware ingest directory: {error}")
        })?;
        if !canonical_current.starts_with(&canonical_data) {
            return Err(format!(
                "hardware ingest directory escapes data root: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

fn remove_managed_tree(data_root: &Path, state_root: &Path, path: &Path) -> Result<(), String> {
    if !path.starts_with(state_root) || !state_root.starts_with(data_root) {
        return Err("refusing to clean hardware ingest path outside managed state".to_owned());
    }
    let canonical_data = fs::canonicalize(data_root)
        .map_err(|error| format!("failed to canonicalize data root before cleanup: {error}"))?;
    let canonical_root = fs::canonicalize(state_root)
        .map_err(|error| format!("failed to canonicalize ingest root before cleanup: {error}"))?;
    if !canonical_root.starts_with(&canonical_data) {
        return Err("hardware ingest root escapes data root".to_owned());
    }
    remove_tree_entry(&canonical_root, path)
}

fn remove_tree_entry(canonical_root: &Path, path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to inspect ingest cleanup path {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return fs::remove_file(path)
            .map_err(|error| format!("failed to remove ingest file {}: {error}", path.display()));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "refusing to remove special ingest file {}",
            path.display()
        ));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize ingest cleanup directory: {error}"))?;
    if !canonical.starts_with(canonical_root) {
        return Err(format!(
            "refusing to traverse ingest directory outside root: {}",
            path.display()
        ));
    }
    for entry in fs::read_dir(path)
        .map_err(|error| format!("failed to read ingest cleanup directory: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("failed to read ingest cleanup entry: {error}"))?;
        remove_tree_entry(canonical_root, &entry.path())?;
    }
    fs::remove_dir(path).map_err(|error| {
        format!(
            "failed to remove ingest directory {}: {error}",
            path.display()
        )
    })
}

fn read_state(data_root: &Path, root: &Path, path: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect hardware ingest state: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_STATE_BYTES
    {
        return Err(format!(
            "invalid hardware ingest state file: {}",
            path.display()
        ));
    }
    let canonical_data = fs::canonicalize(data_root)
        .map_err(|error| format!("failed to canonicalize data root for ingest state: {error}"))?;
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| format!("failed to canonicalize ingest state root: {error}"))?;
    let canonical_path = fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize ingest state: {error}"))?;
    if !canonical_root.starts_with(&canonical_data) || !canonical_path.starts_with(&canonical_root)
    {
        return Err("hardware ingest state escapes managed root".to_owned());
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read hardware ingest state: {error}"))?;
    if contents.len() as u64 > MAX_STATE_BYTES {
        return Err("hardware ingest state exceeds bound after read".to_owned());
    }
    Ok(Some(contents))
}

fn atomic_replace(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "hardware ingest state has no parent".to_owned())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "hardware ingest state filename is not UTF-8".to_owned())?;
    let temporary = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), stamp));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("failed to create hardware ingest transaction: {error}"))?;
        file.write_all(contents.as_bytes())
            .map_err(|error| format!("failed to write hardware ingest transaction: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("failed to sync hardware ingest transaction: {error}"))?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("failed to publish hardware ingest state: {error}"))?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

fn serialize_record(record: &IngestRecord) -> String {
    format!(
        "{STATE_VERSION}\nrepository={}\npr_number={}\nhead_sha={}\nbase_sha={}\npolicy_identity={}\nrequirement_id={}\ndispatch_token={}\ndispatch_repository={}\ndispatch_workflow={}\ndispatch_ref={}\nartifact_id={}\nrun_id={}\nartifact_name={}\nartifact_size_bytes={}\ncandidate_digest={}\ndiscovered_at={}\ndownloaded_at={}\nfinished_at={}\nstatus={}\n",
        record.repository,
        record.pr_number,
        record.head_sha,
        record.base_sha,
        record.policy_identity,
        record.requirement_id,
        record.dispatch_token,
        record.dispatch_repository,
        record.dispatch_workflow,
        record.dispatch_ref,
        record.artifact_id,
        record.run_id,
        record.artifact_name,
        record.artifact_size_bytes,
        record.candidate_digest,
        record.discovered_at,
        record.downloaded_at,
        record.finished_at,
        record.phase.as_str(),
    )
}

fn parse_record(contents: &str) -> Result<IngestRecord, String> {
    let fields = parse_fields(contents)?;
    const ALLOWED: &[&str] = &[
        "repository",
        "pr_number",
        "head_sha",
        "base_sha",
        "policy_identity",
        "requirement_id",
        "dispatch_token",
        "dispatch_repository",
        "dispatch_workflow",
        "dispatch_ref",
        "artifact_id",
        "run_id",
        "artifact_name",
        "artifact_size_bytes",
        "candidate_digest",
        "discovered_at",
        "downloaded_at",
        "finished_at",
        "status",
    ];
    reject_unknown_or_missing(&fields, ALLOWED)?;
    let discovered_at = parse_nonzero_u64("discovered_at", required(&fields, "discovered_at")?)?;
    let downloaded_at = parse_u64("downloaded_at", required(&fields, "downloaded_at")?)?;
    let finished_at = parse_nonzero_u64("finished_at", required(&fields, "finished_at")?)?;
    if finished_at < discovered_at
        || (downloaded_at != 0 && (downloaded_at < discovered_at || downloaded_at > finished_at))
    {
        return Err("hardware ingest state timestamps are not ordered".to_owned());
    }
    let candidate_digest = required(&fields, "candidate_digest")?.to_owned();
    validate_candidate_digest(&candidate_digest)?;
    Ok(IngestRecord {
        repository: required(&fields, "repository")?.to_owned(),
        pr_number: parse_nonzero_u64("pr_number", required(&fields, "pr_number")?)?,
        head_sha: required(&fields, "head_sha")?.to_owned(),
        base_sha: required(&fields, "base_sha")?.to_owned(),
        policy_identity: required(&fields, "policy_identity")?.to_owned(),
        requirement_id: required(&fields, "requirement_id")?.to_owned(),
        dispatch_token: required(&fields, "dispatch_token")?.to_owned(),
        dispatch_repository: required(&fields, "dispatch_repository")?.to_owned(),
        dispatch_workflow: required(&fields, "dispatch_workflow")?.to_owned(),
        dispatch_ref: required(&fields, "dispatch_ref")?.to_owned(),
        artifact_id: parse_nonzero_u64("artifact_id", required(&fields, "artifact_id")?)?,
        run_id: parse_nonzero_u64("run_id", required(&fields, "run_id")?)?,
        artifact_name: required(&fields, "artifact_name")?.to_owned(),
        artifact_size_bytes: parse_nonzero_u64(
            "artifact_size_bytes",
            required(&fields, "artifact_size_bytes")?,
        )?,
        candidate_digest,
        discovered_at,
        downloaded_at,
        finished_at,
        phase: IngestPhase::parse(required(&fields, "status")?)?,
    })
}

fn validate_record_binding(expected: &IngestRecord, actual: &IngestRecord) -> Result<(), String> {
    if expected.repository != actual.repository
        || expected.pr_number != actual.pr_number
        || expected.head_sha != actual.head_sha
        || expected.base_sha != actual.base_sha
        || expected.policy_identity != actual.policy_identity
        || expected.requirement_id != actual.requirement_id
        || expected.dispatch_token != actual.dispatch_token
        || expected.dispatch_repository != actual.dispatch_repository
        || expected.dispatch_workflow != actual.dispatch_workflow
        || expected.dispatch_ref != actual.dispatch_ref
        || expected.artifact_id != actual.artifact_id
        || expected.run_id != actual.run_id
        || expected.artifact_name != actual.artifact_name
        || expected.artifact_size_bytes != actual.artifact_size_bytes
    {
        return Err("hardware ingest state does not match exact candidate binding".to_owned());
    }
    Ok(())
}

fn parse_fields(contents: &str) -> Result<BTreeMap<String, String>, String> {
    if contents.as_bytes().contains(&0) {
        return Err("hardware ingest state contains NUL".to_owned());
    }
    let mut lines = contents.lines();
    let version = lines
        .next()
        .ok_or_else(|| "hardware ingest state is empty".to_owned())?;
    if version != STATE_VERSION {
        return Err(format!(
            "unsupported hardware ingest state version: {version}"
        ));
    }
    let mut fields = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed hardware ingest state line: {line:?}"))?;
        if key.is_empty()
            || value.is_empty()
            || key.chars().any(char::is_whitespace)
            || value.chars().any(|ch| ch.is_control())
        {
            return Err("invalid hardware ingest state field".to_owned());
        }
        if fields.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(format!("duplicate hardware ingest state field: {key}"));
        }
    }
    Ok(fields)
}

fn reject_unknown_or_missing(
    fields: &BTreeMap<String, String>,
    allowed: &[&str],
) -> Result<(), String> {
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    if let Some(unknown) = fields.keys().find(|key| !allowed.contains(key.as_str())) {
        return Err(format!("unknown hardware ingest state field: {unknown}"));
    }
    if let Some(missing) = allowed.iter().find(|key| !fields.contains_key(**key)) {
        return Err(format!("missing hardware ingest state field: {missing}"));
    }
    Ok(())
}

fn required<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    fields
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing hardware ingest state field: {key}"))
}

fn validate_artifact_name(value: &str) -> Result<(), String> {
    if value.len() < 20
        || value.len() > 128
        || !value.starts_with("hardware-evidence-")
        || !value["hardware-evidence-".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("invalid deterministic hardware artifact name".to_owned());
    }
    Ok(())
}

fn validate_git_digest(label: &str, value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid hardware ingest {label}"));
    }
    Ok(())
}

fn candidate_digest(path: &Path) -> Result<String, String> {
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

fn parse_nonzero_u64(label: &str, value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|error| format!("invalid hardware ingest {label} {value:?}: {error}"))?;
    if parsed == 0 {
        return Err(format!("hardware ingest {label} must be nonzero"));
    }
    Ok(parsed)
}

fn unix_timestamp() -> Result<u64, String> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_secs();
    if value == 0 {
        return Err("hardware ingest timestamp must be nonzero".to_owned());
    }
    Ok(value)
}

fn hex_component(value: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(value.len().saturating_mul(2));
    for byte in value.as_bytes() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn bounded_diagnostic(value: &str) -> String {
    let result = value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(512)
        .collect::<String>()
        .trim()
        .replace('\n', " ");
    if result.is_empty() {
        "no diagnostic".to_owned()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "orchestrator-hardware-ingest-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn request<'a>(root: &'a Path) -> HardwareEvidenceRequest<'a> {
        HardwareEvidenceRequest {
            data_root: root,
            repository: "Memorithm/Test",
            pr_number: 53,
            head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            policy_identity: "abcd1234",
            requirement_id: "jetson-thor-real-device",
        }
    }

    fn write_dispatch_and_trust(root: &Path) {
        let dispatch = root.join("config/hardware-dispatch");
        fs::create_dir_all(&dispatch).unwrap();
        fs::write(
            dispatch.join("jetson-thor-real-device.state"),
            "v1\nmode=github_workflow\nrepository=Memorithm/hardware-ci\nworkflow=hardware.yml\nref=main\n",
        )
        .unwrap();
        let trust = root.join("config/hardware-trust");
        fs::create_dir_all(&trust).unwrap();
        fs::write(
            trust.join("jetson-thor-real-device.state"),
            "v1\nsigner_workflow=Memorithm/hardware-ci/.github/workflows/verify.yml\nsigner_digest=cccccccccccccccccccccccccccccccccccccccc\n",
        )
        .unwrap();
    }

    fn canonical_evidence(root: &Path) -> PathBuf {
        root.join("state/hardware-evidence")
            .join(hex_component("Memorithm/Test"))
            .join("pr-53")
            .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .join("jetson-thor-real-device.evidence")
    }

    #[cfg(unix)]
    fn write_fake_gh(root: &Path, token: &str, base_sha: &str, label: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let program = root.join(format!("fake-gh-{label}"));
        let marker = root.join(format!("fake-gh-{label}.log"));
        let script = format!(
            r#"#!/bin/sh
set -eu
printf '%s\n' "$1" >> '{marker}'
case "$1" in
  api)
    test "$2" = 'repos/Memorithm/hardware-ci/actions/artifacts?name=hardware-evidence-{token}&per_page=10'
    test "$3" = --jq
    printf '7\t9\thardware-evidence-{token}\tfalse\t512\n'
    ;;
  run)
    test "$2" = download
    test "$3" = 9
    shift 3
    dir=''
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --repo)
          test "$2" = Memorithm/hardware-ci
          shift 2
          ;;
        --name)
          test "$2" = hardware-evidence-{token}
          shift 2
          ;;
        --dir)
          dir="$2"
          shift 2
          ;;
        *) exit 91 ;;
      esac
    done
    test -n "$dir"
    mkdir -p "$dir"
    cat > "$dir/hardware.evidence" <<'EOF'
v1
repository=Memorithm/Test
pr_number=53
head_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
base_sha={base_sha}
policy_identity=abcd1234
requirement_id=jetson-thor-real-device
result=passed
hardware_class=jetson-thor
device_fingerprint=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
started_at=100
finished_at=101
EOF
    ;;
  attestation)
    test "$2" = verify
    test "$4" = --repo
    test "$5" = Memorithm/Test
    test "$6" = --signer-workflow
    test "$7" = Memorithm/hardware-ci/.github/workflows/verify.yml
    test "$8" = --signer-digest
    test "$9" = cccccccccccccccccccccccccccccccccccccccc
    test "${{10}}" = --source-digest
    test "${{11}}" = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    test "${{12}}" = --predicate-type
    test "${{13}}" = https://slsa.dev/provenance/v1
    ;;
  *) exit 92 ;;
esac
"#,
            marker = marker.display(),
            token = token,
            base_sha = base_sha,
        );
        fs::write(&program, script).unwrap();
        let mut permissions = fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&program, permissions).unwrap();
        (program, marker)
    }

    #[cfg(unix)]
    #[test]
    fn exact_remote_candidate_is_downloaded_verified_imported_and_reverified() {
        let root = temp_root("end-to-end");
        write_dispatch_and_trust(&root);
        let req = request(&root);
        let token = hardware_dispatch::binding_token(&req).unwrap();
        let (program, marker) = write_fake_gh(
            &root,
            &token,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "success",
        );

        let outcome = discover_and_ingest_with_program(&req, &token, program.as_os_str()).unwrap();
        assert!(matches!(
            outcome,
            HardwareIngestOutcome::Imported {
                artifact_id: 7,
                run_id: 9,
                ..
            }
        ));
        let canonical = canonical_evidence(&root);
        assert!(canonical.is_file());
        let evidence = fs::read_to_string(&canonical).unwrap();
        assert!(evidence.contains("base_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
        let actions = fs::read_to_string(&marker).unwrap();
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
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mismatched_remote_manifest_never_reaches_canonical_evidence() {
        let root = temp_root("binding-mismatch");
        write_dispatch_and_trust(&root);
        let req = request(&root);
        let token = hardware_dispatch::binding_token(&req).unwrap();
        let (program, marker) = write_fake_gh(
            &root,
            &token,
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "mismatch",
        );

        let error =
            discover_and_ingest_with_program(&req, &token, program.as_os_str()).unwrap_err();
        assert!(error.contains("binding mismatch for base_sha"));
        assert!(!canonical_evidence(&root).exists());
        let actions = fs::read_to_string(&marker).unwrap();
        assert_eq!(actions.lines().collect::<Vec<_>>(), ["api", "run"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artifact_metadata_is_exact_bounded_and_unambiguous() {
        let token = "cccccccccccccccccccccccccccccccccccccccc";
        let name = format!("hardware-evidence-{token}");
        assert!(matches!(
            parse_candidate_lines(&format!("7\t9\t{name}\tfalse\t512\n"), &name).unwrap(),
            DiscoveryStatus::Found(ArtifactCandidate {
                artifact_id: 7,
                run_id: 9,
                ..
            })
        ));
        assert!(matches!(
            parse_candidate_lines(&format!("7\t9\t{name}\ttrue\t512\n"), &name).unwrap(),
            DiscoveryStatus::Deferred(_)
        ));
        assert!(
            parse_candidate_lines(
                &format!("7\t9\t{name}\tfalse\t512\n8\t10\t{name}\tfalse\t512\n"),
                &name
            )
            .is_err()
        );
        assert!(parse_candidate_lines(&format!("7\t9\t{name}\tfalse\t0\n"), &name).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_or_multiple_payloads_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = temp_root("unsafe-payload");
        let download = root.join("download");
        fs::create_dir_all(&download).unwrap();
        let outside = root.join("outside");
        fs::write(&outside, "evidence").unwrap();
        symlink(&outside, download.join(CANDIDATE_FILENAME)).unwrap();
        assert!(inspect_candidate(&download).is_err());
        fs::remove_file(download.join(CANDIDATE_FILENAME)).unwrap();
        fs::write(download.join(CANDIDATE_FILENAME), "evidence").unwrap();
        fs::write(download.join("extra"), "bad").unwrap();
        assert!(inspect_candidate(&download).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
