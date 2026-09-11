use std::process::Command;

use orchestrator::research_dependency::{
    ResearchDependencyPlan, parse_roadmap_research_dependencies,
};

use super::{PolicyDenial, PolicyDocument, PolicySnapshot};

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

fn selected_plan<'a>(
    snapshot: &'a PolicySnapshot,
) -> Result<Option<(&'a PolicyDocument, ResearchDependencyPlan)>, String> {
    let mut selected = None;
    for document in &snapshot.documents {
        let Some(plan) = parse_roadmap_research_dependencies(&document.content).map_err(|error| {
            format!(
                "research dependency policy rejected in origin/{}:{}: {error}",
                document.ref_name, document.path
            )
        })?
        else {
            continue;
        };
        if selected.replace((document, plan)).is_some() {
            return Err(
                "duplicate research_dependencies sections across mandatory policy documents"
                    .to_owned(),
            );
        }
    }
    Ok(selected)
}

fn eligibility_with_resolver<F>(
    snapshot: &PolicySnapshot,
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
    let Some((document, plan)) = selected_plan(snapshot)? else {
        return Ok(None);
    };

    for requirement in plan.requirements_for(programme) {
        let provider = requirement.repository();
        let required_commit = requirement.merged_commit();
        let state = resolve(provider, required_commit)?;
        if provider_satisfies_requirement(&state) {
            continue;
        }

        return Ok(Some(PolicyDenial {
            item_id: format!("research:{programme}:{provider}"),
            field: "research_dependency",
            value: format!(
                "provider={provider} required_commit={required_commit} default_branch={} default_head={} compare_status={}",
                state.default_branch, state.default_head, state.compare_status
            ),
            source_ref: document.ref_name.clone(),
            source_path: document.path.clone(),
            source_commit: document.commit_sha.clone(),
            source_blob: document.blob_sha.clone(),
        }));
    }

    Ok(None)
}

pub(super) fn eligibility(
    snapshot: &PolicySnapshot,
    body: &str,
) -> Result<Option<PolicyDenial>, String> {
    eligibility_with_resolver(snapshot, body, resolve_provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: &str = "0123456789abcdef0123456789abcdef01234567";
    const HEAD: &str = "89abcdef0123456789abcdef0123456789abcdef";

    fn snapshot(contents: &[&str]) -> PolicySnapshot {
        PolicySnapshot {
            repository: "Memorithm/Consumer".to_owned(),
            base_branch: "main".to_owned(),
            base_sha: "0".repeat(40),
            bootstrap: None,
            documents: contents
                .iter()
                .enumerate()
                .map(|(index, content)| PolicyDocument {
                    ref_name: format!("agent/policy-{index}"),
                    path: format!(".agent/POLICY-{index}.yaml"),
                    commit_sha: format!("{:040x}", index + 1),
                    blob_sha: format!("{:040x}", index + 101),
                    content: (*content).to_owned(),
                })
                .collect(),
        }
    }

    fn dependency_policy(programme: &str) -> String {
        format!(
            "research_dependencies:\n  schema_version: 1\n  requires:\n    - programme: {programme}\n      repository: Memorithm/Provider\n      merged_commit: {REQUIRED}\n"
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
        let policy = dependency_policy("TDI-8");
        let snapshot = snapshot(&[&policy]);
        let mut called = false;
        let result = eligibility_with_resolver(&snapshot, "ordinary issue", |_, _| {
            called = true;
            Ok(provider("ahead"))
        })
        .unwrap();
        assert!(result.is_none());
        assert!(!called);
    }

    #[test]
    fn exact_programme_requires_exact_provider_commit() {
        let policy = dependency_policy("TDI-8");
        let snapshot = snapshot(&[&policy]);
        let mut observed = Vec::new();
        let result = eligibility_with_resolver(&snapshot, &research_body("TDI-8"), |repo, sha| {
            observed.push((repo.to_owned(), sha.to_owned()));
            Ok(provider("ahead"))
        })
        .unwrap();
        assert!(result.is_none());
        assert_eq!(
            observed,
            [("Memorithm/Provider".to_owned(), REQUIRED.to_owned())]
        );
    }

    #[test]
    fn unrelated_programme_does_not_inherit_dependency() {
        let policy = dependency_policy("TDI-8");
        let snapshot = snapshot(&[&policy]);
        let mut called = false;
        let result = eligibility_with_resolver(&snapshot, &research_body("PROOF-1"), |_, _| {
            called = true;
            Ok(provider("ahead"))
        })
        .unwrap();
        assert!(result.is_none());
        assert!(!called);
    }

    #[test]
    fn unresolved_provider_becomes_source_bound_policy_denial() {
        let policy = dependency_policy("TDI-8");
        let snapshot = snapshot(&[&policy]);
        let denial = eligibility_with_resolver(&snapshot, &research_body("TDI-8"), |_, _| {
            Ok(provider("behind"))
        })
        .unwrap()
        .expect("dependency should defer");
        assert_eq!(denial.field, "research_dependency");
        assert!(denial.item_id.contains("TDI-8"));
        assert!(denial.value.contains("compare_status=behind"));
        assert_eq!(denial.source_ref, "agent/policy-0");
        assert_eq!(denial.source_path, ".agent/POLICY-0.yaml");
    }

    #[test]
    fn provider_error_fails_closed() {
        let policy = dependency_policy("TDI-8");
        let snapshot = snapshot(&[&policy]);
        let error = eligibility_with_resolver(&snapshot, &research_body("TDI-8"), |_, _| {
            Err("provider unavailable".to_owned())
        })
        .unwrap_err();
        assert!(error.contains("provider unavailable"));
    }

    #[test]
    fn duplicate_dependency_sections_fail_closed() {
        let policy = dependency_policy("TDI-8");
        let snapshot = snapshot(&[&policy, &policy]);
        let error = eligibility_with_resolver(&snapshot, &research_body("TDI-8"), |_, _| {
            Ok(provider("ahead"))
        })
        .unwrap_err();
        assert!(error.contains("duplicate research_dependencies"));
    }

    #[test]
    fn malformed_dependency_section_fails_closed() {
        let snapshot = snapshot(&[
            "research_dependencies:\n  schema_version: 99\n  requires:\n",
        ]);
        assert!(
            eligibility_with_resolver(&snapshot, &research_body("TDI-8"), |_, _| {
                Ok(provider("ahead"))
            })
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
