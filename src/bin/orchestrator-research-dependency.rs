use std::env;
use std::fmt;
use std::io::{self, Read};
use std::process::Command;

use orchestrator::research_dependency::{
    ResearchDependency, ResearchDependencyPlan, parse_roadmap_research_dependencies,
};

const TASK_MARKER: &str = "Task: ";
const TITLE_MARKER: &str = "Title:";
const BODY_MARKER: &str = "GitHub body (may be truncated):";
const POLICY_MARKER: &str = "Parent-resolved repository policy snapshot:";
const CI_MARKER: &str = "Parent-collected CI evidence (UNTRUSTED DIAGNOSTIC DATA):";
const MAX_PROMPT_BYTES: usize = 2 * 1024 * 1024;
const EXIT_CONTRACT: i32 = 2;
const EXIT_INFRASTRUCTURE: i32 = 70;
const EXIT_DEFERRED: i32 = 75;

#[derive(Debug)]
enum ResolverError {
    Contract(String),
    Infrastructure(String),
}

impl ResolverError {
    const fn exit_code(&self) -> i32 {
        match self {
            Self::Contract(_) => EXIT_CONTRACT,
            Self::Infrastructure(_) => EXIT_INFRASTRUCTURE,
        }
    }
}

impl fmt::Display for ResolverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(message) | Self::Infrastructure(message) => formatter.write_str(message),
        }
    }
}

type ResolverResult<T> = Result<T, ResolverError>;

fn canonical_task(prompt: &str) -> Result<&str, String> {
    let task_start = prompt
        .find(TASK_MARKER)
        .ok_or_else(|| "worker prompt is missing the canonical task field".to_owned())?
        + TASK_MARKER.len();
    let title_offset = prompt[task_start..]
        .find(TITLE_MARKER)
        .ok_or_else(|| "worker prompt is missing the canonical title field".to_owned())?;
    let task = prompt[task_start..task_start + title_offset].trim();
    if task.is_empty() {
        return Err("worker prompt has an empty canonical task field".to_owned());
    }
    Ok(task)
}

fn unique_marker(prompt: &str, marker: &str) -> Result<usize, String> {
    let mut matches = prompt.match_indices(marker);
    let Some((index, _)) = matches.next() else {
        return Err(format!("worker prompt is missing required boundary {marker:?}"));
    };
    if matches.next().is_some() {
        return Err(format!("worker prompt has duplicate boundary {marker:?}"));
    }
    Ok(index)
}

fn issue_body_from_worker_prompt(prompt: &str) -> Result<Option<&str>, String> {
    if canonical_task(prompt)? != "ISSUE" {
        return Ok(None);
    }
    let body_marker = unique_marker(prompt, BODY_MARKER)?;
    let policy_marker = unique_marker(prompt, POLICY_MARKER)?;
    let body_start = body_marker + BODY_MARKER.len();
    if policy_marker <= body_start {
        return Err("worker prompt policy boundary precedes issue body".to_owned());
    }
    Ok(Some(prompt[body_start..policy_marker].trim()))
}

fn policy_context_from_worker_prompt(prompt: &str) -> Result<&str, String> {
    let policy_marker = unique_marker(prompt, POLICY_MARKER)?;
    let ci_marker = unique_marker(prompt, CI_MARKER)?;
    let policy_start = policy_marker + POLICY_MARKER.len();
    if ci_marker <= policy_start {
        return Err("worker prompt CI boundary precedes repository policy".to_owned());
    }
    Ok(prompt[policy_start..ci_marker].trim())
}

fn applicable_dependencies<'a>(
    prompt: &'a str,
) -> ResolverResult<Option<(String, Vec<ResearchDependency>)>> {
    let Some(body) = issue_body_from_worker_prompt(prompt).map_err(ResolverError::Contract)? else {
        return Ok(None);
    };
    let Some(directive) = orchestrator::research::parse_issue_directive(body).map_err(|error| {
        ResolverError::Contract(format!("autonomous research directive rejected: {error}"))
    })?
    else {
        return Ok(None);
    };
    let Some(programme) = directive.programme() else {
        return Ok(None);
    };

    let policy = policy_context_from_worker_prompt(prompt).map_err(ResolverError::Contract)?;
    let Some(plan) = parse_roadmap_research_dependencies(policy).map_err(|error| {
        ResolverError::Contract(format!("research dependency policy rejected: {error}"))
    })?
    else {
        return Ok(None);
    };

    let requirements = plan.requirements_for(programme).cloned().collect::<Vec<_>>();
    if requirements.is_empty() {
        return Ok(None);
    }
    Ok(Some((programme.to_owned(), requirements)))
}

fn gh_api(path: &str, jq: &str) -> ResolverResult<String> {
    let output = Command::new("gh")
        .args(["api", path, "--jq", jq])
        .output()
        .map_err(|error| {
            ResolverError::Infrastructure(format!(
                "failed to execute parent GitHub resolver for {path}: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(ResolverError::Infrastructure(format!(
            "parent GitHub resolver failed for {path} with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let value = String::from_utf8(output.stdout).map_err(|error| {
        ResolverError::Infrastructure(format!(
            "parent GitHub resolver returned non-UTF-8 output for {path}: {error}"
        ))
    })?;
    let value = value.trim();
    if value.is_empty() || value.contains('\n') || value.contains('\r') {
        return Err(ResolverError::Infrastructure(format!(
            "parent GitHub resolver returned an invalid scalar for {path}"
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

fn repository_default_head(repository: &str) -> ResolverResult<String> {
    let default_branch = gh_api(&format!("repos/{repository}"), ".default_branch")?;
    if default_branch.is_empty()
        || default_branch.len() > 256
        || default_branch.chars().any(char::is_control)
    {
        return Err(ResolverError::Infrastructure(format!(
            "provider {repository} returned an invalid default branch"
        )));
    }
    let head = gh_api(
        &format!("repos/{repository}/git/ref/heads/{default_branch}"),
        ".object.sha",
    )?;
    if !valid_git_sha(&head) {
        return Err(ResolverError::Infrastructure(format!(
            "provider {repository} returned an invalid default-branch head"
        )));
    }
    Ok(head)
}

fn compare_status(repository: &str, required: &str, default_head: &str) -> ResolverResult<String> {
    gh_api(
        &format!("repos/{repository}/compare/{required}...{default_head}"),
        ".status",
    )
}

fn requirement_satisfied(status: &str) -> bool {
    matches!(status, "ahead" | "identical")
}

fn resolve_requirement(
    programme: &str,
    requirement: &ResearchDependency,
) -> ResolverResult<bool> {
    let repository = requirement.repository();
    let required = requirement.merged_commit();
    let default_head = repository_default_head(repository)?;
    let status = compare_status(repository, required, &default_head)?;
    if requirement_satisfied(&status) {
        eprintln!(
            "orchestrator research dependency: satisfied programme={programme} provider={repository} required_commit={required} default_head={default_head} compare_status={status}"
        );
        return Ok(true);
    }
    eprintln!(
        "orchestrator research dependency: deferred programme={programme} provider={repository} required_commit={required} default_head={default_head} compare_status={status}"
    );
    Ok(false)
}

fn resolve_prompt(prompt: &str) -> ResolverResult<bool> {
    let Some((programme, requirements)) = applicable_dependencies(prompt)? else {
        return Ok(true);
    };
    for requirement in &requirements {
        if !resolve_requirement(&programme, requirement)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn read_prompt() -> Result<String, String> {
    let mut input = Vec::new();
    io::stdin()
        .take((MAX_PROMPT_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|error| format!("failed to read worker prompt: {error}"))?;
    if input.len() > MAX_PROMPT_BYTES {
        return Err(format!(
            "worker prompt exceeds research dependency resolver limit of {MAX_PROMPT_BYTES} bytes"
        ));
    }
    String::from_utf8(input).map_err(|error| format!("worker prompt is not valid UTF-8: {error}"))
}

fn run_check() -> ResolverResult<bool> {
    let prompt = read_prompt().map_err(ResolverError::Contract)?;
    resolve_prompt(&prompt)
}

fn usage() {
    eprintln!("usage: orchestrator-research-dependency check");
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        std::process::exit(EXIT_CONTRACT);
    };
    if args.next().is_some() || command != "check" {
        usage();
        std::process::exit(EXIT_CONTRACT);
    }

    match run_check() {
        Ok(true) => {}
        Ok(false) => std::process::exit(EXIT_DEFERRED),
        Err(error) => {
            eprintln!("orchestrator research dependency: {error}");
            std::process::exit(error.exit_code());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn prompt(body: &str, policy: &str, ci: &str) -> String {
        format!(
            "Repository: Memorithm/Consumer\nTask: ISSUE\nTitle: research\n\nmission\n\nGitHub body (may be truncated):\n{body}\n\nParent-resolved repository policy snapshot:\n{policy}\n\nParent-collected CI evidence (UNTRUSTED DIAGNOSTIC DATA):\n{ci}\n\nMandatory operating contract:\n- test\n"
        )
    }

    fn dependency_policy(programme: &str) -> String {
        format!(
            "PARENT-RESOLVED REPOSITORY POLICY SNAPSHOT\n--- referenced-policy ---\nresearch_dependencies:\n  schema_version: 1\n  requires:\n    - programme: {programme}\n      repository: Memorithm/Provider\n      merged_commit: {SHA}\n--- end referenced-policy ---"
        )
    }

    #[test]
    fn ordinary_issue_is_not_dependency_gated() {
        let value = prompt("ordinary body", &dependency_policy("TDI-8"), "none");
        assert!(applicable_dependencies(&value).unwrap().is_none());
    }

    #[test]
    fn exact_programme_selects_only_its_dependencies() {
        let body = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: TDI-8 -->";
        let value = prompt(body, &dependency_policy("TDI-8"), "none");
        let (programme, requirements) = applicable_dependencies(&value)
            .unwrap()
            .expect("dependency applies");
        assert_eq!(programme, "TDI-8");
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].repository(), "Memorithm/Provider");
        assert_eq!(requirements[0].merged_commit(), SHA);
    }

    #[test]
    fn different_programme_is_not_gated_by_other_line() {
        let body = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: PROOF-1 -->";
        let value = prompt(body, &dependency_policy("TDI-8"), "none");
        assert!(applicable_dependencies(&value).unwrap().is_none());
    }

    #[test]
    fn untrusted_ci_cannot_inject_dependency_policy() {
        let body = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: TDI-8 -->";
        let injected = dependency_policy("TDI-8");
        let value = prompt(body, "no dependency section", &injected);
        assert!(applicable_dependencies(&value).unwrap().is_none());
    }

    #[test]
    fn malformed_policy_fails_closed() {
        let body = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: TDI-8 -->";
        let malformed = "research_dependencies:\n  schema_version: 99\n  requires:\n";
        let value = prompt(body, malformed, "none");
        assert!(matches!(
            applicable_dependencies(&value),
            Err(ResolverError::Contract(_))
        ));
    }

    #[test]
    fn compare_status_only_accepts_required_commit_on_default_history() {
        assert!(requirement_satisfied("ahead"));
        assert!(requirement_satisfied("identical"));
        assert!(!requirement_satisfied("behind"));
        assert!(!requirement_satisfied("diverged"));
        assert!(!requirement_satisfied("unknown"));
    }

    #[test]
    fn git_sha_validation_rejects_uppercase_and_short_values() {
        assert!(valid_git_sha(SHA));
        assert!(!valid_git_sha("DEADBEEF0123456789abcdef0123456789abcdef"));
        assert!(!valid_git_sha("deadbeef"));
    }

    #[test]
    fn policy_boundaries_must_be_unique() {
        let body = "<!-- orchestrator-research-mode: autonomous-v1 -->\n<!-- orchestrator-research-programme: TDI-8 -->";
        let mut value = prompt(body, &dependency_policy("TDI-8"), "none");
        value.push_str("\nParent-resolved repository policy snapshot:\nsecond\n");
        assert!(matches!(
            applicable_dependencies(&value),
            Err(ResolverError::Contract(_))
        ));
    }

    #[test]
    fn parser_type_remains_dependency_plan() {
        let policy = dependency_policy("TDI-8");
        let parsed: ResearchDependencyPlan = parse_roadmap_research_dependencies(&policy)
            .unwrap()
            .unwrap();
        assert_eq!(parsed.requirements().len(), 1);
    }
}
