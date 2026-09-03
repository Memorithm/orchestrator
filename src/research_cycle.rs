//! Durable, bounded state for explicitly opted-in autonomous research cycles.
//!
//! A cycle record is intentionally an agent report, not authoritative executed
//! evidence. Repository validation, CI, hardware evidence, publication and merge
//! gates remain owned by the Orchestrator parent. Persisted reports are carried
//! into later cycles only as bounded context.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const STATE_VERSION: &str = "research-cycle-v1";
pub const HANDOFF_FILE: &str = ".orchestrator-research-cycle-v1";
const MAX_TEXT_BYTES: usize = 8 * 1024;
const MAX_REPOSITORY_BYTES: usize = 256;
const MAX_PROGRAMME_BYTES: usize = 128;
const MAX_BRANCH_BYTES: usize = 256;
const MAX_LINE_ID_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchDecision {
    Continue,
    Revise,
    Abandon,
    Blocked,
}

impl ResearchDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Revise => "revise",
            Self::Abandon => "abandon",
            Self::Blocked => "blocked",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "continue" => Ok(Self::Continue),
            "revise" => Ok(Self::Revise),
            "abandon" => Ok(Self::Abandon),
            "blocked" => Ok(Self::Blocked),
            other => Err(format!("unknown research decision: {other:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchCycleReport {
    /// Stable, canonical identity for the current research line. `None` exists
    /// only for persisted pre-ORCH9f state; new handoffs require an identifier.
    pub line_id: Option<String>,
    pub hypothesis: String,
    pub experiment: String,
    pub evidence_report: String,
    pub decision: ResearchDecision,
    pub next_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchCycleRecord {
    pub repository: String,
    pub issue_number: u64,
    pub programme: Option<String>,
    pub managed_branch: String,
    pub sequence: u64,
    pub recorded_at: u64,
    pub worker_exit_code: i32,
    pub report: ResearchCycleReport,
}

impl ResearchCycleRecord {
    /// Context for the next research worker. This wording is deliberately
    /// explicit that the stored evidence field is not parent validation.
    #[must_use]
    pub fn continuation_context(&self) -> String {
        format!(
            "PRIOR RESEARCH CYCLE (UNVERIFIED AGENT REPORT)\nSequence: {}\nResearch line ID: {}\nHypothesis: {}\nExperiment: {}\nAgent evidence report: {}\nDecision: {}\nNext action: {}\nParent validation status: UNVERIFIED — this record does not prove that the experiment, benchmark, CI, hardware run, or claimed result succeeded.",
            self.sequence,
            self.report.line_id.as_deref().unwrap_or("(legacy-unbound)"),
            self.report.hypothesis,
            self.report.experiment,
            self.report.evidence_report,
            self.report.decision.as_str(),
            self.report.next_action
        )
    }
}

#[derive(Debug, Clone)]
pub struct ResearchCycleStore {
    root: PathBuf,
}

impl ResearchCycleStore {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn load_latest(
        &self,
        repository: &str,
        issue_number: u64,
        programme: Option<&str>,
    ) -> Result<Option<ResearchCycleRecord>, String> {
        validate_repository(repository)?;
        validate_issue_number(issue_number)?;
        validate_optional_programme(programme)?;
        let programme_root = self.programme_root(repository, issue_number, programme);
        let latest = load_optional_state(&programme_root.join("latest.state"))?;
        let history_candidate = highest_history_candidate(&programme_root.join("history"))?;
        let history = match history_candidate {
            None => None,
            Some((filename_sequence, path)) => match load_optional_state(&path) {
                Ok(Some(record)) => {
                    if record.sequence != filename_sequence {
                        return Err(format!(
                            "research cycle history filename/state sequence mismatch: {}",
                            path.display()
                        ));
                    }
                    Some(record)
                }
                Ok(None) => {
                    return Err(format!(
                        "research cycle history disappeared: {}",
                        path.display()
                    ));
                }
                Err(error) => {
                    let expected_next = latest
                        .as_ref()
                        .map_or(1, |record| record.sequence.saturating_add(1));
                    if filename_sequence != expected_next {
                        return Err(error);
                    }
                    fs::remove_file(&path).map_err(|remove_error| {
                        format!(
                            "failed to remove incomplete research cycle history {} after {error}: {remove_error}",
                            path.display()
                        )
                    })?;
                    return self.load_latest(repository, issue_number, programme);
                }
            },
        };

        match (latest, history) {
            (None, None) => Ok(None),
            (None, Some(history)) => {
                validate_record_identity(&history, repository, issue_number, programme)?;
                Ok(Some(history))
            }
            (Some(_), None) => Err(format!(
                "research cycle latest state exists without append-only history in {}",
                programme_root.display()
            )),
            (Some(latest), Some(history)) => {
                validate_record_identity(&latest, repository, issue_number, programme)?;
                validate_record_identity(&history, repository, issue_number, programme)?;
                if history.sequence < latest.sequence {
                    return Err(format!(
                        "research cycle history is behind latest state in {}",
                        programme_root.display()
                    ));
                }
                if history.sequence == latest.sequence && history != latest {
                    return Err(format!(
                        "research cycle latest/history mismatch in {}",
                        programme_root.display()
                    ));
                }
                Ok(Some(if history.sequence > latest.sequence {
                    history
                } else {
                    latest
                }))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn append(
        &self,
        repository: &str,
        issue_number: u64,
        programme: Option<&str>,
        managed_branch: &str,
        recorded_at: u64,
        worker_exit_code: i32,
        report: ResearchCycleReport,
    ) -> Result<ResearchCycleRecord, String> {
        validate_repository(repository)?;
        validate_issue_number(issue_number)?;
        validate_optional_programme(programme)?;
        validate_managed_branch(managed_branch, issue_number)?;
        validate_worker_exit_code(worker_exit_code)?;
        validate_new_report(&report)?;

        self.ensure_directories(repository, issue_number, programme)?;
        let previous = self.load_latest(repository, issue_number, programme)?;
        let sequence = match previous {
            Some(ref record) => record
                .sequence
                .checked_add(1)
                .ok_or_else(|| "research cycle sequence exhausted".to_owned())?,
            None => 1,
        };

        let record = ResearchCycleRecord {
            repository: repository.to_owned(),
            issue_number,
            programme: programme.map(ToOwned::to_owned),
            managed_branch: managed_branch.to_owned(),
            sequence,
            recorded_at,
            worker_exit_code,
            report,
        };
        validate_record(&record)?;

        let programme_root = self.programme_root(repository, issue_number, programme);
        let history_root = programme_root.join("history");
        let serialized = serialize_state(&record);
        let history_path = history_root.join(format!("{sequence:020}.state"));
        let mut history = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&history_path)
            .map_err(|error| {
                format!(
                    "failed to create append-only research cycle {}: {error}",
                    history_path.display()
                )
            })?;
        if let Err(error) = history.write_all(serialized.as_bytes()) {
            drop(history);
            let _ = fs::remove_file(&history_path);
            return Err(format!(
                "failed to write {}: {error}",
                history_path.display()
            ));
        }
        if let Err(error) = history.sync_all() {
            drop(history);
            let _ = fs::remove_file(&history_path);
            return Err(format!(
                "failed to sync {}: {error}",
                history_path.display()
            ));
        }

        write_latest_atomically(&programme_root, sequence, &serialized)?;
        Ok(record)
    }

    fn ensure_directories(
        &self,
        repository: &str,
        issue_number: u64,
        programme: Option<&str>,
    ) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|error| {
            format!(
                "failed to create research cycle root {}: {error}",
                self.root.display()
            )
        })?;
        require_real_directory(&self.root)?;

        let repository_root = self.root.join(repository.replace('/', "__"));
        ensure_child_directory(&repository_root)?;
        let issue_root = self.issue_root(repository, issue_number);
        ensure_child_directory(&issue_root)?;
        let programme_root = self.programme_root(repository, issue_number, programme);
        ensure_child_directory(&programme_root)?;
        ensure_child_directory(&programme_root.join("history"))
    }

    fn issue_root(&self, repository: &str, issue_number: u64) -> PathBuf {
        self.root
            .join(repository.replace('/', "__"))
            .join(format!("issue-{issue_number}"))
    }

    fn programme_root(
        &self,
        repository: &str,
        issue_number: u64,
        programme: Option<&str>,
    ) -> PathBuf {
        self.issue_root(repository, issue_number)
            .join(programme_component(programme))
    }
}

pub fn parse_handoff(contents: &str) -> Result<ResearchCycleReport, String> {
    if contents.len() > MAX_TEXT_BYTES.saturating_mul(5).saturating_add(2048) {
        return Err("research cycle handoff exceeds bounded size".to_owned());
    }
    let mut lines = contents.lines();
    if lines.next() != Some(STATE_VERSION) {
        return Err("unsupported or missing research cycle handoff version".to_owned());
    }

    let mut seen = BTreeSet::new();
    let mut line_id = None;
    let mut hypothesis = None;
    let mut experiment = None;
    let mut evidence_report = None;
    let mut decision = None;
    let mut next_action = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed research cycle handoff field: {line:?}"))?;
        if !seen.insert(name.to_owned()) {
            return Err(format!("duplicate research cycle handoff field: {name}"));
        }
        match name {
            "line_id" => {
                validate_line_id(value)?;
                line_id = Some(value.to_owned());
            }
            "hypothesis" => {
                validate_bounded_line(name, value, MAX_TEXT_BYTES)?;
                hypothesis = Some(value.to_owned());
            }
            "experiment" => {
                validate_bounded_line(name, value, MAX_TEXT_BYTES)?;
                experiment = Some(value.to_owned());
            }
            "evidence_report" => {
                validate_bounded_line(name, value, MAX_TEXT_BYTES)?;
                evidence_report = Some(value.to_owned());
            }
            "decision" => decision = Some(ResearchDecision::parse(value)?),
            "next_action" => {
                validate_bounded_line(name, value, MAX_TEXT_BYTES)?;
                next_action = Some(value.to_owned());
            }
            other => return Err(format!("unknown research cycle handoff field: {other}")),
        }
    }

    let report = ResearchCycleReport {
        line_id: Some(line_id.ok_or_else(|| "research cycle handoff missing line_id".to_owned())?),
        hypothesis: hypothesis
            .ok_or_else(|| "research cycle handoff missing hypothesis".to_owned())?,
        experiment: experiment
            .ok_or_else(|| "research cycle handoff missing experiment".to_owned())?,
        evidence_report: evidence_report
            .ok_or_else(|| "research cycle handoff missing evidence_report".to_owned())?,
        decision: decision.ok_or_else(|| "research cycle handoff missing decision".to_owned())?,
        next_action: next_action
            .ok_or_else(|| "research cycle handoff missing next_action".to_owned())?,
    };
    validate_new_report(&report)?;
    Ok(report)
}

#[must_use]
pub fn handoff_contract() -> String {
    format!(
        "RESEARCH CYCLE HANDOFF (REQUIRED FOR THIS EXPLICIT RESEARCH MODE)\nBefore finishing, write exactly one parent-only machine handoff file named `{HANDOFF_FILE}` at the repository root. This is the sole exception to the generic rule against status-report files; it is not a product artifact and Orchestrator removes it before diff validation or publication. Use exactly this single-line format (one field per line, no Markdown):\n{STATE_VERSION}\nline_id=<stable canonical identifier for the current research line: ASCII alphanumeric plus ._- only, max {MAX_LINE_ID_BYTES} bytes>\nhypothesis=<current falsifiable hypothesis, one line>\nexperiment=<bounded experiment/control/ablation actually attempted this cycle, one line>\nevidence_report=<what you observed; state negative, null, inconclusive, blocked, or failed results explicitly, one line>\ndecision=<continue|revise|abandon|blocked>\nnext_action=<best next permitted research action, one line>\nKeep the same line_id while continuing, revising, or reporting a blocker on the same research line. Mint a new line_id only when selecting a materially different line. An abandon decision names the line being abandoned. The line_id and evidence_report remain unverified agent report fields: neither is authoritative CI, hardware, benchmark, validation evidence, or permission to cross a parent gate."
    )
}

fn validate_record(record: &ResearchCycleRecord) -> Result<(), String> {
    validate_repository(&record.repository)?;
    validate_issue_number(record.issue_number)?;
    validate_optional_programme(record.programme.as_deref())?;
    validate_managed_branch(&record.managed_branch, record.issue_number)?;
    validate_worker_exit_code(record.worker_exit_code)?;
    if record.sequence == 0 {
        return Err("research cycle sequence must be positive".to_owned());
    }
    validate_report(&record.report)
}

fn validate_record_identity(
    record: &ResearchCycleRecord,
    repository: &str,
    issue_number: u64,
    programme: Option<&str>,
) -> Result<(), String> {
    validate_record(record)?;
    if record.repository != repository
        || record.issue_number != issue_number
        || record.programme.as_deref() != programme
    {
        return Err("research cycle state identity mismatch".to_owned());
    }
    Ok(())
}

fn validate_new_report(report: &ResearchCycleReport) -> Result<(), String> {
    if report.line_id.is_none() {
        return Err("new research cycle report requires line_id".to_owned());
    }
    validate_report(report)
}

fn validate_report(report: &ResearchCycleReport) -> Result<(), String> {
    if let Some(line_id) = report.line_id.as_deref() {
        validate_line_id(line_id)?;
    }
    validate_bounded_line("hypothesis", &report.hypothesis, MAX_TEXT_BYTES)?;
    validate_bounded_line("experiment", &report.experiment, MAX_TEXT_BYTES)?;
    validate_bounded_line("evidence_report", &report.evidence_report, MAX_TEXT_BYTES)?;
    validate_bounded_line("next_action", &report.next_action, MAX_TEXT_BYTES)
}

fn validate_line_id(value: &str) -> Result<(), String> {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err("invalid research line identifier".to_owned());
    };
    if value.len() > MAX_LINE_ID_BYTES
        || !first.is_ascii_alphanumeric()
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid research line identifier".to_owned());
    }
    Ok(())
}

fn validate_repository(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_REPOSITORY_BYTES
        || value.matches('/').count() != 1
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
    {
        return Err("invalid research cycle repository identity".to_owned());
    }
    Ok(())
}

fn validate_issue_number(issue_number: u64) -> Result<(), String> {
    if issue_number == 0 {
        return Err("research cycle issue number must be positive".to_owned());
    }
    Ok(())
}

fn validate_optional_programme(value: Option<&str>) -> Result<(), String> {
    if let Some(value) = value
        && (value.is_empty()
            || value.len() > MAX_PROGRAMME_BYTES
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/')
            }))
    {
        return Err("invalid research cycle programme identifier".to_owned());
    }
    Ok(())
}

fn programme_component(programme: Option<&str>) -> String {
    programme.map_or_else(
        || "programme-unspecified".to_owned(),
        |value| format!("programme-{}", hex_encode(value)),
    )
}

fn validate_managed_branch(branch: &str, issue_number: u64) -> Result<(), String> {
    validate_bounded_line("managed branch", branch, MAX_BRANCH_BYTES)?;
    let prefix = format!("orchestrator/issue-{issue_number}-");
    let timestamp = branch
        .strip_prefix(&prefix)
        .ok_or_else(|| "research cycle managed branch does not match issue identity".to_owned())?;
    if timestamp.is_empty() || !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("research cycle managed branch has invalid timestamp".to_owned());
    }
    Ok(())
}

fn validate_worker_exit_code(exit_code: i32) -> Result<(), String> {
    if !(0..=255).contains(&exit_code) {
        return Err("research cycle worker exit code is outside shell status range".to_owned());
    }
    Ok(())
}

fn validate_bounded_line(name: &str, value: &str, maximum: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(format!("invalid or oversized research cycle {name}"));
    }
    Ok(())
}

fn ensure_child_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "research cycle state directory is not a real directory: {}",
                    path.display()
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(path)
            .map_err(|error| format!("failed to create {}: {error}", path.display())),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

fn require_real_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "research cycle state root is not a real directory: {}",
            path.display()
        ));
    }
    Ok(())
}

fn load_optional_state(path: &Path) -> Result<Option<ResearchCycleRecord>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "research cycle state is not a regular file: {}",
            path.display()
        ));
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read research cycle {}: {error}", path.display()))?;
    parse_state(&contents)
        .map(Some)
        .map_err(|error| format!("invalid research cycle {}: {error}", path.display()))
}

fn highest_history_candidate(path: &Path) -> Result<Option<(u64, PathBuf)>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "research cycle history is not a real directory: {}",
            path.display()
        ));
    }

    let mut highest: Option<(u64, PathBuf)> = None;
    for entry in
        fs::read_dir(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read history entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect history entry: {error}"))?;
        if !file_type.is_file() || file_type.is_symlink() {
            return Err(format!(
                "unexpected non-file research cycle history entry: {}",
                entry.path().display()
            ));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "research cycle history filename is not UTF-8".to_owned())?;
        let sequence_text = name
            .strip_suffix(".state")
            .ok_or_else(|| format!("unexpected research cycle history filename: {name:?}"))?;
        if sequence_text.len() != 20 || !sequence_text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("invalid research cycle history filename: {name:?}"));
        }
        let sequence = sequence_text
            .parse::<u64>()
            .map_err(|error| format!("invalid history sequence {sequence_text}: {error}"))?;
        if highest
            .as_ref()
            .is_none_or(|(current, _)| sequence > *current)
        {
            highest = Some((sequence, entry.path()));
        }
    }
    Ok(highest)
}

fn write_latest_atomically(
    programme_root: &Path,
    sequence: u64,
    serialized: &str,
) -> Result<(), String> {
    let latest = programme_root.join("latest.state");
    if let Ok(metadata) = fs::symlink_metadata(&latest)
        && metadata.file_type().is_symlink()
    {
        return Err(format!(
            "research cycle latest state is a symlink: {}",
            latest.display()
        ));
    }
    let temporary = programme_root.join(format!(
        "latest.state.tmp.{}.{sequence}",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("failed to create {}: {error}", temporary.display()))?;
    file.write_all(serialized.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &latest).map_err(|error| {
        format!(
            "failed to atomically replace research cycle {} with {}: {error}",
            latest.display(),
            temporary.display()
        )
    })
}

fn hex_encode(value: &str) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(value.len().saturating_mul(2));
    for byte in value.as_bytes() {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn hex_decode(name: &str, value: &str, maximum: usize) -> Result<String, String> {
    if !value.len().is_multiple_of(2) || value.len() > maximum.saturating_mul(2) {
        return Err(format!("invalid encoded research cycle {name}"));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        let text = std::str::from_utf8(pair)
            .map_err(|_| format!("invalid encoded research cycle {name}"))?;
        let byte = u8::from_str_radix(text, 16)
            .map_err(|_| format!("invalid encoded research cycle {name}"))?;
        bytes.push(byte);
    }
    let decoded =
        String::from_utf8(bytes).map_err(|_| format!("invalid UTF-8 research cycle {name}"))?;
    validate_bounded_line(name, &decoded, maximum)?;
    Ok(decoded)
}

fn serialize_state(record: &ResearchCycleRecord) -> String {
    format!(
        "{STATE_VERSION}\nrepository_hex={}\nissue_number={}\nprogramme_hex={}\nmanaged_branch_hex={}\nsequence={}\nrecorded_at={}\nworker_exit_code={}\nevidence_authority=agent_report_unverified\nparent_validation=unverified\nline_id_hex={}\nhypothesis_hex={}\nexperiment_hex={}\nevidence_report_hex={}\ndecision={}\nnext_action_hex={}\n",
        hex_encode(&record.repository),
        record.issue_number,
        record
            .programme
            .as_deref()
            .map_or_else(String::new, hex_encode),
        hex_encode(&record.managed_branch),
        record.sequence,
        record.recorded_at,
        record.worker_exit_code,
        record
            .report
            .line_id
            .as_deref()
            .map_or_else(String::new, hex_encode),
        hex_encode(&record.report.hypothesis),
        hex_encode(&record.report.experiment),
        hex_encode(&record.report.evidence_report),
        record.report.decision.as_str(),
        hex_encode(&record.report.next_action)
    )
}

fn parse_state(contents: &str) -> Result<ResearchCycleRecord, String> {
    let mut lines = contents.lines();
    if lines.next() != Some(STATE_VERSION) {
        return Err("unsupported research cycle state version".to_owned());
    }
    let mut fields = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed research cycle state field: {line:?}"))?;
        if fields.insert(name.to_owned(), value.to_owned()).is_some() {
            return Err(format!("duplicate research cycle state field: {name}"));
        }
    }
    let allowed = [
        "repository_hex",
        "issue_number",
        "programme_hex",
        "managed_branch_hex",
        "sequence",
        "recorded_at",
        "worker_exit_code",
        "evidence_authority",
        "parent_validation",
        "line_id_hex",
        "hypothesis_hex",
        "experiment_hex",
        "evidence_report_hex",
        "decision",
        "next_action_hex",
    ];
    for name in fields.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(format!("unknown research cycle state field: {name}"));
        }
    }
    let take = |name: &str| {
        fields
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("research cycle state missing {name}"))
    };
    if take("evidence_authority")? != "agent_report_unverified"
        || take("parent_validation")? != "unverified"
    {
        return Err(
            "research cycle state attempted to claim unsupported evidence authority".to_owned(),
        );
    }

    let repository = hex_decode("repository", take("repository_hex")?, MAX_REPOSITORY_BYTES)?;
    let issue_number = take("issue_number")?
        .parse::<u64>()
        .map_err(|error| format!("invalid research cycle issue number: {error}"))?;
    let programme_hex = take("programme_hex")?;
    let programme = if programme_hex.is_empty() {
        None
    } else {
        Some(hex_decode("programme", programme_hex, MAX_PROGRAMME_BYTES)?)
    };
    let line_id = match fields.get("line_id_hex").map(String::as_str) {
        None => None,
        Some("") => {
            return Err("research cycle state contains explicit empty line_id_hex".to_owned());
        }
        Some(encoded) => {
            let decoded = hex_decode("line_id", encoded, MAX_LINE_ID_BYTES)?;
            validate_line_id(&decoded)?;
            Some(decoded)
        }
    };
    let record = ResearchCycleRecord {
        repository,
        issue_number,
        programme,
        managed_branch: hex_decode(
            "managed_branch",
            take("managed_branch_hex")?,
            MAX_BRANCH_BYTES,
        )?,
        sequence: take("sequence")?
            .parse()
            .map_err(|error| format!("invalid research cycle sequence: {error}"))?,
        recorded_at: take("recorded_at")?
            .parse()
            .map_err(|error| format!("invalid research cycle recorded_at: {error}"))?,
        worker_exit_code: take("worker_exit_code")?
            .parse()
            .map_err(|error| format!("invalid research cycle worker_exit_code: {error}"))?,
        report: ResearchCycleReport {
            line_id,
            hypothesis: hex_decode("hypothesis", take("hypothesis_hex")?, MAX_TEXT_BYTES)?,
            experiment: hex_decode("experiment", take("experiment_hex")?, MAX_TEXT_BYTES)?,
            evidence_report: hex_decode(
                "evidence_report",
                take("evidence_report_hex")?,
                MAX_TEXT_BYTES,
            )?,
            decision: ResearchDecision::parse(take("decision")?)?,
            next_action: hex_decode("next_action", take("next_action_hex")?, MAX_TEXT_BYTES)?,
        },
    };
    validate_record(&record)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "orchestrator-research-cycle-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn report(decision: ResearchDecision) -> ResearchCycleReport {
        ResearchCycleReport {
            line_id: Some("retrieval-fusion-a".to_owned()),
            hypothesis: "bounded state improves recall".to_owned(),
            experiment: "run deterministic recall ablation".to_owned(),
            evidence_report: "null result on synthetic case".to_owned(),
            decision,
            next_action: "revise the retrieval fusion control".to_owned(),
        }
    }

    #[test]
    fn handoff_parser_requires_canonical_line_identity_and_preserves_negative_evidence() {
        let parsed = parse_handoff(
            "research-cycle-v1\nline_id=recall-a\nhypothesis=H1\nexperiment=control A\nevidence_report=null result\ndecision=revise\nnext_action=control B\n",
        )
        .unwrap();
        assert_eq!(parsed.line_id.as_deref(), Some("recall-a"));
        assert_eq!(parsed.decision, ResearchDecision::Revise);
        assert_eq!(parsed.evidence_report, "null result");
        assert!(parse_handoff("research-cycle-v1\nhypothesis=H1\n").is_err());
        assert!(
            parse_handoff("research-cycle-v1\nline_id=bad/line\nhypothesis=H1\nexperiment=x\nevidence_report=y\ndecision=continue\nnext_action=z\n")
                .is_err()
        );
        assert!(
            parse_handoff("research-cycle-v1\nline_id=recall-a\nhypothesis=H1\nhypothesis=H2\nexperiment=x\nevidence_report=y\ndecision=continue\nnext_action=z\n")
                .is_err()
        );
        assert!(
            parse_handoff("research-cycle-v1\nline_id=recall-a\nhypothesis=H1\nexperiment=x\nevidence_report=y\ndecision=win\nnext_action=z\n")
                .is_err()
        );
    }

    #[test]
    fn store_is_append_only_and_latest_round_trips_line_identity() {
        let root = temporary_root("roundtrip");
        let store = ResearchCycleStore::new(root.clone());
        let first = store
            .append(
                "Memorithm/TDI",
                87,
                Some("TDI-8"),
                "orchestrator/issue-87-100",
                100,
                0,
                report(ResearchDecision::Revise),
            )
            .unwrap();
        let second = store
            .append(
                "Memorithm/TDI",
                87,
                Some("TDI-8"),
                "orchestrator/issue-87-200",
                200,
                0,
                report(ResearchDecision::Continue),
            )
            .unwrap();
        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, 2);
        assert_eq!(second.report.line_id.as_deref(), Some("retrieval-fusion-a"));
        assert_eq!(
            store
                .load_latest("Memorithm/TDI", 87, Some("TDI-8"))
                .unwrap(),
            Some(second)
        );
        let history = root.join("Memorithm__TDI/issue-87/programme-5444492d38/history");
        assert_eq!(fs::read_dir(history).unwrap().count(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_state_without_line_id_remains_loadable_as_unbound() {
        let mut record = ResearchCycleRecord {
            repository: "Memorithm/ADA".to_owned(),
            issue_number: 9,
            programme: Some("ADA-R".to_owned()),
            managed_branch: "orchestrator/issue-9-100".to_owned(),
            sequence: 1,
            recorded_at: 100,
            worker_exit_code: 0,
            report: report(ResearchDecision::Continue),
        };
        let serialized = serialize_state(&record);
        let legacy = serialized
            .lines()
            .filter(|line| !line.starts_with("line_id_hex="))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        record.report.line_id = None;
        assert_eq!(parse_state(&legacy).unwrap(), record);
        assert!(
            record
                .continuation_context()
                .contains("Research line ID: (legacy-unbound)")
        );
    }

    #[test]
    fn new_append_requires_line_identity_and_explicit_empty_state_is_rejected() {
        let root = temporary_root("line-id-required");
        let store = ResearchCycleStore::new(root.clone());
        let mut missing = report(ResearchDecision::Continue);
        missing.line_id = None;
        let error = store
            .append(
                "Memorithm/ADA",
                9,
                None,
                "orchestrator/issue-9-100",
                100,
                0,
                missing,
            )
            .unwrap_err();
        assert!(error.contains("requires line_id"));
        assert!(!root.exists());

        let record = ResearchCycleRecord {
            repository: "Memorithm/ADA".to_owned(),
            issue_number: 9,
            programme: None,
            managed_branch: "orchestrator/issue-9-100".to_owned(),
            sequence: 1,
            recorded_at: 100,
            worker_exit_code: 0,
            report: report(ResearchDecision::Continue),
        };
        let serialized = serialize_state(&record);
        let explicit_empty = serialized
            .lines()
            .map(|line| {
                if line.starts_with("line_id_hex=") {
                    "line_id_hex="
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let error = parse_state(&explicit_empty).unwrap_err();
        assert!(error.contains("explicit empty line_id_hex"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn programmes_have_independent_cycle_sequences() {
        let root = temporary_root("programmes");
        let store = ResearchCycleStore::new(root.clone());
        store
            .append(
                "Memorithm/ADA",
                9,
                Some("programme-a"),
                "orchestrator/issue-9-100",
                100,
                0,
                report(ResearchDecision::Continue),
            )
            .unwrap();
        assert!(
            store
                .load_latest("Memorithm/ADA", 9, Some("programme-b"))
                .unwrap()
                .is_none()
        );
        let first_b = store
            .append(
                "Memorithm/ADA",
                9,
                Some("programme-b"),
                "orchestrator/issue-9-200",
                200,
                0,
                report(ResearchDecision::Revise),
            )
            .unwrap();
        assert_eq!(first_b.sequence, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn complete_history_recovers_interrupted_latest_update() {
        let root = temporary_root("recover-complete");
        let store = ResearchCycleStore::new(root.clone());
        let first = store
            .append(
                "Memorithm/ADA",
                9,
                Some("ADA-R"),
                "orchestrator/issue-9-100",
                100,
                0,
                report(ResearchDecision::Continue),
            )
            .unwrap();
        let history_path = root
            .join("Memorithm__ADA/issue-9/programme-4144412d52/history/00000000000000000002.state");
        let mut interrupted = first.clone();
        interrupted.sequence = 2;
        interrupted.managed_branch = "orchestrator/issue-9-200".to_owned();
        fs::write(&history_path, serialize_state(&interrupted)).unwrap();
        let recovered = store
            .load_latest("Memorithm/ADA", 9, Some("ADA-R"))
            .unwrap()
            .unwrap();
        assert_eq!(recovered.sequence, 2);

        let third = store
            .append(
                "Memorithm/ADA",
                9,
                Some("ADA-R"),
                "orchestrator/issue-9-300",
                300,
                0,
                report(ResearchDecision::Revise),
            )
            .unwrap();
        assert_eq!(third.sequence, 3);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn incomplete_next_history_is_removed_and_retry_can_reuse_sequence() {
        let root = temporary_root("recover-partial");
        let store = ResearchCycleStore::new(root.clone());
        store
            .append(
                "Memorithm/ADA",
                9,
                None,
                "orchestrator/issue-9-100",
                100,
                0,
                report(ResearchDecision::Continue),
            )
            .unwrap();
        let history_path = root.join(
            "Memorithm__ADA/issue-9/programme-unspecified/history/00000000000000000002.state",
        );
        fs::write(&history_path, "research-cycle-v1\nrepository_hex=").unwrap();
        let recovered = store
            .load_latest("Memorithm/ADA", 9, None)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.sequence, 1);
        assert!(!history_path.exists());
        let second = store
            .append(
                "Memorithm/ADA",
                9,
                None,
                "orchestrator/issue-9-200",
                200,
                0,
                report(ResearchDecision::Revise),
            )
            .unwrap();
        assert_eq!(second.sequence, 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_or_authority_escalating_state_fails_closed() {
        let root = temporary_root("authority");
        let store = ResearchCycleStore::new(root.clone());
        store
            .append(
                "Memorithm/TDI",
                87,
                None,
                "orchestrator/issue-87-100",
                100,
                0,
                report(ResearchDecision::Blocked),
            )
            .unwrap();
        let latest = root.join("Memorithm__TDI/issue-87/programme-unspecified/latest.state");
        let contents = fs::read_to_string(&latest).unwrap();
        fs::write(
            &latest,
            contents.replace("parent_validation=unverified", "parent_validation=passed"),
        )
        .unwrap();
        assert!(store.load_latest("Memorithm/TDI", 87, None).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn control_characters_line_identity_and_identity_mismatch_fail_closed() {
        assert!(
            parse_handoff("research-cycle-v1\nline_id=recall-a\nhypothesis=H1\texploit\nexperiment=x\nevidence_report=y\ndecision=continue\nnext_action=z\n")
                .is_err()
        );
        assert!(validate_line_id("-starts-with-punctuation").is_err());
        assert!(validate_line_id("bad line").is_err());
        assert!(validate_line_id("line/escape").is_err());
        assert!(validate_line_id("line.valid_2-a").is_ok());
        assert!(validate_managed_branch("orchestrator/issue-8-100", 9).is_err());
        assert!(validate_worker_exit_code(256).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_state_directory_is_rejected_before_write() {
        let root = temporary_root("symlink");
        let outside = temporary_root("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("Memorithm__ADA")).unwrap();
        let store = ResearchCycleStore::new(root.clone());
        assert!(
            store
                .append(
                    "Memorithm/ADA",
                    9,
                    None,
                    "orchestrator/issue-9-100",
                    100,
                    0,
                    report(ResearchDecision::Continue),
                )
                .is_err()
        );
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn continuation_context_never_promotes_line_identity_or_agent_report_to_evidence() {
        let root = temporary_root("context");
        let store = ResearchCycleStore::new(root.clone());
        let record = store
            .append(
                "Memorithm/ADA",
                9,
                Some("ADA-R"),
                "orchestrator/issue-9-100",
                100,
                0,
                report(ResearchDecision::Continue),
            )
            .unwrap();
        let context = record.continuation_context();
        assert!(context.contains("UNVERIFIED AGENT REPORT"));
        assert!(context.contains("Research line ID: retrieval-fusion-a"));
        assert!(context.contains("does not prove"));
        assert!(handoff_contract().contains("line_id=<stable canonical identifier"));
        assert!(handoff_contract().contains("line_id and evidence_report remain unverified"));
        let _ = fs::remove_dir_all(root);
    }
}
