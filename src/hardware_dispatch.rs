use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::hardware_evidence::HardwareEvidenceRequest;

const CONFIG_VERSION: &str = "v1";
const STATE_VERSION: &str = "v1";
const MAX_CONFIG_BYTES: u64 = 16 * 1024;
const MAX_STATE_BYTES: u64 = 96 * 1024;
const MAX_REPOSITORY_CHARS: usize = 256;
const MAX_WORKFLOW_CHARS: usize = 128;
const MAX_REF_CHARS: usize = 256;
const MAX_REQUIREMENT_ID_CHARS: usize = 96;
const MAX_POLICY_IDENTITY_CHARS: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HardwareDispatchOutcome {
    Dispatched { token: String },
    AlreadyDispatched { token: String },
    Deferred(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HardwareDispatchSource {
    pub(crate) repository: String,
    pub(crate) workflow: String,
    pub(crate) ref_name: String,
}

type DispatchConfig = HardwareDispatchSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchPhase {
    Dispatching,
    Dispatched,
}

impl DispatchPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Dispatching => "dispatching",
            Self::Dispatched => "dispatched",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "dispatching" => Ok(Self::Dispatching),
            "dispatched" => Ok(Self::Dispatched),
            other => Err(format!("unknown hardware dispatch status: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DispatchRecord {
    repository: String,
    pr_number: u64,
    head_sha: String,
    base_sha: String,
    policy_identity: String,
    requirement_id: String,
    dispatch_repository: String,
    workflow: String,
    ref_name: String,
    dispatch_token: String,
    requested_at: u64,
    phase: DispatchPhase,
}

pub(crate) fn binding_token(request: &HardwareEvidenceRequest<'_>) -> Result<String, String> {
    binding_token_with_program(request, OsStr::new("git"))
}

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
    )?
    else {
        return Ok(None);
    };
    parse_config(&contents).map(Some)
}

fn binding_token_with_program(
    request: &HardwareEvidenceRequest<'_>,
    program: &OsStr,
) -> Result<String, String> {
    validate_request(request)?;
    let payload = format!(
        "hardware-dispatch-binding-v1\nrepository-hex={}\npr-number={}\nhead-sha={}\nbase-sha={}\npolicy-identity={}\nrequirement-id={}\n",
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

fn ensure_dispatched_with_program(
    request: &HardwareEvidenceRequest<'_>,
    dispatch_token: &str,
    now: u64,
    program: &OsStr,
) -> Result<HardwareDispatchOutcome, String> {
    validate_request(request)?;
    validate_git_digest("dispatch token", dispatch_token)?;
    if now == 0 {
        return Err("hardware dispatch timestamp must be nonzero".to_owned());
    }

    let config_root = request.data_root.join("config/hardware-dispatch");
    let config_path = config_root.join(format!("{}.state", request.requirement_id));
    let config_contents = match read_regular_bounded(
        request.data_root,
        &config_root,
        &config_path,
        MAX_CONFIG_BYTES,
        "hardware dispatch config",
    )? {
        Some(contents) => contents,
        None => {
            return Ok(HardwareDispatchOutcome::Deferred(format!(
                "hardware requirement {} has no local dispatch configuration",
                request.requirement_id
            )));
        }
    };
    let config = parse_config(&config_contents)?;

    let record = DispatchRecord {
        repository: request.repository.to_owned(),
        pr_number: request.pr_number,
        head_sha: request.head_sha.to_owned(),
        base_sha: request.base_sha.to_owned(),
        policy_identity: request.policy_identity.to_owned(),
        requirement_id: request.requirement_id.to_owned(),
        dispatch_repository: config.repository.clone(),
        workflow: config.workflow.clone(),
        ref_name: config.ref_name.clone(),
        dispatch_token: dispatch_token.to_owned(),
        requested_at: now,
        phase: DispatchPhase::Dispatching,
    };

    let state_root = request.data_root.join("state/hardware-dispatch");
    let state_path = state_root
        .join(hex_component(request.repository))
        .join(format!("pr-{}", request.pr_number))
        .join(request.head_sha)
        .join(request.requirement_id)
        .join(format!("{dispatch_token}.state"));
    let state_parent = state_path
        .parent()
        .ok_or_else(|| "hardware dispatch state has no parent".to_owned())?;
    ensure_managed_directory(request.data_root, &state_root, state_parent)?;

    if let Some(existing) = read_regular_bounded(
        request.data_root,
        &state_root,
        &state_path,
        MAX_STATE_BYTES,
        "hardware dispatch state",
    )? {
        let parsed = parse_record(&existing)?;
        validate_record_binding(&record, &parsed)?;
        return match parsed.phase {
            DispatchPhase::Dispatched => Ok(HardwareDispatchOutcome::AlreadyDispatched {
                token: dispatch_token.to_owned(),
            }),
            DispatchPhase::Dispatching => Ok(HardwareDispatchOutcome::Deferred(format!(
                "hardware dispatch token {dispatch_token} already has an in-progress transaction; refusing duplicate dispatch"
            ))),
        };
    }

    let dispatching_contents = serialize_record(&record);
    claim_dispatch(&state_path, &dispatching_contents)?;

    let fields = [
        ("source_repository", request.repository.to_owned()),
        ("pr_number", request.pr_number.to_string()),
        ("head_sha", request.head_sha.to_owned()),
        ("base_sha", request.base_sha.to_owned()),
        ("policy_identity", request.policy_identity.to_owned()),
        ("requirement_id", request.requirement_id.to_owned()),
        ("dispatch_token", dispatch_token.to_owned()),
    ];
    let mut command = Command::new(program);
    command
        .arg("workflow")
        .arg("run")
        .arg(&config.workflow)
        .arg("--repo")
        .arg(&config.repository)
        .arg("--ref")
        .arg(&config.ref_name);
    for (name, value) in fields {
        command.arg("-f").arg(format!("{name}={value}"));
    }

    let output = match command.output() {
        Ok(output) => output,
        Err(error) => {
            remove_claim(&state_path)?;
            return Ok(HardwareDispatchOutcome::Deferred(format!(
                "hardware workflow dispatcher unavailable for requirement {}: {}",
                request.requirement_id,
                bounded_diagnostic(&error.to_string())
            )));
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        remove_claim(&state_path)?;
        return Ok(HardwareDispatchOutcome::Deferred(format!(
            "hardware workflow dispatch failed for requirement {}: {}",
            request.requirement_id,
            bounded_diagnostic(&stderr)
        )));
    }

    let mut completed = record;
    completed.phase = DispatchPhase::Dispatched;
    atomic_replace(&state_path, &serialize_record(&completed))?;
    Ok(HardwareDispatchOutcome::Dispatched {
        token: dispatch_token.to_owned(),
    })
}

fn claim_dispatch(path: &Path, contents: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "failed to claim hardware dispatch {}: {error}",
                path.display()
            )
        })?;
    file.write_all(contents.as_bytes()).map_err(|error| {
        format!(
            "failed to write hardware dispatch claim {}: {error}",
            path.display()
        )
    })?;
    file.sync_all().map_err(|error| {
        format!(
            "failed to sync hardware dispatch claim {}: {error}",
            path.display()
        )
    })
}

fn remove_claim(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to remove unsuccessful hardware dispatch claim {}: {error}",
            path.display()
        )),
    }
}

fn atomic_replace(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "hardware dispatch state has no parent directory".to_owned())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "hardware dispatch state filename is not UTF-8".to_owned())?;
    let temporary = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), stamp));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| {
                format!(
                    "failed to create hardware dispatch transaction {}: {error}",
                    temporary.display()
                )
            })?;
        file.write_all(contents.as_bytes()).map_err(|error| {
            format!(
                "failed to write hardware dispatch transaction {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync hardware dispatch transaction {}: {error}",
                temporary.display()
            )
        })?;
        fs::rename(&temporary, path).map_err(|error| {
            format!(
                "failed to atomically publish hardware dispatch state {}: {error}",
                path.display()
            )
        })?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
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
            "hardware dispatch state path is outside orchestrator data root: {}",
            directory.display()
        ));
    }

    let relative = directory.strip_prefix(data_root).map_err(|_| {
        format!(
            "hardware dispatch directory is outside orchestrator data root: {}",
            directory.display()
        )
    })?;
    let mut current = data_root.to_path_buf();
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
        if !canonical_current.starts_with(&canonical_data) {
            return Err(format!(
                "hardware dispatch directory component escapes orchestrator data root: {}",
                current.display()
            ));
        }
    }

    let state_metadata = fs::symlink_metadata(state_root).map_err(|error| {
        format!(
            "failed to inspect hardware dispatch state root {}: {error}",
            state_root.display()
        )
    })?;
    if state_metadata.file_type().is_symlink() || !state_metadata.is_dir() {
        return Err(format!(
            "hardware dispatch state root must be a non-symlink directory: {}",
            state_root.display()
        ));
    }
    Ok(())
}

fn read_regular_bounded(
    data_root: &Path,
    root: &Path,
    path: &Path,
    max_bytes: u64,
    label: &str,
) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to inspect {label} {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "{label} must be a regular non-symlink file: {}",
            path.display()
        ));
    }
    if metadata.len() > max_bytes {
        return Err(format!(
            "{label} exceeds {max_bytes} byte bound: {}",
            path.display()
        ));
    }
    let root_metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("failed to inspect {label} root {}: {error}", root.display()))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(format!(
            "{label} root must be a non-symlink directory: {}",
            root.display()
        ));
    }
    let canonical_data = fs::canonicalize(data_root).map_err(|error| {
        format!(
            "failed to canonicalize orchestrator data root {}: {error}",
            data_root.display()
        )
    })?;
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        format!(
            "failed to canonicalize {label} root {}: {error}",
            root.display()
        )
    })?;
    let canonical_path = fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize {label} {}: {error}", path.display()))?;
    if !canonical_root.starts_with(&canonical_data) || !canonical_path.starts_with(&canonical_root)
    {
        return Err(format!(
            "{label} escapes orchestrator-owned root: {}",
            path.display()
        ));
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {label} {}: {error}", path.display()))?;
    if contents.len() as u64 > max_bytes {
        return Err(format!(
            "{label} exceeds {max_bytes} byte bound after read: {}",
            path.display()
        ));
    }
    Ok(Some(contents))
}

fn parse_config(contents: &str) -> Result<DispatchConfig, String> {
    let fields = parse_fields(contents, CONFIG_VERSION, "hardware dispatch config")?;
    const ALLOWED: &[&str] = &["mode", "repository", "workflow", "ref"];
    reject_unknown_or_missing(&fields, ALLOWED, "hardware dispatch config")?;
    let mode = required_field(&fields, "mode", "hardware dispatch config")?;
    if mode != "github_workflow" {
        return Err(format!("unsupported hardware dispatch mode: {mode}"));
    }
    let repository = required_field(&fields, "repository", "hardware dispatch config")?;
    let workflow = required_field(&fields, "workflow", "hardware dispatch config")?;
    let ref_name = required_field(&fields, "ref", "hardware dispatch config")?;
    validate_repository(repository)?;
    validate_workflow(workflow)?;
    validate_ref(ref_name)?;
    Ok(DispatchConfig {
        repository: repository.to_owned(),
        workflow: workflow.to_owned(),
        ref_name: ref_name.to_owned(),
    })
}

fn parse_record(contents: &str) -> Result<DispatchRecord, String> {
    let fields = parse_fields(contents, STATE_VERSION, "hardware dispatch state")?;
    const ALLOWED: &[&str] = &[
        "repository",
        "pr_number",
        "head_sha",
        "base_sha",
        "policy_identity",
        "requirement_id",
        "dispatch_repository",
        "workflow",
        "ref",
        "dispatch_token",
        "requested_at",
        "status",
    ];
    reject_unknown_or_missing(&fields, ALLOWED, "hardware dispatch state")?;
    let record = DispatchRecord {
        repository: required_field(&fields, "repository", "hardware dispatch state")?.to_owned(),
        pr_number: parse_u64(
            "pr_number",
            required_field(&fields, "pr_number", "hardware dispatch state")?,
        )?,
        head_sha: required_field(&fields, "head_sha", "hardware dispatch state")?.to_owned(),
        base_sha: required_field(&fields, "base_sha", "hardware dispatch state")?.to_owned(),
        policy_identity: required_field(&fields, "policy_identity", "hardware dispatch state")?
            .to_owned(),
        requirement_id: required_field(&fields, "requirement_id", "hardware dispatch state")?
            .to_owned(),
        dispatch_repository: required_field(
            &fields,
            "dispatch_repository",
            "hardware dispatch state",
        )?
        .to_owned(),
        workflow: required_field(&fields, "workflow", "hardware dispatch state")?.to_owned(),
        ref_name: required_field(&fields, "ref", "hardware dispatch state")?.to_owned(),
        dispatch_token: required_field(&fields, "dispatch_token", "hardware dispatch state")?
            .to_owned(),
        requested_at: parse_u64(
            "requested_at",
            required_field(&fields, "requested_at", "hardware dispatch state")?,
        )?,
        phase: DispatchPhase::parse(required_field(
            &fields,
            "status",
            "hardware dispatch state",
        )?)?,
    };
    validate_record(&record)?;
    Ok(record)
}

fn serialize_record(record: &DispatchRecord) -> String {
    format!(
        "{STATE_VERSION}\nrepository={}\npr_number={}\nhead_sha={}\nbase_sha={}\npolicy_identity={}\nrequirement_id={}\ndispatch_repository={}\nworkflow={}\nref={}\ndispatch_token={}\nrequested_at={}\nstatus={}\n",
        record.repository,
        record.pr_number,
        record.head_sha,
        record.base_sha,
        record.policy_identity,
        record.requirement_id,
        record.dispatch_repository,
        record.workflow,
        record.ref_name,
        record.dispatch_token,
        record.requested_at,
        record.phase.as_str(),
    )
}

fn validate_record(record: &DispatchRecord) -> Result<(), String> {
    validate_repository(&record.repository)?;
    validate_repository(&record.dispatch_repository)?;
    if record.pr_number == 0 || record.requested_at == 0 {
        return Err("hardware dispatch state contains zero numeric identity".to_owned());
    }
    validate_git_digest("head SHA", &record.head_sha)?;
    validate_git_digest("base SHA", &record.base_sha)?;
    validate_hex(
        "policy identity",
        &record.policy_identity,
        MAX_POLICY_IDENTITY_CHARS,
    )?;
    validate_requirement_id(&record.requirement_id)?;
    validate_workflow(&record.workflow)?;
    validate_ref(&record.ref_name)?;
    validate_git_digest("dispatch token", &record.dispatch_token)
}

fn validate_record_binding(
    expected: &DispatchRecord,
    actual: &DispatchRecord,
) -> Result<(), String> {
    if expected.repository != actual.repository
        || expected.pr_number != actual.pr_number
        || expected.head_sha != actual.head_sha
        || expected.base_sha != actual.base_sha
        || expected.policy_identity != actual.policy_identity
        || expected.requirement_id != actual.requirement_id
        || expected.dispatch_repository != actual.dispatch_repository
        || expected.workflow != actual.workflow
        || expected.ref_name != actual.ref_name
        || expected.dispatch_token != actual.dispatch_token
    {
        return Err(
            "hardware dispatch state does not match exact current binding/configuration".to_owned(),
        );
    }
    Ok(())
}

fn parse_fields(
    contents: &str,
    expected_version: &str,
    label: &str,
) -> Result<BTreeMap<String, String>, String> {
    let mut lines = contents.lines();
    let version = lines.next().unwrap_or_default();
    if version != expected_version {
        return Err(format!("unsupported {label} version: {version}"));
    }
    let mut fields = BTreeMap::new();
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

fn reject_unknown_or_missing(
    fields: &BTreeMap<String, String>,
    allowed: &[&str],
    label: &str,
) -> Result<(), String> {
    if let Some(unknown) = fields.keys().find(|name| !allowed.contains(&name.as_str())) {
        return Err(format!("unknown {label} field: {unknown}"));
    }
    for required in allowed {
        if !fields.contains_key(*required) {
            return Err(format!("missing {label} field: {required}"));
        }
    }
    if fields.len() != allowed.len() {
        return Err(format!("invalid {label} field count"));
    }
    Ok(())
}

fn required_field<'a>(
    fields: &'a BTreeMap<String, String>,
    name: &str,
    label: &str,
) -> Result<&'a str, String> {
    fields
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {label} field: {name}"))
}

fn validate_request(request: &HardwareEvidenceRequest<'_>) -> Result<(), String> {
    validate_repository(request.repository)?;
    if request.pr_number == 0 {
        return Err("hardware dispatch PR number must be nonzero".to_owned());
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

fn validate_repository(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_REPOSITORY_CHARS
        || value
            .bytes()
            .any(|byte| matches!(byte, b'\n' | b'\r' | 0 | b'\\'))
    {
        return Err("invalid hardware dispatch repository".to_owned());
    }
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        return Err("hardware dispatch repository must be owner/name".to_owned());
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
        return Err(format!("invalid hardware dispatch {label}"));
    }
    Ok(())
}

fn validate_workflow(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_WORKFLOW_CHARS
        || value.contains('/')
        || value.contains('\\')
        || !(value.ends_with(".yml") || value.ends_with(".yaml"))
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid hardware dispatch workflow filename".to_owned());
    }
    Ok(())
}

fn validate_ref(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_REF_CHARS
        || value.starts_with('/')
        || value.ends_with('/')
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("@{")
        || value.ends_with(".lock")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err("invalid hardware dispatch ref".to_owned());
    }
    if value.split('/').any(|segment| {
        segment.is_empty() || matches!(segment, "." | "..") || segment.ends_with(".lock")
    }) {
        return Err("invalid hardware dispatch ref segment".to_owned());
    }
    Ok(())
}

fn validate_requirement_id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_REQUIREMENT_ID_CHARS {
        return Err("invalid hardware dispatch requirement_id".to_owned());
    }
    let mut bytes = value.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid hardware dispatch requirement_id".to_owned());
    }
    Ok(())
}

fn validate_git_digest(label: &str, value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid hardware dispatch {label}"));
    }
    Ok(())
}

fn validate_hex(label: &str, value: &str, max_chars: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_chars
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("invalid hardware dispatch {label}"));
    }
    Ok(())
}

fn parse_u64(label: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid hardware dispatch integer {label}: {value}"))
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
        "no dispatcher diagnostic".to_owned()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "orchestrator-hardware-dispatch-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn request<'a>(root: &'a Path) -> HardwareEvidenceRequest<'a> {
        HardwareEvidenceRequest {
            data_root: root,
            repository: "Memorithm/Test",
            pr_number: 51,
            head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            policy_identity: "abcd1234",
            requirement_id: "jetson-thor-real-device",
        }
    }

    fn write_config(root: &Path) {
        let directory = root.join("config/hardware-dispatch");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("jetson-thor-real-device.state"),
            "v1\nmode=github_workflow\nrepository=Memorithm/hardware-ci\nworkflow=dispatch.yml\nref=main\n",
        )
        .unwrap();
    }

    #[test]
    fn missing_config_defers_without_remote_mutation() {
        let root = temp_root("missing-config");
        let outcome = ensure_dispatched_with_program(
            &request(&root),
            "cccccccccccccccccccccccccccccccccccccccc",
            100,
            OsStr::new("definitely-not-a-dispatcher"),
        )
        .unwrap();
        assert!(
            matches!(outcome, HardwareDispatchOutcome::Deferred(reason) if reason.contains("no local dispatch configuration"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_config_fails_closed() {
        let root = temp_root("bad-config");
        let directory = root.join("config/hardware-dispatch");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("jetson-thor-real-device.state"),
            "v1\nmode=github_workflow\nrepository=Memorithm/hardware-ci\nworkflow=../dispatch.yml\nref=main\n",
        )
        .unwrap();
        let error = ensure_dispatched_with_program(
            &request(&root),
            "cccccccccccccccccccccccccccccccccccccccc",
            100,
            OsStr::new("unused"),
        )
        .unwrap_err();
        assert!(error.contains("workflow filename"));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn exact_missing_evidence_dispatch_is_literal_and_idempotent() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("dispatch");
        write_config(&root);
        let log = root.join("dispatch.log");
        let fake = root.join("fake-gh");
        fs::write(
            &fake,
            format!(
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" > '{}'\nexit 0\n",
                log.display()
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake, permissions).unwrap();
        let token = "cccccccccccccccccccccccccccccccccccccccc";

        let first =
            ensure_dispatched_with_program(&request(&root), token, 100, fake.as_os_str()).unwrap();
        assert_eq!(
            first,
            HardwareDispatchOutcome::Dispatched {
                token: token.to_owned()
            }
        );
        let arguments = fs::read_to_string(&log).unwrap();
        for expected in [
            "workflow",
            "run",
            "dispatch.yml",
            "--repo",
            "Memorithm/hardware-ci",
            "--ref",
            "main",
            "source_repository=Memorithm/Test",
            "pr_number=51",
            "head_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "base_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "policy_identity=abcd1234",
            "requirement_id=jetson-thor-real-device",
            "dispatch_token=cccccccccccccccccccccccccccccccccccccccc",
        ] {
            assert!(
                arguments.lines().any(|line| line == expected),
                "missing {expected:?} in {arguments:?}"
            );
        }

        fs::write(&log, "not-dispatched-again\n").unwrap();
        let second =
            ensure_dispatched_with_program(&request(&root), token, 101, fake.as_os_str()).unwrap();
        assert_eq!(
            second,
            HardwareDispatchOutcome::AlreadyDispatched {
                token: token.to_owned()
            }
        );
        assert_eq!(fs::read_to_string(&log).unwrap(), "not-dispatched-again\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_dispatch_leaves_no_success_and_can_retry() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("failed");
        write_config(&root);
        let fake = root.join("fake-gh-fail");
        fs::write(&fake, "#!/bin/sh\necho dispatch-failed >&2\nexit 1\n").unwrap();
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake, permissions).unwrap();
        let token = "cccccccccccccccccccccccccccccccccccccccc";
        let outcome =
            ensure_dispatched_with_program(&request(&root), token, 100, fake.as_os_str()).unwrap();
        assert!(
            matches!(outcome, HardwareDispatchOutcome::Deferred(reason) if reason.contains("dispatch failed"))
        );

        let success = root.join("fake-gh-success");
        fs::write(&success, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&success).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&success, permissions).unwrap();
        let retry =
            ensure_dispatched_with_program(&request(&root), token, 101, success.as_os_str())
                .unwrap();
        assert!(matches!(retry, HardwareDispatchOutcome::Dispatched { .. }));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_claim_never_causes_duplicate_dispatch() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("claim");
        write_config(&root);
        let token = "cccccccccccccccccccccccccccccccccccccccc";
        let req = request(&root);
        let config = parse_config(
            &fs::read_to_string(
                root.join("config/hardware-dispatch/jetson-thor-real-device.state"),
            )
            .unwrap(),
        )
        .unwrap();
        let record = DispatchRecord {
            repository: req.repository.to_owned(),
            pr_number: req.pr_number,
            head_sha: req.head_sha.to_owned(),
            base_sha: req.base_sha.to_owned(),
            policy_identity: req.policy_identity.to_owned(),
            requirement_id: req.requirement_id.to_owned(),
            dispatch_repository: config.repository,
            workflow: config.workflow,
            ref_name: config.ref_name,
            dispatch_token: token.to_owned(),
            requested_at: 100,
            phase: DispatchPhase::Dispatching,
        };
        let state_root = root.join("state/hardware-dispatch");
        let path = state_root
            .join(hex_component(req.repository))
            .join("pr-51")
            .join(req.head_sha)
            .join(req.requirement_id)
            .join(format!("{token}.state"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serialize_record(&record)).unwrap();

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
        let outcome = ensure_dispatched_with_program(&req, token, 101, fake.as_os_str()).unwrap();
        assert!(
            matches!(outcome, HardwareDispatchOutcome::Deferred(reason) if reason.contains("in-progress transaction"))
        );
        assert!(!marker.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
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
        use std::os::unix::fs::{PermissionsExt, symlink};

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

    #[test]
    fn changed_binding_or_config_cannot_reuse_existing_dispatch_state() {
        let root = temp_root("mismatch");
        write_config(&root);
        let token = "cccccccccccccccccccccccccccccccccccccccc";
        let req = request(&root);
        let config = parse_config(
            &fs::read_to_string(
                root.join("config/hardware-dispatch/jetson-thor-real-device.state"),
            )
            .unwrap(),
        )
        .unwrap();
        let record = DispatchRecord {
            repository: req.repository.to_owned(),
            pr_number: req.pr_number,
            head_sha: req.head_sha.to_owned(),
            base_sha: req.base_sha.to_owned(),
            policy_identity: req.policy_identity.to_owned(),
            requirement_id: req.requirement_id.to_owned(),
            dispatch_repository: config.repository,
            workflow: config.workflow,
            ref_name: config.ref_name,
            dispatch_token: token.to_owned(),
            requested_at: 100,
            phase: DispatchPhase::Dispatched,
        };
        let state_root = root.join("state/hardware-dispatch");
        let path = state_root
            .join(hex_component(req.repository))
            .join("pr-51")
            .join(req.head_sha)
            .join(req.requirement_id)
            .join(format!("{token}.state"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serialize_record(&record)).unwrap();
        fs::write(
            root.join("config/hardware-dispatch/jetson-thor-real-device.state"),
            "v1\nmode=github_workflow\nrepository=Memorithm/hardware-ci\nworkflow=different.yml\nref=main\n",
        )
        .unwrap();
        let error =
            ensure_dispatched_with_program(&req, token, 101, OsStr::new("unused")).unwrap_err();
        assert!(error.contains("does not match exact current binding/configuration"));
        fs::remove_dir_all(root).unwrap();
    }
}
