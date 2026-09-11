//! Strict, machine-readable cross-repository research dependency contract.
//!
//! The parser deliberately consumes only a dedicated top-level roadmap section.
//! It never infers dependencies from prose, issue text, repository names in
//! documentation, or agent reports. A later parent-scheduler slice may resolve
//! these exact declarations against GitHub state before launching research work.

use core::fmt;
use std::collections::BTreeSet;

const SECTION: &str = "research_dependencies:";
const SCHEMA_VERSION: &str = "1";
const MAX_REQUIREMENTS: usize = 64;
const MAX_PROGRAMME_BYTES: usize = 128;
const MAX_REPOSITORY_BYTES: usize = 256;

/// One exact provider commit required by a named autonomous-research programme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchDependency {
    programme: String,
    repository: String,
    merged_commit: String,
}

impl ResearchDependency {
    /// Stable programme identifier used by the explicit research-mode contract.
    #[must_use]
    pub fn programme(&self) -> &str {
        &self.programme
    }

    /// Canonical `owner/repository` provider identity.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// Exact full Git object identifier that must be reachable from the
    /// provider's current default branch before the dependency is satisfied.
    #[must_use]
    pub fn merged_commit(&self) -> &str {
        &self.merged_commit
    }
}

/// Versioned dependency declarations extracted from one roadmap document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchDependencyPlan {
    requirements: Vec<ResearchDependency>,
}

impl ResearchDependencyPlan {
    /// All declared requirements in deterministic document order.
    #[must_use]
    pub fn requirements(&self) -> &[ResearchDependency] {
        &self.requirements
    }

    /// Requirements that apply to one exact programme identifier.
    pub fn requirements_for<'a>(
        &'a self,
        programme: &'a str,
    ) -> impl Iterator<Item = &'a ResearchDependency> + 'a {
        self.requirements
            .iter()
            .filter(move |requirement| requirement.programme == programme)
    }
}

/// Fail-closed parse errors for the roadmap dependency contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResearchDependencyError {
    DuplicateSection { line: usize },
    TabIndentation { line: usize },
    MalformedField { line: usize },
    UnknownField { line: usize, field: String },
    DuplicateField { line: usize, field: String },
    MissingSchemaVersion,
    UnsupportedSchemaVersion { line: usize, version: String },
    MissingRequires,
    TooManyRequirements { line: usize },
    MissingRequirementField { field: &'static str },
    InvalidProgramme { line: usize },
    InvalidRepository { line: usize },
    InvalidCommit { line: usize },
    DuplicateRequirement { line: usize },
}

impl fmt::Display for ResearchDependencyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSection { line } => {
                write!(
                    formatter,
                    "duplicate research_dependencies section on line {line}"
                )
            }
            Self::TabIndentation { line } => {
                write!(formatter, "tab indentation is not allowed on line {line}")
            }
            Self::MalformedField { line } => {
                write!(
                    formatter,
                    "malformed research dependency field on line {line}"
                )
            }
            Self::UnknownField { line, field } => write!(
                formatter,
                "unknown research dependency field {field:?} on line {line}"
            ),
            Self::DuplicateField { line, field } => write!(
                formatter,
                "duplicate research dependency field {field:?} on line {line}"
            ),
            Self::MissingSchemaVersion => {
                formatter.write_str("research_dependencies is missing schema_version")
            }
            Self::UnsupportedSchemaVersion { line, version } => write!(
                formatter,
                "unsupported research dependency schema version {version:?} on line {line}"
            ),
            Self::MissingRequires => {
                formatter.write_str("research_dependencies is missing requires")
            }
            Self::TooManyRequirements { line } => write!(
                formatter,
                "research dependency plan exceeds {MAX_REQUIREMENTS} requirements on line {line}"
            ),
            Self::MissingRequirementField { field } => {
                write!(
                    formatter,
                    "research dependency requirement is missing {field}"
                )
            }
            Self::InvalidProgramme { line } => {
                write!(
                    formatter,
                    "invalid research programme identifier on line {line}"
                )
            }
            Self::InvalidRepository { line } => {
                write!(formatter, "invalid dependency repository on line {line}")
            }
            Self::InvalidCommit { line } => {
                write!(formatter, "invalid dependency commit on line {line}")
            }
            Self::DuplicateRequirement { line } => {
                write!(
                    formatter,
                    "duplicate research dependency requirement on line {line}"
                )
            }
        }
    }
}

impl std::error::Error for ResearchDependencyError {}

#[derive(Default)]
struct PendingRequirement {
    programme: Option<(String, usize)>,
    repository: Option<(String, usize)>,
    merged_commit: Option<(String, usize)>,
}

fn split_field(line: &str, line_number: usize) -> Result<(&str, &str), ResearchDependencyError> {
    let (key, value) = line
        .split_once(':')
        .ok_or(ResearchDependencyError::MalformedField { line: line_number })?;
    Ok((key.trim(), value.trim()))
}

fn set_once(
    target: &mut Option<(String, usize)>,
    field: &str,
    value: &str,
    line: usize,
) -> Result<(), ResearchDependencyError> {
    if target.is_some() {
        return Err(ResearchDependencyError::DuplicateField {
            line,
            field: field.to_owned(),
        });
    }
    if value.is_empty() {
        return Err(ResearchDependencyError::MalformedField { line });
    }
    *target = Some((value.to_owned(), line));
    Ok(())
}

fn finish_requirement(
    pending: &mut Option<PendingRequirement>,
    requirements: &mut Vec<ResearchDependency>,
    seen: &mut BTreeSet<(String, String, String)>,
    line: usize,
) -> Result<(), ResearchDependencyError> {
    let Some(pending) = pending.take() else {
        return Ok(());
    };
    if requirements.len() >= MAX_REQUIREMENTS {
        return Err(ResearchDependencyError::TooManyRequirements { line });
    }

    let (programme, programme_line) = pending
        .programme
        .ok_or(ResearchDependencyError::MissingRequirementField { field: "programme" })?;
    let (repository, repository_line) =
        pending
            .repository
            .ok_or(ResearchDependencyError::MissingRequirementField {
                field: "repository",
            })?;
    let (merged_commit, commit_line) =
        pending
            .merged_commit
            .ok_or(ResearchDependencyError::MissingRequirementField {
                field: "merged_commit",
            })?;

    if !valid_programme(&programme) {
        return Err(ResearchDependencyError::InvalidProgramme {
            line: programme_line,
        });
    }
    if !valid_repository(&repository) {
        return Err(ResearchDependencyError::InvalidRepository {
            line: repository_line,
        });
    }
    if !valid_commit(&merged_commit) {
        return Err(ResearchDependencyError::InvalidCommit { line: commit_line });
    }

    let identity = (
        programme.clone(),
        repository.to_ascii_lowercase(),
        merged_commit.clone(),
    );
    if !seen.insert(identity) {
        return Err(ResearchDependencyError::DuplicateRequirement { line });
    }

    requirements.push(ResearchDependency {
        programme,
        repository,
        merged_commit,
    });
    Ok(())
}

/// Parse the dedicated top-level `research_dependencies` roadmap section.
///
/// Supported schema:
///
/// ```text
/// research_dependencies:
///   schema_version: 1
///   requires:
///     - programme: TDI-8
///       repository: Memorithm/SciRust
///       merged_commit: 0123456789abcdef0123456789abcdef01234567
/// ```
///
/// The parser ignores all content outside this top-level section. Once the
/// section is present, its structure is strict and malformed/unknown fields fail
/// closed. The returned commit is evidence *requested* by policy; this parser
/// does not claim that the commit is actually merged or otherwise authoritative.
pub fn parse_roadmap_research_dependencies(
    document: &str,
) -> Result<Option<ResearchDependencyPlan>, ResearchDependencyError> {
    let mut in_section = false;
    let mut saw_section = false;
    let mut schema_version = None;
    let mut saw_requires = false;
    let mut in_requires = false;
    let mut pending = None;
    let mut requirements = Vec::new();
    let mut seen = BTreeSet::new();

    for (zero_indexed, raw_line) in document.lines().enumerate() {
        let line_number = zero_indexed + 1;
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if !in_section {
            if raw_line == SECTION {
                if saw_section {
                    return Err(ResearchDependencyError::DuplicateSection { line: line_number });
                }
                saw_section = true;
                in_section = true;
            }
            continue;
        }

        if raw_line.starts_with('\t') {
            return Err(ResearchDependencyError::TabIndentation { line: line_number });
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();

        if indent == 0 {
            finish_requirement(&mut pending, &mut requirements, &mut seen, line_number)?;
            in_section = false;
            in_requires = false;
            if raw_line == SECTION {
                return Err(ResearchDependencyError::DuplicateSection { line: line_number });
            }
            continue;
        }

        if indent == 2 {
            finish_requirement(&mut pending, &mut requirements, &mut seen, line_number)?;
            in_requires = false;
            let (key, value) = split_field(trimmed, line_number)?;
            match key {
                "schema_version" => {
                    if schema_version.is_some() {
                        return Err(ResearchDependencyError::DuplicateField {
                            line: line_number,
                            field: key.to_owned(),
                        });
                    }
                    if value != SCHEMA_VERSION {
                        return Err(ResearchDependencyError::UnsupportedSchemaVersion {
                            line: line_number,
                            version: value.to_owned(),
                        });
                    }
                    schema_version = Some(SCHEMA_VERSION);
                }
                "requires" => {
                    if saw_requires {
                        return Err(ResearchDependencyError::DuplicateField {
                            line: line_number,
                            field: key.to_owned(),
                        });
                    }
                    if !value.is_empty() {
                        return Err(ResearchDependencyError::MalformedField { line: line_number });
                    }
                    saw_requires = true;
                    in_requires = true;
                }
                other => {
                    return Err(ResearchDependencyError::UnknownField {
                        line: line_number,
                        field: other.to_owned(),
                    });
                }
            }
            continue;
        }

        if !in_requires {
            return Err(ResearchDependencyError::MalformedField { line: line_number });
        }

        if indent == 4 {
            let item = trimmed
                .strip_prefix("- ")
                .ok_or(ResearchDependencyError::MalformedField { line: line_number })?;
            finish_requirement(&mut pending, &mut requirements, &mut seen, line_number)?;
            let (key, value) = split_field(item, line_number)?;
            if key != "programme" {
                return Err(ResearchDependencyError::UnknownField {
                    line: line_number,
                    field: key.to_owned(),
                });
            }
            let mut next = PendingRequirement::default();
            set_once(&mut next.programme, key, value, line_number)?;
            pending = Some(next);
            continue;
        }

        if indent == 6 {
            let pending = pending
                .as_mut()
                .ok_or(ResearchDependencyError::MalformedField { line: line_number })?;
            let (key, value) = split_field(trimmed, line_number)?;
            match key {
                "repository" => {
                    set_once(&mut pending.repository, key, value, line_number)?;
                }
                "merged_commit" => {
                    set_once(&mut pending.merged_commit, key, value, line_number)?;
                }
                "programme" => {
                    set_once(&mut pending.programme, key, value, line_number)?;
                }
                other => {
                    return Err(ResearchDependencyError::UnknownField {
                        line: line_number,
                        field: other.to_owned(),
                    });
                }
            }
            continue;
        }

        return Err(ResearchDependencyError::MalformedField { line: line_number });
    }

    if in_section {
        finish_requirement(
            &mut pending,
            &mut requirements,
            &mut seen,
            document.lines().count().saturating_add(1),
        )?;
    }

    if !saw_section {
        return Ok(None);
    }
    if schema_version != Some(SCHEMA_VERSION) {
        return Err(ResearchDependencyError::MissingSchemaVersion);
    }
    if !saw_requires {
        return Err(ResearchDependencyError::MissingRequires);
    }

    Ok(Some(ResearchDependencyPlan { requirements }))
}

fn valid_programme(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROGRAMME_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

fn valid_repository(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_REPOSITORY_BYTES || value.chars().any(char::is_control)
    {
        return false;
    }
    let mut parts = value.split('/');
    let Some(owner) = parts.next() else {
        return false;
    };
    let Some(repository) = parts.next() else {
        return false;
    };
    if parts.next().is_some() || owner.is_empty() || repository.is_empty() {
        return false;
    }
    owner
        .bytes()
        .chain(repository.bytes())
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "0123456789abcdef0123456789abcdef01234567";
    const SHA_B: &str = "89abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn absent_section_is_inert() {
        assert_eq!(
            parse_roadmap_research_dependencies("roadmap:\n  - id: TDI-8\n"),
            Ok(None)
        );
    }

    #[test]
    fn parses_exact_programme_repository_and_commit_requirements() {
        let document = format!(
            "kind: test\nresearch_dependencies:\n  schema_version: 1\n  requires:\n    - programme: TDI-8\n      repository: Memorithm/SciRust\n      merged_commit: {SHA_A}\n    - programme: TDI-8\n      repository: Memorithm/ElasticXxx\n      merged_commit: {SHA_B}\nroadmap:\n  - id: TDI-8\n"
        );
        let plan = parse_roadmap_research_dependencies(&document)
            .expect("valid dependency contract")
            .expect("section present");
        assert_eq!(plan.requirements().len(), 2);
        let tdi = plan.requirements_for("TDI-8").collect::<Vec<_>>();
        assert_eq!(tdi.len(), 2);
        assert_eq!(tdi[0].repository(), "Memorithm/SciRust");
        assert_eq!(tdi[0].merged_commit(), SHA_A);
        assert_eq!(tdi[1].repository(), "Memorithm/ElasticXxx");
    }

    #[test]
    fn requirements_for_other_programme_are_not_selected() {
        let document = format!(
            "research_dependencies:\n  schema_version: 1\n  requires:\n    - programme: TDI-8\n      repository: Memorithm/SciRust\n      merged_commit: {SHA_A}\n    - programme: PROOF-1\n      repository: Memorithm/ProofLab\n      merged_commit: {SHA_B}\n"
        );
        let plan = parse_roadmap_research_dependencies(&document)
            .unwrap()
            .unwrap();
        let proof = plan.requirements_for("PROOF-1").collect::<Vec<_>>();
        assert_eq!(proof.len(), 1);
        assert_eq!(proof[0].programme(), "PROOF-1");
        assert_eq!(proof[0].repository(), "Memorithm/ProofLab");
    }

    #[test]
    fn duplicate_requirement_fails_closed_case_insensitively_for_repository() {
        let document = format!(
            "research_dependencies:\n  schema_version: 1\n  requires:\n    - programme: TDI-8\n      repository: Memorithm/SciRust\n      merged_commit: {SHA_A}\n    - programme: TDI-8\n      repository: memorithm/scirust\n      merged_commit: {SHA_A}\n"
        );
        assert!(matches!(
            parse_roadmap_research_dependencies(&document),
            Err(ResearchDependencyError::DuplicateRequirement { .. })
        ));
    }

    #[test]
    fn malformed_or_unsupported_section_fails_closed() {
        assert_eq!(
            parse_roadmap_research_dependencies("research_dependencies:\n  requires:\n"),
            Err(ResearchDependencyError::MissingSchemaVersion)
        );
        assert!(matches!(
            parse_roadmap_research_dependencies(
                "research_dependencies:\n  schema_version: 2\n  requires:\n"
            ),
            Err(ResearchDependencyError::UnsupportedSchemaVersion { .. })
        ));
        assert!(matches!(
            parse_roadmap_research_dependencies(
                "research_dependencies:\n  schema_version: 1\n  magic: yes\n"
            ),
            Err(ResearchDependencyError::UnknownField { .. })
        ));
    }

    #[test]
    fn unsafe_values_fail_closed() {
        let bad_repository = format!(
            "research_dependencies:\n  schema_version: 1\n  requires:\n    - programme: TDI-8\n      repository: Memorithm/SciRust/extra\n      merged_commit: {SHA_A}\n"
        );
        assert!(matches!(
            parse_roadmap_research_dependencies(&bad_repository),
            Err(ResearchDependencyError::InvalidRepository { .. })
        ));

        let bad_commit = "research_dependencies:\n  schema_version: 1\n  requires:\n    - programme: TDI-8\n      repository: Memorithm/SciRust\n      merged_commit: deadbeef\n";
        assert!(matches!(
            parse_roadmap_research_dependencies(bad_commit),
            Err(ResearchDependencyError::InvalidCommit { .. })
        ));

        let bad_programme = format!(
            "research_dependencies:\n  schema_version: 1\n  requires:\n    - programme: TDI 8\n      repository: Memorithm/SciRust\n      merged_commit: {SHA_A}\n"
        );
        assert!(matches!(
            parse_roadmap_research_dependencies(&bad_programme),
            Err(ResearchDependencyError::InvalidProgramme { .. })
        ));
    }

    #[test]
    fn nested_or_prose_mentions_do_not_activate_section() {
        let prose = "notes:\n  research_dependencies:\n    schema_version: 1\n";
        assert_eq!(parse_roadmap_research_dependencies(prose), Ok(None));
    }
}
