use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const TRUST_VERSION: &str = "v1";
const EVIDENCE_VERSION: &str = "v1";
const MAX_TRUST_BYTES: u64 = 8 * 1024;
const MAX_EVIDENCE_BYTES: u64 = 128 * 1024;
const MAX_REPOSITORY_CHARS: usize = 256;
const MAX_POLICY_IDENTITY_CHARS: usize = 65_536;
const MAX_REQUIREMENT_ID_CHARS: usize = 96;
const MAX_HARDWARE_CLASS_CHARS: usize = 64;
const MAX_SIGNER_WORKFLOW_CHARS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HardwareEvidenceRequest<'a> {
    pub(crate) data_root: &'a Path,
    pub(crate) repository: &'a str,
    pub(crate) pr_number: u64,
    pub(crate) head_sha: &'a str,
    pub(crate) base_sha: &'a str,
    pub(crate) policy_identity: &'a str,
    pub(crate) requirement_id: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct HardwareTrust {
    signer_workflow: String,
    signer_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HardwareManifest {
    repository: String,
    pr_number: u64,
    head_sha: String,
    base_sha: String,
    policy_identity: String,
    requirement_id: String,
    hardware_class: String,
    device_fingerprint: String,
}

pub(crate) fn verify(
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
    let candidate_status =
        verify_path_with_program(request, request.data_root, candidate_path, verifier)?;
    let HardwareEvidenceStatus::Satisfied { .. } = candidate_status else {
        return Ok(candidate_status);
    };

    let canonical_path = canonical_evidence_path(request)?;
    if fs::symlink_metadata(&canonical_path).is_ok() {
        return verify_with_program(request, verifier);
    }
    let contents = read_regular_bounded(request.data_root, candidate_path, MAX_EVIDENCE_BYTES)?
        .ok_or_else(|| {
            "verified hardware evidence candidate disappeared before promotion".to_owned()
        })?;
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
    let evidence_contents =
        match read_regular_bounded(managed_root, evidence_path, MAX_EVIDENCE_BYTES)? {
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
            return Err(
                "hardware evidence promotion path contains a non-normal component".to_owned(),
            );
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

fn validate_request(request: &HardwareEvidenceRequest<'_>) -> Result<(), String> {
    validate_repository(request.repository)?;
    if request.pr_number == 0 {
        return Err("hardware evidence PR number must be nonzero".to_owned());
    }
    validate_git_digest("head SHA", request.head_sha)?;
    validate_git_digest("base SHA", request.base_sha)?;
    validate_hex(
        "policy identity",
        request.policy_identity,
        MAX_POLICY_IDENTITY_CHARS,
    )?;
    validate_requirement_id(request.requirement_id)
}

fn read_regular_bounded(
    root: &Path,
    path: &Path,
    max_bytes: u64,
) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to inspect hardware evidence state {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "hardware evidence state must be a regular non-symlink file: {}",
            path.display()
        ));
    }
    if metadata.len() > max_bytes {
        return Err(format!(
            "hardware evidence state exceeds {} byte bound: {}",
            max_bytes,
            path.display()
        ));
    }

    let canonical_root = fs::canonicalize(root).map_err(|error| {
        format!(
            "failed to canonicalize hardware evidence root {}: {error}",
            root.display()
        )
    })?;
    let canonical_path = fs::canonicalize(path).map_err(|error| {
        format!(
            "failed to canonicalize hardware evidence state {}: {error}",
            path.display()
        )
    })?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(format!(
            "hardware evidence state escapes managed root: {}",
            path.display()
        ));
    }

    let contents = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read hardware evidence state {}: {error}",
            path.display()
        )
    })?;
    if contents.len() as u64 > max_bytes {
        return Err(format!(
            "hardware evidence state exceeds {} byte bound after read: {}",
            max_bytes,
            path.display()
        ));
    }
    Ok(Some(contents))
}

fn parse_trust(contents: &str) -> Result<HardwareTrust, String> {
    let fields = parse_fields(contents, TRUST_VERSION, "hardware trust")?;
    let signer_workflow = required_field(&fields, "signer_workflow", "hardware trust")?;
    let signer_digest = required_field(&fields, "signer_digest", "hardware trust")?;
    if fields.len() != 2 {
        let unknown = fields
            .keys()
            .find(|name| !matches!(name.as_str(), "signer_workflow" | "signer_digest"))
            .cloned()
            .unwrap_or_else(|| "duplicate-or-extra".to_owned());
        return Err(format!("unknown hardware trust field: {unknown}"));
    }
    validate_signer_workflow(signer_workflow)?;
    validate_git_digest("signer digest", signer_digest)?;
    Ok(HardwareTrust {
        signer_workflow: signer_workflow.to_owned(),
        signer_digest: signer_digest.to_owned(),
    })
}

fn parse_manifest(contents: &str) -> Result<HardwareManifest, String> {
    let fields = parse_fields(contents, EVIDENCE_VERSION, "hardware evidence")?;
    const ALLOWED: &[&str] = &[
        "repository",
        "pr_number",
        "head_sha",
        "base_sha",
        "policy_identity",
        "requirement_id",
        "result",
        "hardware_class",
        "device_fingerprint",
        "started_at",
        "finished_at",
    ];
    if fields.len() != ALLOWED.len()
        && let Some(unknown) = fields.keys().find(|name| !ALLOWED.contains(&name.as_str()))
    {
        return Err(format!("unknown hardware evidence field: {unknown}"));
    }
    for field in ALLOWED {
        if !fields.contains_key(*field) {
            return Err(format!("missing hardware evidence field: {field}"));
        }
    }

    let repository = required_field(&fields, "repository", "hardware evidence")?.to_owned();
    let pr_number = parse_u64(
        "pr_number",
        required_field(&fields, "pr_number", "hardware evidence")?,
    )?;
    let head_sha = required_field(&fields, "head_sha", "hardware evidence")?.to_owned();
    let base_sha = required_field(&fields, "base_sha", "hardware evidence")?.to_owned();
    let policy_identity =
        required_field(&fields, "policy_identity", "hardware evidence")?.to_owned();
    let requirement_id = required_field(&fields, "requirement_id", "hardware evidence")?.to_owned();
    let result = required_field(&fields, "result", "hardware evidence")?;
    if result != "passed" {
        return Err(format!(
            "hardware evidence result must be passed, got {result}"
        ));
    }
    let hardware_class = required_field(&fields, "hardware_class", "hardware evidence")?.to_owned();
    let device_fingerprint =
        required_field(&fields, "device_fingerprint", "hardware evidence")?.to_owned();
    let started_at = parse_u64(
        "started_at",
        required_field(&fields, "started_at", "hardware evidence")?,
    )?;
    let finished_at = parse_u64(
        "finished_at",
        required_field(&fields, "finished_at", "hardware evidence")?,
    )?;

    validate_repository(&repository)?;
    if pr_number == 0 {
        return Err("hardware evidence PR number must be nonzero".to_owned());
    }
    validate_git_digest("head SHA", &head_sha)?;
    validate_git_digest("base SHA", &base_sha)?;
    validate_hex(
        "policy identity",
        &policy_identity,
        MAX_POLICY_IDENTITY_CHARS,
    )?;
    validate_requirement_id(&requirement_id)?;
    validate_path_token("hardware class", &hardware_class, MAX_HARDWARE_CLASS_CHARS)?;
    validate_exact_hex("device fingerprint", &device_fingerprint, 64)?;
    if started_at == 0 || finished_at == 0 || finished_at < started_at {
        return Err(format!(
            "invalid hardware evidence timestamps: started_at={started_at} finished_at={finished_at}"
        ));
    }

    Ok(HardwareManifest {
        repository,
        pr_number,
        head_sha,
        base_sha,
        policy_identity,
        requirement_id,
        hardware_class,
        device_fingerprint,
    })
}

fn validate_manifest_binding(
    request: &HardwareEvidenceRequest<'_>,
    manifest: &HardwareManifest,
) -> Result<(), String> {
    let checks = [
        (
            "repository",
            manifest.repository.as_str(),
            request.repository,
        ),
        ("head_sha", manifest.head_sha.as_str(), request.head_sha),
        ("base_sha", manifest.base_sha.as_str(), request.base_sha),
        (
            "policy_identity",
            manifest.policy_identity.as_str(),
            request.policy_identity,
        ),
        (
            "requirement_id",
            manifest.requirement_id.as_str(),
            request.requirement_id,
        ),
    ];
    for (field, actual, expected) in checks {
        if actual != expected {
            return Err(format!(
                "hardware evidence binding mismatch for {field}: expected {expected:?}, got {actual:?}"
            ));
        }
    }
    if manifest.pr_number != request.pr_number {
        return Err(format!(
            "hardware evidence binding mismatch for pr_number: expected {}, got {}",
            request.pr_number, manifest.pr_number
        ));
    }
    Ok(())
}

fn parse_fields(
    contents: &str,
    expected_version: &str,
    label: &str,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut lines = contents.lines();
    let version = lines.next().unwrap_or_default();
    if version != expected_version {
        return Err(format!("unsupported {label} version: {version}"));
    }
    let mut fields = std::collections::BTreeMap::new();
    let mut seen = BTreeSet::new();
    for line in lines {
        if line.is_empty() {
            return Err(format!("empty field line in {label}"));
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed {label} field: {line}"))?;
        if name.is_empty() || value.is_empty() {
            return Err(format!("empty {label} field name or value: {line}"));
        }
        if !seen.insert(name.to_owned()) {
            return Err(format!("duplicate {label} field: {name}"));
        }
        fields.insert(name.to_owned(), value.to_owned());
    }
    Ok(fields)
}

fn required_field<'a>(
    fields: &'a std::collections::BTreeMap<String, String>,
    name: &str,
    label: &str,
) -> Result<&'a str, String> {
    fields
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {label} field: {name}"))
}

fn parse_u64(label: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid hardware evidence integer {label}: {value}"))
}

fn validate_repository(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_REPOSITORY_CHARS
        || value
            .bytes()
            .any(|byte| matches!(byte, b'\n' | b'\r' | 0 | b'\\'))
    {
        return Err("invalid hardware evidence repository".to_owned());
    }
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        return Err("hardware evidence repository must be owner/name".to_owned());
    }
    validate_github_component("repository owner", owner)?;
    validate_github_component("repository name", repository)
}

fn validate_github_component(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || matches!(value, "." | "..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("invalid hardware evidence {label}"));
    }
    Ok(())
}

fn validate_requirement_id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_REQUIREMENT_ID_CHARS {
        return Err("invalid hardware evidence requirement_id".to_owned());
    }
    let mut bytes = value.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid hardware evidence requirement_id".to_owned());
    }
    Ok(())
}

fn validate_path_token(label: &str, value: &str, max_chars: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_chars
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("invalid hardware evidence {label}"));
    }
    Ok(())
}

fn validate_signer_workflow(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_SIGNER_WORKFLOW_CHARS
        || value
            .bytes()
            .any(|byte| matches!(byte, b'\n' | b'\r' | 0 | b'\\' | b'@'))
    {
        return Err("invalid hardware trust signer_workflow".to_owned());
    }
    let parts = value.split('/').collect::<Vec<_>>();
    if parts.len() != 5 || parts[2] != ".github" || parts[3] != "workflows" {
        return Err(
            "hardware trust signer_workflow must be owner/repo/.github/workflows/file.yml"
                .to_owned(),
        );
    }
    validate_github_component("signer workflow owner", parts[0])?;
    validate_github_component("signer workflow repository", parts[1])?;
    let file = parts[4];
    if !(file.ends_with(".yml") || file.ends_with(".yaml"))
        || !file
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid hardware trust workflow filename".to_owned());
    }
    Ok(())
}

fn validate_git_digest(label: &str, value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid hardware evidence {label}"));
    }
    Ok(())
}

fn validate_hex(label: &str, value: &str, max_chars: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_chars
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("invalid hardware evidence {label}"));
    }
    Ok(())
}

fn validate_exact_hex(label: &str, value: &str, chars: usize) -> Result<(), String> {
    if value.len() != chars || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid hardware evidence {label}"));
    }
    Ok(())
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
    let mut result = value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(512)
        .collect::<String>();
    result = result.trim().replace('\n', " ");
    if result.is_empty() {
        "no verifier diagnostic".to_owned()
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
            "orchestrator-hardware-evidence-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn request<'a>(root: &'a Path) -> HardwareEvidenceRequest<'a> {
        HardwareEvidenceRequest {
            data_root: root,
            repository: "Memorithm/Test",
            pr_number: 49,
            head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            policy_identity: "abcd1234",
            requirement_id: "jetson-thor-real-device",
        }
    }

    fn write_trust(root: &Path) {
        let directory = root.join("config/hardware-trust");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("jetson-thor-real-device.state"),
            "v1\nsigner_workflow=Memorithm/hardware-ci/.github/workflows/verify.yml\nsigner_digest=cccccccccccccccccccccccccccccccccccccccc\n",
        )
        .unwrap();
    }

    fn evidence_path(root: &Path) -> PathBuf {
        root.join("state/hardware-evidence")
            .join(hex_component("Memorithm/Test"))
            .join("pr-49")
            .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .join("jetson-thor-real-device.evidence")
    }

    fn write_evidence(root: &Path) {
        let path = evidence_path(root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            "v1\nrepository=Memorithm/Test\npr_number=49\nhead_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbase_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\npolicy_identity=abcd1234\nrequirement_id=jetson-thor-real-device\nresult=passed\nhardware_class=jetson-thor\ndevice_fingerprint=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\nstarted_at=100\nfinished_at=101\n",
        )
        .unwrap();
    }

    #[test]
    fn missing_trust_or_evidence_defers_without_verification() {
        let root = temp_root("missing");
        let req = request(&root);
        let status =
            verify_with_program(&req, OsStr::new("definitely-not-a-real-verifier")).unwrap();
        assert!(matches!(
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
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_or_mismatched_state_fails_closed() {
        let root = temp_root("mismatch");
        write_trust(&root);
        write_evidence(&root);
        let path = evidence_path(&root);
        let contents = fs::read_to_string(&path).unwrap();
        fs::write(&path, contents.replace("base_sha=bbbb", "base_sha=eeee")).unwrap();
        let error = verify_with_program(&request(&root), OsStr::new("unused")).unwrap_err();
        assert!(error.contains("binding mismatch for base_sha"));

        fs::write(
            root.join("config/hardware-trust/jetson-thor-real-device.state"),
            "v2\nsigner_workflow=Memorithm/hardware-ci/.github/workflows/verify.yml\nsigner_digest=cccccccccccccccccccccccccccccccccccccccc\n",
        )
        .unwrap();
        let error = verify_with_program(&request(&root), OsStr::new("unused")).unwrap_err();
        assert!(error.contains("unsupported hardware trust version"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_parser_rejects_unknown_duplicate_and_invalid_terminal_data() {
        let base = "v1\nrepository=Memorithm/Test\npr_number=49\nhead_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbase_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\npolicy_identity=abcd1234\nrequirement_id=jetson-thor-real-device\nresult=passed\nhardware_class=jetson-thor\ndevice_fingerprint=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\nstarted_at=100\nfinished_at=101\n";
        assert!(parse_manifest(base).is_ok());
        assert!(parse_manifest(&format!("{base}unknown=value\n")).is_err());
        assert!(
            parse_manifest(&base.replace("pr_number=49\n", "pr_number=49\npr_number=49\n"))
                .is_err()
        );
        assert!(parse_manifest(&base.replace("result=passed", "result=failed")).is_err());
        assert!(parse_manifest(&base.replace("finished_at=101", "finished_at=99")).is_err());
    }

    #[test]
    fn trust_parser_rejects_repository_controlled_or_ambiguous_signer_forms() {
        assert!(parse_trust("v1\nsigner_workflow=Memorithm/hardware-ci/.github/workflows/verify.yml\nsigner_digest=cccccccccccccccccccccccccccccccccccccccc\n").is_ok());
        assert!(parse_trust("v1\nsigner_workflow=Memorithm/hardware-ci/.github/workflows/verify.yml@main\nsigner_digest=cccccccccccccccccccccccccccccccccccccccc\n").is_err());
        assert!(parse_trust("v1\nsigner_workflow=Memorithm/hardware-ci/.github/workflows/verify.yml\nsigner_digest=cccccccccccccccccccccccccccccccccccccccc\nextra=bad\n").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn exact_manifest_and_pinned_verifier_satisfy_hardware_gate() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("verified");
        write_trust(&root);
        write_evidence(&root);
        let verifier = root.join("fake-gh");
        let expected_path = evidence_path(&root);
        fs::write(
            &verifier,
            format!(
                "#!/bin/sh\nset -eu\ntest \"$1\" = attestation\ntest \"$2\" = verify\ntest \"$3\" = \"{}\"\ntest \"$4\" = --repo\ntest \"$5\" = Memorithm/Test\ntest \"$6\" = --signer-workflow\ntest \"$7\" = Memorithm/hardware-ci/.github/workflows/verify.yml\ntest \"$8\" = --signer-digest\ntest \"$9\" = cccccccccccccccccccccccccccccccccccccccc\ntest \"${{10}}\" = --source-digest\ntest \"${{11}}\" = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nexit 0\n",
                expected_path.display()
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&verifier).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&verifier, permissions).unwrap();

        let status = verify_with_program(&request(&root), verifier.as_os_str()).unwrap();
        assert!(matches!(
            status,
            HardwareEvidenceStatus::Satisfied {
                hardware_class,
                device_fingerprint,
                ..
            } if hardware_class == "jetson-thor" && device_fingerprint == "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
        ));
        fs::remove_dir_all(root).unwrap();
    }

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
    #[cfg(unix)]
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

        let status =
            promote_candidate_with_program(&request(&root), &candidate, verifier.as_os_str())
                .unwrap();
        assert!(matches!(status, HardwareEvidenceStatus::Satisfied { .. }));
        assert_eq!(fs::read_to_string(&canonical).unwrap(), source);
        assert_eq!(fs::read_to_string(&marker).unwrap().lines().count(), 2);
        assert!(canonical_parent.is_dir());

        fs::write(
            &candidate,
            source.replace("hardware_class=jetson-thor", "hardware_class=other"),
        )
        .unwrap();
        let status =
            promote_candidate_with_program(&request(&root), &candidate, verifier.as_os_str())
                .unwrap();
        assert!(matches!(status, HardwareEvidenceStatus::Satisfied { .. }));
        assert_eq!(fs::read_to_string(&canonical).unwrap(), source);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_cryptographic_verifier_never_satisfies_evidence() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("verify-fail");
        write_trust(&root);
        write_evidence(&root);
        let verifier = root.join("fake-gh-fail");
        fs::write(
            &verifier,
            "#!/bin/sh\necho invalid-attestation >&2\nexit 1\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&verifier).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&verifier, permissions).unwrap();

        let status = verify_with_program(&request(&root), verifier.as_os_str()).unwrap();
        assert!(matches!(
            status,
            HardwareEvidenceStatus::Deferred(HardwareEvidenceDeferred {
                reason: HardwareEvidenceDeferReason::AttestationNotVerified,
                detail,
            }) if detail.contains("did not verify")
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
