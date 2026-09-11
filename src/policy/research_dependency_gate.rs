use std::process::Command;

use orchestrator::research_dependency::{
    ResearchDependencyPlan, parse_roadmap_research_dependencies,
};

use super::{PolicyDenial, PolicySnapshot};

const MAX_DEFAULT_BRANCH_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderState {
    default_branch: String,
    default_head: String,
    compare_status: String,
}

fn gh_scalar(path: &str, jq: &str) -> Result<String, String> {
    let output = Command::new("gh")
        .args(["api", path, "--jq", jq])
        .output()
        .map_err(|error| format!("failed to execute parent GitHub dependency resolver: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "parent GitHub dependency resolver failed for {path}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("invalid UTF-8 from parent GitHub dependency resolver: {error}"))?;
    let value = value.trim();
    if value.is_empty() || value.contains('\n') || value.contains('\r') {
        return Err(format!(
            "parent GitHub dependency resolver returned an invalid scalar for {path}"
        ));
    }
    Ok(value.to_owned())
}

fn valid_git_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn resolve_provider(repository: &str, required_commit: &str) -> Result<ProviderState, String> {
    let default_branch = gh_scalar(&format!("repos/{repository}"), ".default_branch")?;
    if default_branch.is_empty()
        || default_branch.len() > MAX_DEFAULT_BRANCH_BYTES
        || default_branch.chars().any(char::is_control)
    {
        return Err(format!(
            "provider {repository} returned an invalid default branch"
        ));
    }

    let default_head = gh_scalar(
        &format!("repos/{repository}/git/ref/heads/{default_branch}"),
        ".object.sha",
    )?;
    if !valid_git_sha(&default_head) {
        return Err(format!(
            "provider {repository} returned an invalid default-branch head"
        ));
    }

    let compare_status = gh_scalar(
        &format!("repos/{repository}/compare/{required_commit}...{default_head}"),
        ".status",
    )?;

    Ok(ProviderState {
        default_branch,
 default_head,
        compare_status,
    })
}

fn provider_satisfies_requirement(state: &ProviderState) -> bool {
    matches!(state.compare_status.as_str(), "ahead" | "identical")
}

fn selected_plan(policy_context: &str) -> Result<Option<ResearchDependencyPlan>, String> {
    parse_roadmap_research_dependencies(policy_context)
        .map_err(|error| format!("research dependency policy rejected: {error}"))
}

fn eligibility_with_resolver<F>(
    policy_identity: &str,
    policy_context: &str,
    body: &str,
    mut resolve: F,
) -> Result<Option<PolicyDenial>, String>
where
    F: FnMut(&str, &str) -> Result<ProviderState, String>,
{
    let Some(directive) = orchestrator::research::parse_issue_directive(body)
        .map_err(|error| format!("autonomous research directive rejected: {error}"))?
    else {
        return Ok(None);
    };
    let Some(programme) = directive.programme() else {
        return Ok(None);
    };
    let Some(plan) = selected_plan(policy_context)? else {
        return Ok(None);
    };

    for requirement in plan.requirements_for(programme) {
        let provider = requirement.repository();
        let required_commit = requirement.merged_commit();
        let state = resolve(provider, required_commit)?;
        if provider_satisfies_requirement(&state) {
            continue;
        }

        let detail = format!(
            "research dependency deferred programme={programme} provider={provider} required_commit={required_commit} default_branch={} default_head={} compare_status={} policy_identity={policy_identity}",
            state.default_branch, state.default_head, state.compare_status
        );
        return Ok(Some(PolicyDenial::research_dependency(detail)));
    }

    Ok(None)
}

pub(super) fn eligibility(
    snapshot: &PolicySnapshot,
    body: &str,
) -> Result<Option<PolicyDenial>, String> {
    let policy_identity = snapshot.identity_token();
    let policy_context = snapshot.prompt_context();
    eligibility_with_resolver(
        &policy_identity,
        &policy_context,
        body,
        resolve_provider,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: &str = "0123456789abcdef0123456789abcdef01234567";
    const HEAD: &str = "89abcdef0123456789abcdef0123456789abcdef";

    fn dependency_policy(programme: &str) -> String {
        format!(
            "PARENT-RESOLVED REPOSITORY POLICY SNAPSHOT\n--- referenced-policy ---\nresearch_dependencies:\n  schema_version: 1\n  requires:\n    - programme: {programme}\n      repository: Memorithm/Provider\n      merged_commit: {REQUIRED}\n--- end policy document ---\n"
        )
    }

    fn research_body(programme: &str) -> String {
        format!(
            "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: {programme} -->"
        )
    }

    fn provider(status: &str) -> ProviderState {
        ProviderState {
            default_branch: "main".to_owned(),
            default_head: HEAD.to_owned(),
            compare_status: status.to_owned(),
        }
    }

    #[test]
    fn ordinary_issue_is_inert() {
        let mut called = false;
        let result = eligibility_with_resolver(
            "policy-id",
            &dependency_policy("TDI-8"),
            "ordinary issue",
            |_, _| {
                called = true;
                Ok(provider("ahead"))
            },
        )
        .unwrap();
        assert!(result.is_none());
        assert!(!called);
    }

    #[test]
    fn exact_programme_requires_exact_provider_commit() {
        let mut observed = Vec::new();
        let result = eligibility_with_resolver(
            "policy-id",
            &dependency_policy("TDI-8"),
            &research_body("TDI-8"),
            |repo, sha| {
                observed.push((repo.to_owned(), sha.to_owned()));
                Ok(provider("ahead"))
            },
        )
        .unwrap();
        assert!(result.is_none());
        assert_eq!(
            observed,
            [("Memorithm/Provider".to_owned(), REQUIRED.to_owned())]
        );
    }

    #[test]
    fn unrelated_programme_does_not_inherit_dependency() {
        let mut called = false;
        let result = eligibility_with_resolver(
            "policy-id",
            &dependency_policy("TDI-8"),
            &research_body("PROOF-1"),
            |_, _| {
                called = true;
                Ok(provider("ahead"))
            },
        )
        .unwrap();
        assert!(result.is_none());
        assert!(!called);
    }

    #[test]
    fn unresolved_provider_becomes_policy_bound_deferral() {
        let denial = eligibility_with_resolver(
            "policy-id-123",
            &dependency_policy("TDI-8"),
            &research_body("TDI-8"),
            |_, _| Ok(provider("behind")),
        )
        .unwrap()
        .expect("dependency should defer");
        let detail = denial.research_detail().expect("research detail");
        assert!(detail.contains("programme=TDI-8"));
        assert!(detail.contains("provider=Memorithm/Provider"));
        assert!(detail.contains("compare_status=behind"));
        assert!(detail.contains("policy_identity=policy-id-123"));
    }

    #[test]
    fn provider_error_fails_closed() {
        let error = eligibility_with_resolver(
            "policy-id",
            &dependency_policy("TDI-8"),
            &research_body("TDI-8"),
            |_, _| Err("provider unavailable".to_owned()),
        )
        .unwrap_err();
        assert!(error.contains("provider unavailable"));
    }

    #[test]
    fn duplicate_dependency_sections_fail_closed() {
        let policy = format!(
            "{}\n{}",
            dependency_policy("TDI-8"),
            dependency_policy("TDI-8")
        );
        assert!(
            eligibility_with_resolver(
                "policy-id",
                &policy,
                &research_body("TDI-8"),
                |_, _| Ok(provider("ahead")),
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_dependency_section_fails_closed() {
        assert!(
            eligibility_with_resolver(
                "policy-id",
                "research_dependencies:\n  schema_version: 99\n  requires:\n",
                &research_body("TDI-8"),
                |_, _| Ok(provider("ahead")),
            )
            .is_err()
        );
    }

    #[test]
    fn only_default_history_success_states_satisfy() {
        assert!(provider_satisfies_requirement(&provider("ahead")));
        assert!(provider_satisfies_requirement(&provider("identical")));
        assert!(!provider_satisfies_requirement(&provider("behind")));
        assert!(!provider_satisfies_requirement(&provider("diverged")));
        assert!(!provider_satisfies_requirement(&provider("unknown")));
    }
}
