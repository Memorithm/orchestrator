use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::hardware_dispatch::HardwareDispatchSource;
use crate::hardware_evidence::HardwareEvidenceRequest;

const CONFIG_VERSION: &str = "v1";
const AUDIT_VERSION: &str = "v1";
const MAX_CONFIG_BYTES: u64 = 32 * 1024;
const MAX_QUERY_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_RUNNERS: usize = 16;
const MAX_RUNNER_RESPONSE_LINES: usize = 100;
const MAX_LABELS: usize = 64;
const MAX_TOKEN_CHARS: usize = 128;
const MAX_LABEL_CHARS: usize = 256;
const MAX_ENCODED_FIELD_CHARS: usize = 4096;
const MAX_POLICY_IDENTITY_CHARS: usize = 65_536;
const MAX_REQUIREMENT_ID_CHARS: usize = 96;
const MAX_REPOSITORY_CHARS: usize = 256;
const MAX_AUDIT_BYTES: usize = 96 * 1024;
const RUNNER_JQ: &str = ".runners[] | [.id, (.name|@uri), (.os|@uri), .status, (.busy|tostring), ([.labels[].name|@uri] | join(\",\"))] | @tsv";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HardwareCapabilityOutcome {
    Schedulable,
    Deferred(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapabilityMode {
    Manual,
    Hosted,
    SelfHostedRepository,
}

impl CapabilityMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Hosted => "hosted",
            Self::SelfHostedRepository => "self_hosted_repository",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapabilityConfig {
    mode: CapabilityMode,
    repository: String,
    runners: Vec<String>,
    required_labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunnerObservation {
    id: u64,
    name: String,
    os: String,
    status: String,
    busy: bool,
    labels: Vec<String>,
    audit_row: String,
}

#[derive(Debug)]
struct AuditRecord<'a> {
    request: &'a HardwareEvidenceRequest<'a>,
    source: &'a HardwareDispatchSource,
    config: &'a CapabilityConfig,
    observed_at: u64,
    outcome: &'a str,
    observations: &'a [RunnerObservation],
    query_errors: &'a [String],
}

pub(crate) fn check_schedulable_with_program(
    request: &HardwareEvidenceRequest<'_>,
    source: &HardwareDispatchSource,
    observed_at: u64,
    program: &OsStr,
) -> Result<HardwareCapabilityOutcome, String> {
    validate_request(request)?;
    validate_source(source)?;
    if observed_at == 0 {
        return Err("hardware capability observation timestamp must be nonzero".to_owned());
    }

    let config_root = request.data_root.join("config/hardware-capabilities");
    let config_path = config_root.join(format!("{}.state", request.requirement_id));
    let Some(contents) = read_regular_bounded(
        request.data_root,
        &config_root,
        &config_path,
        MAX_CONFIG_BYTES,
        "hardware capability inventory",
    )?
    else {
        return Ok(HardwareCapabilityOutcome::Deferred(format!(
            "hardware requirement {} has no trusted local capability inventory",
            request.requirement_id
        )));
    };
    let config = parse_config(&contents)?;
    if config.repository != source.repository {
        return Err(format!(
            "hardware capability inventory repository {} does not match exact dispatch repository {}",
            config.repository, source.repository
        ));
    }

    match config.mode {
        CapabilityMode::Manual => {
            let record = AuditRecord {
                request,
                source,
                config: &config,
                observed_at,
                outcome: "manual_deferred",
                observations: &[],
                query_errors: &[],
            };
            write_audit(&record)?;
            Ok(HardwareCapabilityOutcome::Deferred(format!(
                "hardware requirement {} is configured for manual scheduling and cannot be dispatched autonomously",
                request.requirement_id
            )))
        }
        CapabilityMode::Hosted => {
            let record = AuditRecord {
                request,
                source,
                config: &config,
                observed_at,
                outcome: "hosted_schedulable",
                observations: &[],
                query_errors: &[],
            };
            write_audit(&record)?;
            Ok(HardwareCapabilityOutcome::Schedulable)
        }
        CapabilityMode::SelfHostedRepository => {
            check_self_hosted(request, source, &config, observed_at, program)
        }
    }
}

fn check_self_hosted(
    request: &HardwareEvidenceRequest<'_>,
    source: &HardwareDispatchSource,
    config: &CapabilityConfig,
    observed_at: u64,
    program: &OsStr,
) -> Result<HardwareCapabilityOutcome, String> {
    let mut observations = Vec::new();
    let mut query_errors = Vec::new();

    for runner_name in &config.runners {
        match query_exact_runner(program, &source.repository, runner_name) {
            Ok(Some(observation)) => observations.push(observation),
            Ok(None) => {}
            Err(QueryError::Unavailable) => query_errors.push(runner_name.clone()),
            Err(QueryError::Malformed(message)) => return Err(message),
        }
    }

    let schedulable = observations.iter().any(|observation| {
        observation.status == "online"
            && config
                .required_labels
                .iter()
                .all(|required| observation.labels.iter().any(|label| label == required))
    });

    let outcome = if schedulable {
        "self_hosted_schedulable"
    } else if query_errors.is_empty() {
        "self_hosted_unavailable"
    } else {
        "self_hosted_query_incomplete"
    };
    let record = AuditRecord {
        request,
        source,
        config,
        observed_at,
        outcome,
        observations: &observations,
        query_errors: &query_errors,
    };
    write_audit(&record)?;

    if schedulable {
        return Ok(HardwareCapabilityOutcome::Schedulable);
    }
    if !query_errors.is_empty() {
        return Ok(HardwareCapabilityOutcome::Deferred(format!(
            "trusted runner availability for requirement {} could not be fully inspected; refusing dispatch",
            request.requirement_id
        )));
    }
    Ok(HardwareCapabilityOutcome::Deferred(format!(
        "no configured trusted self-hosted runner is currently online with all required scheduling labels for requirement {}",
        request.requirement_id
    )))
}

#[derive(Debug)]
enum QueryError {
    Unavailable,
    Malformed(String),
}

fn query_exact_runner(
    program: &OsStr,
    repository: &str,
    runner_name: &str,
) -> Result<Option<RunnerObservation>, QueryError> {
    let endpoint = format!("repos/{repository}/actions/runners?name={runner_name}&per_page=100");
    let output = Command::new(program)
        .arg("api")
        .arg(&endpoint)
        .arg("--jq")
        .arg(RUNNER_JQ)
        .output()
        .map_err(|_| QueryError::Unavailable)?;
    if !output.status.success() {
        return Err(QueryError::Unavailable);
    }
    if output.stdout.len() > MAX_QUERY_OUTPUT_BYTES {
        return Err(QueryError::Malformed(
            "hardware runner API output exceeds the bounded size limit".to_owned(),
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| QueryError::Malformed("hardware runner API output is not UTF-8".to_owned()))?;
    let observations = parse_runner_lines(&stdout).map_err(QueryError::Malformed)?;
    let exact = observations
        .into_iter()
        .filter(|observation| observation.name == runner_name)
        .collect::<Vec<_>>();
    match exact.len() {
        0 => Ok(None),
        1 => Ok(exact.into_iter().next()),
        count => Err(QueryError::Malformed(format!(
            "hardware runner API returned {count} ambiguous exact-name entries for {runner_name}"
        ))),
    }
}

fn parse_runner_lines(stdout: &str) -> Result<Vec<RunnerObservation>, String> {
    let mut observations = Vec::new();
    let mut line_count = 0usize;
    for line in stdout.lines().filter(|line| !line.is_empty()) {
        line_count += 1;
        if line_count > MAX_RUNNER_RESPONSE_LINES {
            return Err("hardware runner API returned too many runner entries".to_owned());
        }
        if line.len() > MAX_ENCODED_FIELD_CHARS * 3 {
            return Err("hardware runner API row exceeds bound".to_owned());
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 6 {
            return Err("malformed hardware runner API row".to_owned());
        }
        let id = fields[0]
            .parse::<u64>()
            .map_err(|_| "invalid hardware runner id".to_owned())?;
        if id == 0 {
            return Err("hardware runner id must be nonzero".to_owned());
        }
        let name = percent_decode(fields[1], "runner name")?;
        validate_observed_string("runner name", &name, MAX_TOKEN_CHARS)?;
        let os = percent_decode(fields[2], "runner os")?;
        validate_observed_string("runner os", &os, MAX_TOKEN_CHARS)?;
        let status = match fields[3] {
            "online" | "offline" => fields[3].to_owned(),
            _ => return Err("invalid hardware runner status".to_owned()),
        };
        let busy = match fields[4] {
            "true" => true,
            "false" => false,
            _ => return Err("invalid hardware runner busy flag".to_owned()),
        };
        if fields[5].len() > MAX_ENCODED_FIELD_CHARS {
            return Err("hardware runner encoded labels exceed bound".to_owned());
        }
        let labels = if fields[5].is_empty() {
            Vec::new()
        } else {
            let encoded = fields[5].split(',').collect::<Vec<_>>();
            if encoded.len() > MAX_LABELS {
                return Err("hardware runner has too many labels".to_owned());
            }
            let mut labels = Vec::with_capacity(encoded.len());
            for label in encoded {
                let decoded = percent_decode(label, "runner label")?;
                validate_observed_string("runner label", &decoded, MAX_LABEL_CHARS)?;
                labels.push(decoded);
            }
            labels
        };
        let audit_row = fields.join("|");
        if audit_row.chars().any(char::is_control) {
            return Err("hardware runner audit row contains control characters".to_owned());
        }
        observations.push(RunnerObservation {
            id,
            name,
            os,
            status,
            busy,
            labels,
            audit_row,
        });
    }
    Ok(observations)
}

fn percent_decode(value: &str, label: &str) -> Result<String, String> {
    if value.len() > MAX_ENCODED_FIELD_CHARS || !value.is_ascii() {
        return Err(format!("encoded hardware {label} is invalid or oversized"));
    }
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(format!("invalid percent encoding in hardware {label}"));
            }
            let high = hex_digit(bytes[index + 1])
                .ok_or_else(|| format!("invalid percent encoding in hardware {label}"))?;
            let low = hex_digit(bytes[index + 2])
                .ok_or_else(|| format!("invalid percent encoding in hardware {label}"))?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| format!("decoded hardware {label} is not UTF-8"))
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_config(contents: &str) -> Result<CapabilityConfig, String> {
    let fields = parse_fields(contents)?;
    let mode = required(&fields, "mode")?;
    let repository = required(&fields, "repository")?;
    validate_repository(repository)?;
    match mode {
        "manual" => {
            reject_exact_fields(&fields, &["mode", "repository"])?;
            Ok(CapabilityConfig {
                mode: CapabilityMode::Manual,
                repository: repository.to_owned(),
                runners: Vec::new(),
                required_labels: Vec::new(),
            })
        }
        "hosted" => {
            reject_exact_fields(&fields, &["mode", "repository"])?;
            Ok(CapabilityConfig {
                mode: CapabilityMode::Hosted,
                repository: repository.to_owned(),
                runners: Vec::new(),
                required_labels: Vec::new(),
            })
        }
        "self_hosted_repository" => {
            reject_exact_fields(&fields, &["mode", "repository", "runners", "labels"])?;
            let runners = parse_token_list(required(&fields, "runners")?, "runner", MAX_RUNNERS)?;
            let required_labels =
                parse_token_list(required(&fields, "labels")?, "label", MAX_LABELS)?;
            if !required_labels.iter().any(|label| label == "self-hosted") {
                return Err(
                    "self-hosted capability inventory must require the self-hosted label"
                        .to_owned(),
                );
            }
            Ok(CapabilityConfig {
                mode: CapabilityMode::SelfHostedRepository,
                repository: repository.to_owned(),
                runners,
                required_labels,
            })
        }
        other => Err(format!(
            "unsupported hardware capability scheduling mode: {other}"
        )),
    }
}

fn parse_fields(contents: &str) -> Result<BTreeMap<String, String>, String> {
    if contents.len() as u64 > MAX_CONFIG_BYTES || contents.as_bytes().contains(&0) {
        return Err("hardware capability inventory is oversized or contains NUL".to_owned());
    }
    let mut lines = contents.lines();
    if lines.next().unwrap_or_default() != CONFIG_VERSION {
        return Err("unsupported hardware capability inventory version".to_owned());
    }
    let mut fields = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            return Err("empty hardware capability inventory field".to_owned());
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| "malformed hardware capability inventory field".to_owned())?;
        if key.is_empty()
            || value.is_empty()
            || key.chars().any(char::is_control)
            || value.chars().any(char::is_control)
        {
            return Err("invalid hardware capability inventory field".to_owned());
        }
        if fields.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(format!(
                "duplicate hardware capability inventory field: {key}"
            ));
        }
    }
    Ok(fields)
}

fn reject_exact_fields(fields: &BTreeMap<String, String>, allowed: &[&str]) -> Result<(), String> {
    if let Some(unknown) = fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!(
            "unknown hardware capability inventory field: {unknown}"
        ));
    }
    if let Some(missing) = allowed.iter().find(|key| !fields.contains_key(**key)) {
        return Err(format!(
            "missing hardware capability inventory field: {missing}"
        ));
    }
    if fields.len() != allowed.len() {
        return Err("invalid hardware capability inventory field count".to_owned());
    }
    Ok(())
}

fn required<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    fields
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing hardware capability inventory field: {key}"))
}

fn parse_token_list(value: &str, label: &str, max_items: usize) -> Result<Vec<String>, String> {
    let parts = value.split(',').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > max_items {
        return Err(format!("invalid hardware capability {label} list length"));
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(parts.len());
    for part in parts {
        validate_config_token(label, part)?;
        if !seen.insert(part.to_owned()) {
            return Err(format!("duplicate hardware capability {label}: {part}"));
        }
        result.push(part.to_owned());
    }
    result.sort();
    Ok(result)
}

fn validate_config_token(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_TOKEN_CHARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("invalid hardware capability {label} token"));
    }
    Ok(())
}

fn validate_observed_string(label: &str, value: &str, max_chars: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max_chars || value.chars().any(char::is_control) {
        return Err(format!("invalid observed hardware {label}"));
    }
    Ok(())
}

fn write_audit(record: &AuditRecord<'_>) -> Result<PathBuf, String> {
    let state_root = record.request.data_root.join("state/hardware-capability");
    let directory = state_root
        .join(hex_component(record.request.repository))
        .join(format!("pr-{}", record.request.pr_number))
        .join(record.request.head_sha)
        .join(record.request.requirement_id);
    ensure_managed_directory(record.request.data_root, &state_root, &directory)?;

    let configured_runners = if record.config.runners.is_empty() {
        "-".to_owned()
    } else {
        record.config.runners.join(",")
    };
    let required_labels = if record.config.required_labels.is_empty() {
        "-".to_owned()
    } else {
        record.config.required_labels.join(",")
    };
    let runner_observations = if record.observations.is_empty() {
        "-".to_owned()
    } else {
        record
            .observations
            .iter()
            .map(|observation| observation.audit_row.as_str())
            .collect::<Vec<_>>()
            .join(";")
    };
    let query_errors = if record.query_errors.is_empty() {
        "-".to_owned()
    } else {
        record.query_errors.join(",")
    };
    let contents = format!(
        "{AUDIT_VERSION}\nrepository={}\npr_number={}\nhead_sha={}\nbase_sha={}\npolicy_identity={}\nrequirement_id={}\ndispatch_repository={}\ndispatch_workflow={}\ndispatch_ref={}\nmode={}\nconfigured_runners={}\nrequired_labels={}\nobserved_at={}\noutcome={}\nrunner_observations={}\nquery_errors={}\n",
        record.request.repository,
        record.request.pr_number,
        record.request.head_sha,
        record.request.base_sha,
        record.request.policy_identity,
        record.request.requirement_id,
        record.source.repository,
        record.source.workflow,
        record.source.ref_name,
        record.config.mode.as_str(),
        configured_runners,
        required_labels,
        record.observed_at,
        record.outcome,
        runner_observations,
        query_errors,
    );
    if contents.len() > MAX_AUDIT_BYTES || contents.chars().any(|ch| ch == '\0') {
        return Err("hardware capability audit record exceeds bound".to_owned());
    }

    for sequence in 0..128u32 {
        let path = directory.join(format!(
            "{}-{}-{sequence}.state",
            record.observed_at,
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(contents.as_bytes()).map_err(|error| {
                    format!("failed to write hardware capability audit record: {error}")
                })?;
                file.sync_all().map_err(|error| {
                    format!("failed to sync hardware capability audit record: {error}")
                })?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create hardware capability audit record: {error}"
                ));
            }
        }
    }
    Err("hardware capability audit sequence exhausted".to_owned())
}

fn ensure_managed_directory(
    data_root: &Path,
    state_root: &Path,
    directory: &Path,
) -> Result<(), String> {
    let canonical_data = fs::canonicalize(data_root)
        .map_err(|error| format!("failed to canonicalize orchestrator data root: {error}"))?;
    if !state_root.starts_with(data_root) || !directory.starts_with(state_root) {
        return Err("hardware capability audit path is outside orchestrator data root".to_owned());
    }
    let relative = directory
        .strip_prefix(data_root)
        .map_err(|_| "hardware capability audit path is outside data root".to_owned())?;
    let mut current = data_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err("hardware capability audit path contains non-normal component".to_owned());
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "hardware capability audit directory component must be a non-symlink directory: {}",
                        current.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    format!(
                        "failed to create hardware capability audit directory {}: {error}",
                        current.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect hardware capability audit directory {}: {error}",
                    current.display()
                ));
            }
        }
        let canonical_current = fs::canonicalize(&current).map_err(|error| {
            format!("failed to canonicalize hardware capability audit directory: {error}")
        })?;
        if !canonical_current.starts_with(&canonical_data) {
            return Err("hardware capability audit directory escapes data root".to_owned());
        }
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
        Err(error) => return Err(format!("failed to inspect {label}: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{label} must be a regular non-symlink file"));
    }
    if metadata.len() > max_bytes {
        return Err(format!("{label} exceeds {max_bytes} byte bound"));
    }
    let root_metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("failed to inspect {label} root: {error}"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(format!("{label} root must be a non-symlink directory"));
    }
    let canonical_data = fs::canonicalize(data_root)
        .map_err(|error| format!("failed to canonicalize data root for {label}: {error}"))?;
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| format!("failed to canonicalize {label} root: {error}"))?;
    let canonical_path = fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize {label}: {error}"))?;
    if !canonical_root.starts_with(&canonical_data) || !canonical_path.starts_with(&canonical_root)
    {
        return Err(format!("{label} escapes orchestrator-owned root"));
    }
    let contents =
        fs::read_to_string(path).map_err(|error| format!("failed to read {label}: {error}"))?;
    if contents.len() as u64 > max_bytes {
        return Err(format!("{label} exceeds bound after read"));
    }
    Ok(Some(contents))
}

fn validate_request(request: &HardwareEvidenceRequest<'_>) -> Result<(), String> {
    validate_repository(request.repository)?;
    if request.pr_number == 0 {
        return Err("hardware capability PR number must be nonzero".to_owned());
    }
    validate_git_digest("head SHA", request.head_sha)?;
    validate_git_digest("base SHA", request.base_sha)?;
    validate_hex(
        "policy identity",
        request.policy_identity,
        MAX_POLICY_IDENTITY_CHARS,
    )?;
    validate_config_token("requirement id", request.requirement_id)?;
    if request.requirement_id.len() > MAX_REQUIREMENT_ID_CHARS {
        return Err("hardware capability requirement id exceeds bound".to_owned());
    }
    Ok(())
}

fn validate_source(source: &HardwareDispatchSource) -> Result<(), String> {
    validate_repository(&source.repository)?;
    if source.workflow.is_empty()
        || source.workflow.len() > MAX_TOKEN_CHARS
        || source.workflow.chars().any(char::is_control)
        || source.ref_name.is_empty()
        || source.ref_name.len() > 256
        || source.ref_name.chars().any(char::is_control)
    {
        return Err("invalid hardware capability dispatch source".to_owned());
    }
    Ok(())
}

fn validate_repository(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_REPOSITORY_CHARS
        || value
            .bytes()
            .any(|byte| matches!(byte, b'\n' | b'\r' | 0 | b'\\'))
    {
        return Err("invalid hardware capability repository".to_owned());
    }
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        return Err("hardware capability repository must be owner/name".to_owned());
    }
    validate_config_token("repository owner", owner)?;
    validate_config_token("repository name", repository)
}

fn validate_git_digest(label: &str, value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid hardware capability {label}"));
    }
    Ok(())
}

fn validate_hex(label: &str, value: &str, max_chars: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_chars
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("invalid hardware capability {label}"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "orchestrator-hardware-capability-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn request<'a>(root: &'a Path) -> HardwareEvidenceRequest<'a> {
        HardwareEvidenceRequest {
            data_root: root,
            repository: "Memorithm/Test",
            pr_number: 56,
            head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            policy_identity: "abcd1234",
            requirement_id: "jetson-thor-real-device",
        }
    }

    fn source() -> HardwareDispatchSource {
        HardwareDispatchSource {
            repository: "Memorithm/hardware-ci".to_owned(),
            workflow: "dispatch.yml".to_owned(),
            ref_name: "main".to_owned(),
        }
    }

    fn write_inventory(root: &Path, contents: &str) {
        let directory = root.join("config/hardware-capabilities");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("jetson-thor-real-device.state"), contents).unwrap();
    }

    fn audit_files(root: &Path) -> Vec<PathBuf> {
        let directory = root
            .join("state/hardware-capability")
            .join(hex_component("Memorithm/Test"))
            .join("pr-56")
            .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .join("jetson-thor-real-device");
        if !directory.exists() {
            return Vec::new();
        }
        fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }

    #[test]
    fn hosted_inventory_is_schedulable_without_runner_query() {
        let root = temp_root("hosted");
        write_inventory(&root, "v1\nmode=hosted\nrepository=Memorithm/hardware-ci\n");
        let outcome = check_schedulable_with_program(
            &request(&root),
            &source(),
            100,
            OsStr::new("definitely-not-a-runner-query"),
        )
        .unwrap();
        assert_eq!(outcome, HardwareCapabilityOutcome::Schedulable);
        assert_eq!(audit_files(&root).len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manual_inventory_always_defers() {
        let root = temp_root("manual");
        write_inventory(&root, "v1\nmode=manual\nrepository=Memorithm/hardware-ci\n");
        let outcome =
            check_schedulable_with_program(&request(&root), &source(), 100, OsStr::new("unused"))
                .unwrap();
        assert!(
            matches!(outcome, HardwareCapabilityOutcome::Deferred(reason) if reason.contains("manual scheduling"))
        );
        assert_eq!(audit_files(&root).len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn online_busy_self_hosted_runner_with_required_labels_is_schedulable() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("online-busy");
        write_inventory(
            &root,
            "v1\nmode=self_hosted_repository\nrepository=Memorithm/hardware-ci\nrunners=tarek-scirust-arm64-01\nlabels=self-hosted,Linux,ARM64,jetson-thor,internal\n",
        );
        let fake = root.join("fake-gh");
        fs::write(
            &fake,
            "#!/bin/sh\nset -eu\ntest \"$1\" = api\nprintf '23\\ttarek-scirust-arm64-01\\tlinux\\tonline\\ttrue\\tself-hosted,Linux,ARM64,jetson-thor,internal\\n'\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake, permissions).unwrap();
        let outcome =
            check_schedulable_with_program(&request(&root), &source(), 100, fake.as_os_str())
                .unwrap();
        assert_eq!(outcome, HardwareCapabilityOutcome::Schedulable);
        let audit = fs::read_to_string(&audit_files(&root)[0]).unwrap();
        assert!(audit.contains("outcome=self_hosted_schedulable"));
        assert!(audit.contains("online|true"));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn offline_or_missing_required_label_defers() {
        use std::os::unix::fs::PermissionsExt;

        for (label, row) in [
            (
                "offline",
                "23\ttarek-scirust-arm64-01\tlinux\toffline\tfalse\tself-hosted,Linux,ARM64,jetson-thor,internal\n",
            ),
            (
                "missing-label",
                "23\ttarek-scirust-arm64-01\tlinux\tonline\tfalse\tself-hosted,Linux,ARM64,jetson-thor\n",
            ),
        ] {
            let root = temp_root(label);
            write_inventory(
                &root,
                "v1\nmode=self_hosted_repository\nrepository=Memorithm/hardware-ci\nrunners=tarek-scirust-arm64-01\nlabels=self-hosted,Linux,ARM64,jetson-thor,internal\n",
            );
            let fake = root.join("fake-gh");
            fs::write(
                &fake,
                format!(
                    "#!/bin/sh\nset -eu\nprintf '{}'\n",
                    row.replace('\\', "\\\\")
                        .replace('\n', "\\n")
                        .replace('\t', "\\t")
                ),
            )
            .unwrap();
            let mut permissions = fs::metadata(&fake).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&fake, permissions).unwrap();
            let outcome =
                check_schedulable_with_program(&request(&root), &source(), 100, fake.as_os_str())
                    .unwrap();
            assert!(matches!(outcome, HardwareCapabilityOutcome::Deferred(_)));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn runner_query_failure_defers_without_claiming_absence() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("query-failure");
        write_inventory(
            &root,
            "v1\nmode=self_hosted_repository\nrepository=Memorithm/hardware-ci\nrunners=tarek-scirust-arm64-01\nlabels=self-hosted,Linux,ARM64\n",
        );
        let fake = root.join("fake-gh");
        fs::write(&fake, "#!/bin/sh\nexit 1\n").unwrap();
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake, permissions).unwrap();
        let outcome =
            check_schedulable_with_program(&request(&root), &source(), 100, fake.as_os_str())
                .unwrap();
        assert!(
            matches!(outcome, HardwareCapabilityOutcome::Deferred(reason) if reason.contains("could not be fully inspected"))
        );
        let audit = fs::read_to_string(&audit_files(&root)[0]).unwrap();
        assert!(audit.contains("outcome=self_hosted_query_incomplete"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repository_mismatch_and_unknown_fields_fail_closed() {
        let root = temp_root("mismatch");
        write_inventory(&root, "v1\nmode=hosted\nrepository=Memorithm/other\n");
        let error =
            check_schedulable_with_program(&request(&root), &source(), 100, OsStr::new("unused"))
                .unwrap_err();
        assert!(error.contains("does not match exact dispatch repository"));

        write_inventory(
            &root,
            "v1\nmode=hosted\nrepository=Memorithm/hardware-ci\nextra=bad\n",
        );
        assert!(
            check_schedulable_with_program(&request(&root), &source(), 101, OsStr::new("unused"),)
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_inventory_fails_closed() {
        use std::os::unix::fs::symlink;

        let root = temp_root("symlink");
        let outside = temp_root("outside");
        fs::create_dir_all(root.join("config")).unwrap();
        symlink(&outside, root.join("config/hardware-capabilities")).unwrap();
        fs::write(
            outside.join("jetson-thor-real-device.state"),
            "v1\nmode=hosted\nrepository=Memorithm/hardware-ci\n",
        )
        .unwrap();
        let error =
            check_schedulable_with_program(&request(&root), &source(), 100, OsStr::new("unused"))
                .unwrap_err();
        assert!(error.contains("root must be a non-symlink directory"));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stale_audit_never_replaces_fresh_runner_query() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("fresh-query");
        write_inventory(
            &root,
            "v1\nmode=self_hosted_repository\nrepository=Memorithm/hardware-ci\nrunners=tarek-scirust-arm64-01\nlabels=self-hosted,Linux,ARM64\n",
        );
        let fake = root.join("fake-gh");
        fs::write(
            &fake,
            "#!/bin/sh\nprintf '23\\ttarek-scirust-arm64-01\\tlinux\\tonline\\tfalse\\tself-hosted,Linux,ARM64\\n'\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake, permissions).unwrap();
        assert_eq!(
            check_schedulable_with_program(&request(&root), &source(), 100, fake.as_os_str(),)
                .unwrap(),
            HardwareCapabilityOutcome::Schedulable
        );
        fs::write(
            &fake,
            "#!/bin/sh\nprintf '23\\ttarek-scirust-arm64-01\\tlinux\\toffline\\tfalse\\tself-hosted,Linux,ARM64\\n'\n",
        )
        .unwrap();
        let second =
            check_schedulable_with_program(&request(&root), &source(), 101, fake.as_os_str())
                .unwrap();
        assert!(matches!(second, HardwareCapabilityOutcome::Deferred(_)));
        assert_eq!(audit_files(&root).len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runner_parser_rejects_ambiguous_or_malformed_encoded_metadata() {
        let decoded = parse_runner_lines(
            "23\ttarek-scirust-arm64-01\tlinux\tonline\ttrue\tself-hosted,Linux,ARM64\n",
        )
        .unwrap();
        assert_eq!(decoded[0].id, 23);
        assert!(decoded[0].busy);
        assert_eq!(decoded[0].labels, ["self-hosted", "Linux", "ARM64"]);
        assert!(
            percent_decode("jetson%2Dthor", "fixture")
                .unwrap()
                .contains("jetson-thor")
        );
        assert!(percent_decode("%ZZ", "fixture").is_err());
    }
}
