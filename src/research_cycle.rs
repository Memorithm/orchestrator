//! Durable, bounded state for explicitly opted-in autonomous research cycles.
//!
//! A cycle record is intentionally an agent report, not authoritative executed
//! evidence. Repository validation, CI, hardware evidence, publication and merge
//! gates remain owned by the Orchestrator parent. Persisted reports are carried
//! into later cycles only as bounded context.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const STATE_VERSION: &str = "research-cycle-v1";
pub const HANDOFF_FILE: &str = ".orchestrator-research-cycle-v1";
const MAX_TEXT_BYTES: usize = 8 * 1024;
const MAX_REPOSITORY_BYTES: usize = 256;
const MAX_PROGRAMME_BYTES: usize = 128;
const MAX_BRANCH_BYTES: usize = 256;

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
            "PRIOR RESEARCH CYCLE (UNVERIFIED AGENT REPORT)\nSequence: {}\nHypothesis: {}\nExperiment: {}\nAgent evidence report: {}\nDecision: {}\nNext action: {}\nParent validation status: UNVERIFIED — this record does not prove that the experiment, benchmark, CI, hardware run, or claimed result succeeded.",
            self.sequence,
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
    ) -> Result<Option<ResearchCycleRecord>, String> {
        validate_repository(repository)?;
        let path = self.issue_root(repository, issue_number).join("latest.state");
        if !path.exists() {
            return Ok(None);
        }
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read research cycle {}: {error}", path.display()))?;
        let record = parse_state(&contents)
            .map_err(|error| format!("invalid research cycle {}: {error}", path.display()))?;
        if record.repository != repository || record.issue_number != issue_number {
            return Err(format!(
                "research cycle identity mismatch in {}",
                path.display()
            ));
        }
        Ok(Some(record))
    }

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
        validate_optional_programme(programme)?;
        validate_bounded_line("managed branch", managed_branch, MAX_BRANCH_BYTES)?;
        validate_report(&report)?;

        let previous = self.load_latest(repository, issue_number)?;
        let sequence = previous
            .as_ref()
            .map_or(1, |record| record.sequence.saturating_add(1));
        if sequence == u64::MAX && previous.as_ref().is_some_and(|record| record.sequence == u64::MAX)
        {
            return Err("research cycle sequence exhausted".to_owned());
        }

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
        let issue_root = self.issue_root(repository, issue_number);
        let history_root = issue_root.join("history");
        fs::create_dir_all(&history_root).map_err(|error| {
            format!(
                "failed to create research cycle history {}: {error}",
                history_root.display()
            )
        })?;
        reject_symlink(&issue_root)?;
        reject_symlink(&history_root)?;

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
        history
            .write_all(serialized.as_bytes())
            .map_err(|error| format!("failed to write {}: {error}", history_path.display()))?;
        history
            .sync_all()
            .map_err(|error| format!("failed to sync {}: {error}", history_path.display()))?;

        let latest = issue_root.join("latest.state");
        let temporary = issue_root.join(format!("latest.state.tmp.{}", std::process::id()));
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("failed to open {}: {error}", temporary.display()))?;
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
        })?;
        Ok(record)
    }

    fn issue_root(&self, repository: &str, issue_number: u64) -> PathBuf {
        self.root
            .join(repository.replace('/', "__"))
            .join(format!("issue-{issue_number}"))
    }
}

pub fn parse_handoff(contents: &str) -> Result<ResearchCycleReport, String> {
    if contents.len() > MAX_TEXT_BYTES.saturating_mul(5).saturating_add(1024) {
        return Err("research cycle handoff exceeds bounded size".to_owned());
    }
    let mut lines = contents.lines();
    if lines.next() != Some(STATE_VERSION) {
        return Err("unsupported or missing research cycle handoff version".to_owned());
    }

    let mut seen = BTreeSet::new();
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
        hypothesis: hypothesis.ok_or_else(|| "research cycle handoff missing hypothesis".to_owned())?,
        experiment: experiment.ok_or_else(|| "research cycle handoff missing experiment".to_owned())?,
        evidence_report: evidence_report
            .ok_or_else(|| "research cycle handoff missing evidence_report".to_owned())?,
        decision: decision.ok_or_else(|| "research cycle handoff missing decision".to_owned())?,
        next_action: next_action.ok_or_else(|| "research cycle handoff missing next_action".to_owned())?,
    };
    validate_report(&report)?;
    Ok(report)
}

#[must_use]
pub fn handoff_contract() -> String {
    format!(
        "RESEARCH CYCLE HANDOFF (REQUIRED FOR THIS EXPLICIT RESEARCH MODE)\nBefore finishing, write exactly one parent-only machine handoff file named `{HANDOFF_FILE}` at the repository root. This is the sole exception to the generic rule against status-report files; it is not a product artifact and Orchestrator removes it before diff validation or publication. Use exactly this single-line format (one field per line, no Markdown):\n{STATE_VERSION}\nhypothesis=<current falsifiable hypothesis, one line>\nexperiment=<bounded experiment/control/ablation actually attempted this cycle, one line>\nevidence_report=<what you observed; state negative, null, inconclusive, blocked, or failed results explicitly, one line>\ndecision=<continue|revise|abandon|blocked>\nnext_action=<best next permitted research action, one line>\nThe evidence_report is an unverified agent report. Never describe it as authoritative CI, hardware, benchmark, or validation evidence unless the parent-provided evidence actually establishes that fact."
    )
}

fn validate_report(report: &ResearchCycleReport) -> Result<(), String> {
    validate_bounded_line("hypothesis", &report.hypothesis, MAX_TEXT_BYTES)?;
    validate_bounded_line("experiment", &report.experiment, MAX_TEXT_BYTES)?;
    validate_bounded_line("evidence_report", &report.evidence_report, MAX_TEXT_BYTES)?;
    validate_bounded_line("next_action", &report.next_action, MAX_TEXT_BYTES)
}

fn validate_repository(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_REPOSITORY_BYTES
        || value.matches('/').count() != 1
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.')
        })
    {
        return Err("invalid research cycle repository identity".to_owned());
    }
    Ok(())
}

fn validate_optional_programme(value: Option<&str>) -> Result<(), String> {
    if let Some(value) = value {
        validate_bounded_line("programme", value, MAX_PROGRAMME_BYTES)?;
    }
    Ok(())
}

fn validate_bounded_line(name: &str, value: &str, maximum: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > maximum
        || value.bytes().any(|byte| matches!(byte, b'\n' | b'\r' | 0))
    {
        return Err(format!("invalid or oversized research cycle {name}"));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    if fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
        .file_type()
        .is_symlink()
    {
        return Err(format!("research cycle state path is a symlink: {}", path.display()));
    }
    Ok(())
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
    for pair in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(pair)
            .map_err(|_| format!("invalid encoded research cycle {name}"))?;
        let byte = u8::from_str_radix(text, 16)
            .map_err(|_| format!("invalid encoded research cycle {name}"))?;
        bytes.push(byte);
    }
    let decoded = String::from_utf8(bytes)
        .map_err(|_| format!("invalid UTF-8 research cycle {name}"))?;
    validate_bounded_line(name, &decoded, maximum)?;
    Ok(decoded)
}

fn serialize_state(record: &ResearchCycleRecord) -> String {
    format!(
        "{STATE_VERSION}\nrepository_hex={}\nissue_number={}\nprogramme_hex={}\nmanaged_branch_hex={}\nsequence={}\nrecorded_at={}\nworker_exit_code={}\nevidence_authority=agent_report_unverified\nparent_validation=unverified\nhypothesis_hex={}\nexperiment_hex={}\nevidence_report_hex={}\ndecision={}\nnext_action_hex={}\n",
        hex_encode(&record.repository),
        record.issue_number,
        record.programme.as_deref().map_or_else(String::new, hex_encode),
        hex_encode(&record.managed_branch),
        record.sequence,
        record.recorded_at,
        record.worker_exit_code,
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
    let mut fields = std::collections::BTreeMap::new();
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
        return Err("research cycle state attempted to claim unsupported evidence authority".to_owned());
    }
    let repository = hex_decode("repository", take("repository_hex")?, MAX_REPOSITORY_BYTES)?;
    validate_repository(&repository)?;
    let programme_hex = take("programme_hex")?;
    let programme = if programme_hex.is_empty() {
        None
    } else {
        Some(hex_decode("programme", programme_hex, MAX_PROGRAMME_BYTES)?)
    };
    let report = ResearchCycleReport {
        hypothesis: hex_decode("hypothesis", take("hypothesis_hex")?, MAX_TEXT_BYTES)?,
        experiment: hex_decode("experiment", take("experiment_hex")?, MAX_TEXT_BYTES)?,
        evidence_report: hex_decode(
            "evidence_report",
            take("evidence_report_hex")?,
            MAX_TEXT_BYTES,
        )?,
        decision: ResearchDecision::parse(take("decision")?)?,
        next_action: hex_decode("next_action", take("next_action_hex")?, MAX_TEXT_BYTES)?,
    };
    validate_report(&report)?;
    Ok(ResearchCycleRecord {
        repository,
        issue_number: take("issue_number")?
            .parse()
            .map_err(|error| format!("invalid research cycle issue number: {error}"))?,
        programme,
        managed_branch: hex_decode("managed_branch", take("managed_branch_hex")?, MAX_BRANCH_BYTES)?,
        sequence: take("sequence")?
            .parse()
            .map_err(|error| format!("invalid research cycle sequence: {error}"))?,
        recorded_at: take("recorded_at")?
            .parse()
            .map_err(|error| format!("invalid research cycle recorded_at: {error}"))?,
        worker_exit_code: take("worker_exit_code")?
            .parse()
            .map_err(|error| format!("invalid research cycle worker_exit_code: {error}"))?,
        report,
    })
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
            hypothesis: "bounded state improves recall".to_owned(),
            experiment: "run deterministic recall ablation".to_owned(),
            evidence_report: "null result on synthetic case".to_owned(),
            decision,
            next_action: "revise the retrieval fusion control".to_owned(),
        }
    }

    #[test]
    fn handoff_parser_is_strict_and_preserves_negative_evidence() {
        let parsed = parse_handoff(
            "research-cycle-v1\nhypothesis=H1\nexperiment=control A\nevidence_report=null result\ndecision=revise\nnext_action=control B\n",
        )
        .unwrap();
        assert_eq!(parsed.decision, ResearchDecision::Revise);
        assert_eq!(parsed.evidence_report, "null result");
        assert!(parse_handoff("research-cycle-v1\nhypothesis=H1\n").is_err());
        assert!(parse_handoff("research-cycle-v1\nhypothesis=H1\nhypothesis=H2\nexperiment=x\nevidence_report=y\ndecision=continue\nnext_action=z\n").is_err());
        assert!(parse_handoff("research-cycle-v1\nhypothesis=H1\nexperiment=x\nevidence_report=y\ndecision=win\nnext_action=z\n").is_err());
    }

    #[test]
    fn store_is_append_only_and_latest_round_trips() {
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
        assert_eq!(store.load_latest("Memorithm/TDI", 87).unwrap(), Some(second));
        let history = root.join("Memorithm__TDI/issue-87/history");
        assert_eq!(fs::read_dir(history).unwrap().count(), 2);
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
        let latest = root.join("Memorithm__TDI/issue-87/latest.state");
        let contents = fs::read_to_string(&latest).unwrap();
        fs::write(
            &latest,
            contents.replace("parent_validation=unverified", "parent_validation=passed"),
        )
        .unwrap();
        assert!(store.load_latest("Memorithm/TDI", 87).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn continuation_context_never_promotes_agent_report_to_evidence() {
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
        assert!(context.contains("does not prove"));
        assert!(handoff_contract().contains(HANDOFF_FILE));
        let _ = fs::remove_dir_all(root);
    }
}
