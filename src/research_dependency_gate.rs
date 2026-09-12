use std::process::Command;

use crate::research_eval::{
    ProviderComparison, UnresolvedProviderDependency, default_history_satisfies,
    selected_requirements, valid_object_id,
};

const MAX_DEFAULT_BRANCH_BYTES: usize = 256;

/// Parent-owned live evaluation of declared research provider prerequisites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResearchDependencyGate {
    Allow,
    Defer { reason: String },
}

fn validation_error(message: impl Into<String>) -> String {
    message.into()
}

fn infrastructure_error(message: impl Into<String>) -> String {
    message.into()
}

fn gh_scalar(path: &str, jq: &str) -> Result<String, String> {
    let output = Command::new("gh")
        .args(["api", path, "--jq", jq])
        .output()
        .map_err(|error| {
            infrastructure_error(format!(
                "failed to execute parent GitHub dependency resolver for {path}: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(infrastructure_error(format!(
            "parent GitHub dependency resolver failed for {path}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let value = String::from_utf8(output.stdout).map_err(|error| {
        infrastructure_error(format!(
            "parent GitHub dependency resolver returned non-UTF-8 output for {path}: {error}"
        ))
    })?;
    let value = value.trim();
    if value.is_empty() || value.contains('\n') || value.contains('\r') {
        return Err(infrastructure_error(format!(
            "parent GitHub dependency resolver returned an invalid scalar for {path}"
        )));
    }
    Ok(value.to_owned())
}

fn resolve_provider(repository: &str, required_commit: &str) -> Result<ProviderComparison, String> {
    let default_branch = gh_scalar(&format!("repos/{repository}"), ".default_branch")?;
    if default_branch.is_empty()
        || default_branch.len() > MAX_DEFAULT_BRANCH_BYTES
        || default_branch.chars().any(char::is_control)
    {
        return Err(infrastructure_error(format!(
            "research dependency provider {repository} returned an invalid default branch"
        )));
    }

    let default_head = gh_scalar(
        &format!("repos/{repository}/git/ref/heads/{default_branch}"),
        ".object.sha",
    )?;
    if !valid_object_id(&default_head) {
        return Err(infrastructure_error(format!(
            "research dependency provider {repository} returned an invalid default-branch head"
        )));
    }

    let compare_status = gh_scalar(
        &format!("repos/{repository}/compare/{required_commit}...{default_head}"),
        ".status",
    )?;
    Ok(ProviderComparison::new(
        default_branch,
        default_head,
        compare_status,
    ))
}

fn evaluate_with_resolver<F>(
    body: &str,
    policy_context: &str,
    policy_identity: &str,
    mut resolve: F,
) -> Result<ResearchDependencyGate, String>
where
    F: FnMut(&str, &str) -> Result<ProviderComparison, String>,
{
    let Some((programme, requirements)) = selected_requirements(body, policy_context)
        .map_err(|error| validation_error(error.to_string()))?
    else {
        return Ok(ResearchDependencyGate::Allow);
    };

    for requirement in &requirements {
        let comparison = resolve(requirement.repository(), requirement.merged_commit())?;
        if default_history_satisfies(comparison.compare_status()) {
            continue;
        }
        let unresolved = UnresolvedProviderDependency::new(
            &programme,
            requirement,
            comparison,
            policy_identity,
        );
        return Ok(ResearchDependencyGate::Defer {
            reason: unresolved.defer_reason(),
        });
    }

    Ok(ResearchDependencyGate::Allow)
}

/// Evaluate declared provider prerequisites against live GitHub state.
///
/// Missing research opt-in or unmatched programme requirements are inert.
/// Unresolved but well-formed prerequisites are a non-failure deferral.
/// Malformed policy/directives and GitHub/API failures fail closed.
///
/// `policy_context` must be the parent-resolved snapshot text (not untrusted CI).
/// `policy_identity` is the consumer snapshot identity token.
pub fn evaluate(
    body: &str,
    policy_context: &str,
    policy_identity: &str,
) -> Result<ResearchDependencyGate, String> {
    evaluate_with_resolver(body, policy_context, policy_identity, resolve_provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: &str = "0123456789abcdef0123456789abcdef01234567";
    const HEAD: &str = "89abcdef0123456789abcdef0123456789abcdef";

    fn dependency_policy(programme: &str) -> String {
        format!(
            "PARENT-RESOLVED REPOSITORY POLICY SNAPSHOT\n--- referenced-policy ---\nresearch_dependencies:\n  schema_version: 1\n  requires:\n    - programme: {programme}\n      repository: Memorithm/Provider\n      merged_commit: {REQUIRED}\n--- end referenced-policy ---\n"
        )
    }

    fn research_body(programme: &str) -> String {
        format!(
            "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: {programme} -->"
        )
    }

    fn comparison(status: &str) -> ProviderComparison {
        ProviderComparison::new("main", HEAD, status)
    }

    #[test]
    fn ordinary_issue_does_not_query_providers() {
        let mut called = false;
        let decision = evaluate_with_resolver(
            "ordinary issue",
            &dependency_policy("TDI-8"),
            "policy-id",
            |_, _| {
                called = true;
                Ok(comparison("ahead"))
            },
        )
        .unwrap();
        assert_eq!(decision, ResearchDependencyGate::Allow);
        assert!(!called);
    }

    #[test]
    fn satisfied_provider_allows_the_exact_programme() {
        let mut observed = Vec::new();
        let decision = evaluate_with_resolver(
            &research_body("TDI-8"),
            &dependency_policy("TDI-8"),
            "policy-id",
            |repo, sha| {
                observed.push((repo.to_owned(), sha.to_owned()));
                Ok(comparison("identical"))
            },
        )
        .unwrap();
        assert_eq!(decision, ResearchDependencyGate::Allow);
        assert_eq!(
            observed,
            [("Memorithm/Provider".to_owned(), REQUIRED.to_owned())]
        );
    }

    #[test]
    fn unresolved_provider_is_an_explicit_deferral() {
        let decision = evaluate_with_resolver(
            &research_body("TDI-8"),
            &dependency_policy("TDI-8"),
            "policy-token-123",
            |_, _| Ok(comparison("behind")),
        )
        .unwrap();
        let ResearchDependencyGate::Defer { reason } = decision else {
            panic!("expected first-class deferral");
        };
        assert!(reason.contains("programme=TDI-8"));
        assert!(reason.contains("provider=Memorithm/Provider"));
        assert!(reason.contains(&format!("required_commit={REQUIRED}")));
        assert!(reason.contains(&format!("default_head={HEAD}")));
        assert!(reason.contains("compare_status=behind"));
        assert!(reason.contains("policy_identity=policy-token-123"));
    }

    #[test]
    fn unrelated_programme_does_not_inherit_dependency() {
        let mut called = false;
        let decision = evaluate_with_resolver(
            &research_body("PROOF-1"),
            &dependency_policy("TDI-8"),
            "policy-id",
            |_, _| {
                called = true;
                Ok(comparison("behind"))
            },
        )
        .unwrap();
        assert_eq!(decision, ResearchDependencyGate::Allow);
        assert!(!called);
    }

    #[test]
    fn provider_resolver_failure_fails_closed() {
        let error = evaluate_with_resolver(
            &research_body("TDI-8"),
            &dependency_policy("TDI-8"),
            "policy-id",
            |_, _| Err("provider unavailable".to_owned()),
        )
        .unwrap_err();
        assert!(error.contains("provider unavailable"));
    }

    #[test]
    fn malformed_dependency_policy_fails_closed() {
        let error = evaluate_with_resolver(
            &research_body("TDI-8"),
            "research_dependencies:\n  schema_version: 99\n  requires:\n",
            "policy-id",
            |_, _| Ok(comparison("ahead")),
        )
        .unwrap_err();
        assert!(error.contains("research dependency policy rejected"));
    }
}
