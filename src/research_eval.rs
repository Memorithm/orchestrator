//! Parent-owned evaluation of declared research provider prerequisites.
//!
//! This module does not talk to GitHub. It only turns an issue body and a
//! parent-resolved policy snapshot into exact programme requirements and a
//! durable defer reason. Live default-branch comparison stays in the parent
//! scheduler gate.

use core::fmt;

use crate::research_dependency::{
    ResearchDependency, ResearchDependencyError, parse_roadmap_research_dependencies,
};

/// Observed GitHub default-branch comparison for one declared requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderComparison {
    default_branch: String,
    default_head: String,
    compare_status: String,
}

impl ProviderComparison {
    /// Construct a comparison after the parent resolver validated scalars.
    #[must_use]
    pub fn new(
        default_branch: impl Into<String>,
        default_head: impl Into<String>,
        compare_status: impl Into<String>,
    ) -> Self {
        Self {
            default_branch: default_branch.into(),
            default_head: default_head.into(),
            compare_status: compare_status.into(),
        }
    }

    /// Provider default branch used for the comparison.
    #[must_use]
    pub fn default_branch(&self) -> &str {
        &self.default_branch
    }

    /// Exact current default-branch object identifier.
    #[must_use]
    pub fn default_head(&self) -> &str {
        &self.default_head
    }

    /// GitHub compare status between the required commit and the default head.
    #[must_use]
    pub fn compare_status(&self) -> &str {
        &self.compare_status
    }
}

/// Durable scheduler/publication reason for one unresolved provider commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedProviderDependency {
    programme: String,
    repository: String,
    required_commit: String,
    comparison: ProviderComparison,
    policy_identity: String,
}

impl UnresolvedProviderDependency {
    /// Bind an unresolved comparison to the consumer policy identity.
    #[must_use]
    pub fn new(
        programme: impl Into<String>,
        requirement: &ResearchDependency,
        comparison: ProviderComparison,
        policy_identity: impl Into<String>,
    ) -> Self {
        Self {
            programme: programme.into(),
            repository: requirement.repository().to_owned(),
            required_commit: requirement.merged_commit().to_owned(),
            comparison,
            policy_identity: policy_identity.into(),
        }
    }

    /// Exact programme that requested the provider commit.
    #[must_use]
    pub fn programme(&self) -> &str {
        &self.programme
    }

    /// Canonical provider repository.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// Required provider object identifier.
    #[must_use]
    pub fn required_commit(&self) -> &str {
        &self.required_commit
    }

    /// Observed provider comparison.
    #[must_use]
    pub fn comparison(&self) -> &ProviderComparison {
        &self.comparison
    }

    /// Parent-resolved consumer policy identity token.
    #[must_use]
    pub fn policy_identity(&self) -> &str {
        &self.policy_identity
    }

    /// Stable trajectory/scheduler detail. Provider identity is explicit.
    #[must_use]
    pub fn defer_reason(&self) -> String {
        format!(
            "research dependency deferred programme={} provider={} required_commit={} default_branch={} default_head={} compare_status={} policy_identity={}",
            self.programme,
            self.repository,
            self.required_commit,
            self.comparison.default_branch(),
            self.comparison.default_head(),
            self.comparison.compare_status(),
            self.policy_identity
        )
    }
}

/// True when GitHub reports the required commit is the current default head
/// or an ancestor of that head.
#[must_use]
pub fn default_history_satisfies(compare_status: &str) -> bool {
    matches!(compare_status, "ahead" | "identical")
}

/// Full lowercase Git object identifier accepted by the dependency contract.
#[must_use]
pub fn valid_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Fail-closed errors while selecting programme-scoped provider requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedRequirementsError {
    Directive(crate::research::ResearchDirectiveError),
    Policy(ResearchDependencyError),
}

impl fmt::Display for SelectedRequirementsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Directive(error) => {
                write!(
                    formatter,
                    "autonomous research directive rejected: {error}"
                )
            }
            Self::Policy(error) => {
                write!(formatter, "research dependency policy rejected: {error}")
            }
        }
    }
}

impl std::error::Error for SelectedRequirementsError {}

/// Select declared requirements for one issue body and policy snapshot.
///
/// Ordinary issues, research issues without a programme identifier, and
/// programmes without matching `research_dependencies` entries are inert.
pub fn selected_requirements(
    body: &str,
    policy_context: &str,
) -> Result<Option<(String, Vec<ResearchDependency>)>, SelectedRequirementsError> {
    let Some(directive) = crate::research::parse_issue_directive(body)
        .map_err(SelectedRequirementsError::Directive)?
    else {
        return Ok(None);
    };
    let Some(programme) = directive.programme() else {
        return Ok(None);
    };
    let Some(plan) = parse_roadmap_research_dependencies(policy_context)
        .map_err(SelectedRequirementsError::Policy)?
    else {
        return Ok(None);
    };
    let requirements = plan
        .requirements_for(programme)
        .cloned()
        .collect::<Vec<_>>();
    if requirements.is_empty() {
        return Ok(None);
    }
    Ok(Some((programme.to_owned(), requirements)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "0123456789abcdef0123456789abcdef01234567";
    const SHA_B: &str = "89abcdef0123456789abcdef0123456789abcdef";

    fn requirement() -> ResearchDependency {
        let document = format!(
            "research_dependencies:\n  schema_version: 1\n  requires:\n    - programme: TDI-8\n      repository: Memorithm/Provider\n      merged_commit: {SHA_A}\n"
        );
        parse_roadmap_research_dependencies(&document)
            .unwrap()
            .unwrap()
            .requirements_for("TDI-8")
            .next()
            .unwrap()
            .clone()
    }

    #[test]
    fn only_default_history_success_states_satisfy() {
        assert!(default_history_satisfies("ahead"));
        assert!(default_history_satisfies("identical"));
        assert!(!default_history_satisfies("behind"));
        assert!(!default_history_satisfies("diverged"));
        assert!(!default_history_satisfies("unknown"));
    }

    #[test]
    fn selected_requirements_are_programme_exact_and_opt_in_only() {
        let policy = format!(
            "research_dependencies:\n  schema_version: 1\n  requires:\n    - programme: TDI-8\n      repository: Memorithm/SciRust\n      merged_commit: {SHA_A}\n"
        );
        assert!(selected_requirements("ordinary issue", &policy)
            .unwrap()
            .is_none());

        let opted = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: TDI-8 -->";
        let (programme, requirements) = selected_requirements(opted, &policy)
            .unwrap()
            .expect("declared programme should select");
        assert_eq!(programme, "TDI-8");
        assert_eq!(requirements[0].repository(), "Memorithm/SciRust");

        let other = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: PROOF-1 -->";
        assert!(selected_requirements(other, &policy).unwrap().is_none());
    }

    #[test]
    fn defer_reason_carries_provider_and_policy_identity() {
        let unresolved = UnresolvedProviderDependency::new(
            "TDI-8",
            &requirement(),
            ProviderComparison::new("main", SHA_B, "behind"),
            "policy-token",
        );
        let reason = unresolved.defer_reason();
        assert!(reason.contains("programme=TDI-8"));
        assert!(reason.contains("provider=Memorithm/Provider"));
        assert!(reason.contains(&format!("required_commit={SHA_A}")));
        assert!(reason.contains(&format!("default_head={SHA_B}")));
        assert!(reason.contains("compare_status=behind"));
        assert!(reason.contains("policy_identity=policy-token"));
    }

    #[test]
    fn object_id_validation_rejects_uppercase_and_short_values() {
        assert!(valid_object_id(SHA_A));
        assert!(!valid_object_id("DEADBEEF0123456789abcdef0123456789abcdef"));
        assert!(!valid_object_id("deadbeef"));
    }
}
