//! Explicit autonomous-research programme contract.
//!
//! Research authority is never inferred from issue prose. A broad programme
//! issue opts in only through an exact, versioned HTML-comment directive. The
//! resulting directive changes research strategy, not repository permissions:
//! target-repository policy, human-only/forbidden actions, validation,
//! publication, hardware-evidence and exact-head merge gates remain external
//! authorities that this module cannot override.

use core::fmt;

const MODE_KEY: &str = "orchestrator-research-mode";
const PROGRAMME_KEY: &str = "orchestrator-research-programme";
const AUTONOMOUS_V1: &str = "autonomous-v1";
const MAX_PROGRAMME_ID_BYTES: usize = 128;

/// Versioned research mode explicitly requested by a programme issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchMode {
    /// The agent may choose and revise the next bounded research slice from
    /// repository state and executed evidence without intermediate human
    /// approval, subject to all repository policy gates.
    AutonomousV1,
}

impl ResearchMode {
    /// Stable machine-readable mode identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AutonomousV1 => AUTONOMOUS_V1,
        }
    }
}

/// Parsed autonomous-research authority attached to one issue revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchDirective {
    mode: ResearchMode,
    programme: Option<String>,
}

impl ResearchDirective {
    /// Explicit versioned mode.
    #[must_use]
    pub const fn mode(&self) -> ResearchMode {
        self.mode
    }

    /// Optional stable programme identifier supplied by the project manager.
    #[must_use]
    pub fn programme(&self) -> Option<&str> {
        self.programme.as_deref()
    }

    /// Generate the strategy paragraph injected into an issue worker mission.
    ///
    /// This text deliberately grants scientific choice but no publication,
    /// credential, financial, hardware-attestation, holdout or policy-bypass
    /// authority.
    #[must_use]
    pub fn mission(&self, issue_number: u64) -> String {
        let programme = self
            .programme()
            .map_or_else(|| "(unspecified)".to_owned(), ToOwned::to_owned);
        format!(
            "Operate issue #{issue_number} as autonomous research programme {programme}. \
Inspect the current repository state, merged work, roadmap, tests, benchmarks and executed evidence before choosing the next slice. \
Within the actions permitted by the repository policy, independently formulate or revise hypotheses, choose the highest-value bounded evidence-producing experiment, control or ablation, implement it, run the relevant non-forbidden validation, analyze the executed result, and decide whether the permitted research line should continue, be revised, or be abandoned. \
Do not wait for intermediate human approval merely to choose among permitted research directions. Keep this execution to one coherent reviewable publication slice. \
Never interpret research autonomy as permission to cross a human-only, forbidden, financial, credential, hardware-evidence, holdout, validation, publication or merge gate. If the scientifically preferred next action is gated, advance the best permitted precursor or record the blocker without fabricating authorization. \
Executed evidence is authoritative over model speculation; preserve negative, null and inconclusive evidence and make no unsupported scientific claim."
        )
    }
}

/// Fail-closed errors for explicit research directives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResearchDirectiveError {
    /// A line used the reserved machine-directive namespace but was malformed.
    MalformedDirective { line: usize },
    /// A reserved but unsupported research directive key was supplied.
    UnknownDirective { line: usize, key: String },
    /// The mode directive appeared more than once.
    DuplicateMode { line: usize },
    /// The programme directive appeared more than once.
    DuplicateProgramme { line: usize },
    /// A versioned mode other than the currently supported contract was used.
    UnsupportedMode { line: usize, mode: String },
    /// A programme identifier was supplied without an explicit mode opt-in.
    ProgrammeWithoutMode,
    /// The optional programme identifier was empty, too large or contained an
    /// unsafe/non-canonical character.
    InvalidProgramme { line: usize },
}

impl fmt::Display for ResearchDirectiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedDirective { line } => {
                write!(formatter, "malformed autonomous-research directive on line {line}")
            }
            Self::UnknownDirective { line, key } => write!(
                formatter,
                "unknown autonomous-research directive {key:?} on line {line}"
            ),
            Self::DuplicateMode { line } => {
                write!(formatter, "duplicate autonomous-research mode on line {line}")
            }
            Self::DuplicateProgramme { line } => write!(
                formatter,
                "duplicate autonomous-research programme identifier on line {line}"
            ),
            Self::UnsupportedMode { line, mode } => write!(
                formatter,
                "unsupported autonomous-research mode {mode:?} on line {line}"
            ),
            Self::ProgrammeWithoutMode => formatter.write_str(
                "autonomous-research programme identifier requires an explicit research mode",
            ),
            Self::InvalidProgramme { line } => write!(
                formatter,
                "invalid autonomous-research programme identifier on line {line}"
            ),
        }
    }
}

impl std::error::Error for ResearchDirectiveError {}

/// Parse an issue body for an explicit autonomous-research opt-in.
///
/// Only whole trimmed HTML-comment lines in the reserved namespace are
/// machine directives. Mentions in prose, quoted examples and arbitrary
/// substrings do not grant authority. The currently supported form is:
///
/// `<!-- orchestrator-research-mode: autonomous-v1 -->`
///
/// Optionally followed by exactly one canonical programme identifier:
///
/// `<!-- orchestrator-research-programme: TDI-8 -->`
pub fn parse_issue_directive(
    body: &str,
) -> Result<Option<ResearchDirective>, ResearchDirectiveError> {
    let mut mode = None;
    let mut programme = None;

    for (zero_indexed, raw_line) in body.lines().enumerate() {
        let line_number = zero_indexed + 1;
        let line = raw_line.trim();

        // A normal prose mention is deliberately inert. Only a whole comment
        // line enters the reserved machine-directive parser.
        if !line.starts_with("<!-- orchestrator-research-") {
            continue;
        }
        let inner = line
            .strip_prefix("<!--")
            .and_then(|value| value.strip_suffix("-->"))
            .map(str::trim)
            .ok_or(ResearchDirectiveError::MalformedDirective { line: line_number })?;
        let (key, value) = inner
            .split_once(':')
            .ok_or(ResearchDirectiveError::MalformedDirective { line: line_number })?;
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            return Err(ResearchDirectiveError::MalformedDirective { line: line_number });
        }

        match key {
            MODE_KEY => {
                if mode.is_some() {
                    return Err(ResearchDirectiveError::DuplicateMode { line: line_number });
                }
                if value != AUTONOMOUS_V1 {
                    return Err(ResearchDirectiveError::UnsupportedMode {
                        line: line_number,
                        mode: value.to_owned(),
                    });
                }
                mode = Some(ResearchMode::AutonomousV1);
            }
            PROGRAMME_KEY => {
                if programme.is_some() {
                    return Err(ResearchDirectiveError::DuplicateProgramme {
                        line: line_number,
                    });
                }
                if !valid_programme_id(value) {
                    return Err(ResearchDirectiveError::InvalidProgramme { line: line_number });
                }
                programme = Some(value.to_owned());
            }
            other => {
                return Err(ResearchDirectiveError::UnknownDirective {
                    line: line_number,
                    key: other.to_owned(),
                });
            }
        }
    }

    match mode {
        Some(mode) => Ok(Some(ResearchDirective { mode, programme })),
        None if programme.is_some() => Err(ResearchDirectiveError::ProgrammeWithoutMode),
        None => Ok(None),
    }
}

fn valid_programme_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROGRAMME_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_directive_keeps_standard_issue_semantics() {
        assert_eq!(
            parse_issue_directive("Investigate the best next experiment."),
            Ok(None)
        );
    }

    #[test]
    fn exact_versioned_opt_in_enables_autonomous_research() {
        let directive = parse_issue_directive(
            "# TDI-8 programme\n<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: TDI-8 -->\n",
        )
        .expect("valid directive")
        .expect("opted in");
        assert_eq!(directive.mode(), ResearchMode::AutonomousV1);
        assert_eq!(directive.mode().as_str(), "autonomous-v1");
        assert_eq!(directive.programme(), Some("TDI-8"));

        let mission = directive.mission(87);
        assert!(mission.contains("independently formulate or revise hypotheses"));
        assert!(mission.contains("Do not wait for intermediate human approval"));
        assert!(mission.contains("human-only"));
        assert!(mission.contains("Executed evidence is authoritative"));
    }

    #[test]
    fn prose_mention_never_grants_research_authority() {
        let body = "Please document <!-- orchestrator-research-mode: autonomous-v1 --> as an example.";
        assert_eq!(parse_issue_directive(body), Ok(None));
    }

    #[test]
    fn duplicate_or_unknown_mode_fails_closed() {
        let duplicate = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-mode: autonomous-v1 -->";
        assert!(matches!(
            parse_issue_directive(duplicate),
            Err(ResearchDirectiveError::DuplicateMode { line: 2 })
        ));

        let unsupported = "<!-- orchestrator-research-mode: autonomous-v2 -->";
        assert!(matches!(
            parse_issue_directive(unsupported),
            Err(ResearchDirectiveError::UnsupportedMode { line: 1, .. })
        ));
    }

    #[test]
    fn malformed_reserved_comment_fails_closed() {
        assert!(matches!(
            parse_issue_directive("<!-- orchestrator-research-mode autonomous-v1 -->"),
            Err(ResearchDirectiveError::MalformedDirective { line: 1 })
        ));
        assert!(matches!(
            parse_issue_directive("<!-- orchestrator-research-mode: autonomous-v1"),
            Err(ResearchDirectiveError::MalformedDirective { line: 1 })
        ));
    }

    #[test]
    fn programme_requires_mode_and_canonical_identifier() {
        assert_eq!(
            parse_issue_directive("<!-- orchestrator-research-programme: TDI-8 -->"),
            Err(ResearchDirectiveError::ProgrammeWithoutMode)
        );
        assert!(matches!(
            parse_issue_directive(
                "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: TDI 8 -->"
            ),
            Err(ResearchDirectiveError::InvalidProgramme { line: 2 })
        ));
    }

    #[test]
    fn unknown_reserved_directive_fails_closed() {
        assert!(matches!(
            parse_issue_directive("<!-- orchestrator-research-permission: all -->"),
            Err(ResearchDirectiveError::UnknownDirective { line: 1, .. })
        ));
    }
}
