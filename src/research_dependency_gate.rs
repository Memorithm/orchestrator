use orchestrator::research_dependency::{ResearchDependency, parse_roadmap_research_dependencies};

use crate::{ActionExecution, ActionFailure, WorkItem, capture, policy, state};

const MAX_DEFAULT_BRANCH_BYTES: usize = 256;

fn validation_failure(message: impl Into<String>) -> ActionFailure {
    ActionFailure::new(state::FailureClass::Validation, message)
}

fn infrastructure_failure(message: impl Into<String>) -> ActionFailure {
    ActionFailure::new(state::FailureClass::Infrastructure, message)
}

fn selected_dependencies(
    body: &str,
    policy_context: &str,
) -> Result<Option<(String, Vec<ResearchDependency>)>, ActionFailure> {
    let Some(directive) = orchestrator::research::parse_issue_directive(body).map_err(|error| {
        validation_failure(format!("autonomous research directive rejected: {error}"))
    })?
    else {
        return Ok(None);
    };
    let Some(programme) = directive.programme() else {
        return Ok(None);
    };

    let Some(plan) = parse_roadmap_research_dependencies(policy_context).map_err(|error| {
        validation_failure(format!("research dependency policy rejected: {error}"))
    })?
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

fn gh_scalar(path: &str, jq: &str) -> Result<String, ActionFailure> {
    let value = capture("gh", &["api", path, "--jq", jq]).map_err(infrastructure_failure)?;
    let value = value.trim();
    if value.is_empty() || value.contains('\n') || value.contains('\r') {
        return Err(infrastructure_failure(format!(
            "research dependency resolver returned an invalid scalar for {path}"
        )));
    }
    Ok(value.to_owned())
}

fn valid_git_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn repository_default_head(repository: &str) -> Result<(String, String), ActionFailure> {
    let default_branch = gh_scalar(&format!("repos/{repository}"), ".default_branch")?;
    if default_branch.is_empty()
        || default_branch.len() > MAX_DEFAULT_BRANCH_BYTES
        || default_branch.chars().any(char::is_control)
    {
        return Err(infrastructure_failure(format!(
            "research dependency provider {repository} returned an invalid default branch"
        )));
    }

    let head = gh_scalar(
        &format!("repos/{repository}/git/ref/heads/{default_branch}"),
        ".object.sha",
    )?;
    if !valid_git_sha(&head) {
        return Err(infrastructure_failure(format!(
            "research dependency provider {repository} returned an invalid default-branch head"
        )));
    }
    Ok((default_branch, head))
}

fn compare_status(
    repository: &str,
    required_commit: &str,
    default_head: &str,
) -> Result<String, ActionFailure> {
    gh_scalar(
        &format!("repos/{repository}/compare/{required_commit}...{default_head}"),
        ".status",
    )
}

fn requirement_satisfied(status: &str) -> bool {
    matches!(status, "ahead" | "identical")
}

pub(crate) fn enforce(
    item: &WorkItem,
    body: &str,
    policy_snapshot: &policy::PolicySnapshot,
) -> Result<Option<ActionExecution>, ActionFailure> {
    let policy_context = policy_snapshot.prompt_context();
    let Some((programme, requirements)) = selected_dependencies(body, &policy_context)? else {
        return Ok(None);
    };

    for requirement in &requirements {
        let provider = requirement.repository();
        let required_commit = requirement.merged_commit();
        let (default_branch, default_head) = repository_default_head(provider)?;
        let status = compare_status(provider, required_commit, &default_head)?;

        if requirement_satisfied(&status) {
            println!(
                "research dependency: SATISFIED consumer={}#{} programme={} provider={} required_commit={} default_branch={} default_head={} compare_status={}",
                item.repository,
                item.number,
                programme,
                provider,
                required_commit,
                default_branch,
                default_head,
                status
            );
            continue;
        }

        let reason = format!(
            "research dependency deferred consumer={}#{} programme={} provider={} required_commit={} default_branch={} default_head={} compare_status={} policy_identity={}",
            item.repository,
            item.number,
            programme,
            provider,
            required_commit,
            default_branch,
            default_head,
            status,
            policy_snapshot.identity_token()
        );
        println!("research dependency: DEFERRED {reason}");
        return Ok(Some(ActionExecution::deferred(reason)));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn dependency_policy(programme: &str) -> String {
        format!(
            "research_dependencies:\n  schema_version: 1\n  requires:\n    - programme: {programme}\n      repository: Memorithm/Provider\n      merged_commit: {SHA}\n"
        )
    }

    #[test]
    fn ordinary_issue_has_no_scheduler_dependency_gate() {
        assert!(
            selected_dependencies("ordinary issue", &dependency_policy("TDI-8"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn exact_programme_selects_declared_provider() {
        let body = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: TDI-8 -->";
        let (programme, requirements) = selected_dependencies(body, &dependency_policy("TDI-8"))
            .unwrap()
            .expect("dependency should apply");
        assert_eq!(programme, "TDI-8");
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].repository(), "Memorithm/Provider");
        assert_eq!(requirements[0].merged_commit(), SHA);
    }

    #[test]
    fn other_programme_does_not_inherit_dependency() {
        let body = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: PROOF-1 -->";
        assert!(
            selected_dependencies(body, &dependency_policy("TDI-8"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn malformed_dependency_policy_fails_closed_as_validation() {
        let body = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: TDI-8 -->";
        let error = selected_dependencies(
            body,
            "research_dependencies:\n  schema_version: 99\n  requires:\n",
        )
        .unwrap_err();
        assert_eq!(error.class, state::FailureClass::Validation);
    }

    #[test]
    fn only_default_history_compare_success_is_satisfied() {
        assert!(requirement_satisfied("ahead"));
        assert!(requirement_satisfied("identical"));
        assert!(!requirement_satisfied("behind"));
        assert!(!requirement_satisfied("diverged"));
        assert!(!requirement_satisfied("unknown"));
    }

    #[test]
    fn commit_validation_is_exact_and_lowercase() {
        assert!(valid_git_sha(SHA));
        assert!(!valid_git_sha("deadbeef"));
        assert!(!valid_git_sha("DEADBEEF0123456789abcdef0123456789abcdef"));
    }
}
