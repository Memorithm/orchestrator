#!/usr/bin/env python3
from pathlib import Path

path = Path("src/main.rs")
source = path.read_text()


def replace_once(old: str, new: str, label: str) -> None:
    global source
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    source = source.replace(old, new, 1)


replace_once("mod state;\nmod trajectory;\n", "mod publication;\nmod state;\nmod trajectory;\n", "publication module")

replace_once(
    '''fn reject_sensitive_paths(workspace: &Path) -> Result<(), String> {
    let status = capture_in_dir(workspace, "git", &["status", "--porcelain"])?;
    for line in status.lines() {
        let path = line.get(3..).unwrap_or_default().trim();
        let normalized = path.trim_matches('"');
        let file_name = Path::new(normalized)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();

        if file_name == ".env"
            || file_name.starts_with(".env.")
            || file_name == "id_rsa"
            || file_name == "id_ed25519"
            || file_name.ends_with(".pem")
            || file_name.ends_with(".key")
        {
            return Err(format!(
                "refusing to commit potentially sensitive path: {normalized}"
            ));
        }
    }
    Ok(())
}
''',
    '''fn path_is_sensitive(path: &str) -> bool {
    let normalized = path.trim_matches('"');
    let file_name = Path::new(normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    file_name == ".env"
        || file_name.starts_with(".env.")
        || file_name == "id_rsa"
        || file_name == "id_ed25519"
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
}

fn reject_sensitive_paths(workspace: &Path) -> Result<(), String> {
    let status = capture_in_dir(workspace, "git", &["status", "--porcelain"])?;
    for line in status.lines() {
        let path = line.get(3..).unwrap_or_default().trim();
        if path_is_sensitive(path) {
            return Err(format!(
                "refusing to commit potentially sensitive path: {}",
                path.trim_matches('"')
            ));
        }
    }
    Ok(())
}

fn reject_sensitive_committed_paths(workspace: &Path, base_ref: &str) -> Result<(), String> {
    let range = format!("{base_ref}...HEAD");
    let paths = capture_in_dir(workspace, "git", &["diff", "--name-only", range.as_str()])?;
    for path in paths.lines().filter(|path| !path.trim().is_empty()) {
        if path_is_sensitive(path) {
            return Err(format!(
                "refusing to publish potentially sensitive committed path: {path}"
            ));
        }
    }
    Ok(())
}
''',
    "sensitive path helpers",
)

marker = '''fn execute_issue(
    config: &RunConfig,
    repositories: &[Repository],
    item: &WorkItem,
) -> Result<(), String> {
'''
if source.count(marker) != 1:
    raise SystemExit(f"execute_issue marker: expected 1, found {source.count(marker)}")

helpers = r'''fn issue_publication_store(config: &RunConfig) -> publication::PublicationStore {
    publication::PublicationStore::new(config.data_root.join("state/publications"))
}

fn issue_publication_key(item: &WorkItem) -> publication::PublicationKey {
    publication::PublicationKey::new(&item.repository, item.number)
}

fn open_pr_number(repository: &str) -> Result<Option<u64>, String> {
    let output = capture(
        "gh",
        &[
            "pr",
            "list",
            "--repo",
            repository,
            "--state",
            "open",
            "--limit",
            "1",
            "--json",
            "number",
            "--jq",
            ".[0].number // empty",
        ],
    )?;
    if output.trim().is_empty() {
        Ok(None)
    } else {
        output
            .trim()
            .parse::<u64>()
            .map(Some)
            .map_err(|error| format!("invalid PR number from gh: {output}: {error}"))
    }
}

fn existing_pr_for_head(repository: &str, branch: &str) -> Result<Option<String>, String> {
    let output = capture(
        "gh",
        &[
            "pr",
            "list",
            "--repo",
            repository,
            "--head",
            branch,
            "--state",
            "all",
            "--limit",
            "1",
            "--json",
            "number,state,url",
            "--jq",
            r#"if length == 0 then "" else "#\(.[0].number) \(.[0].state) \(.[0].url)" end"#,
        ],
    )?;
    if output.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(output))
    }
}

fn optional_git_ref(workspace: &Path, reference: &str) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["show-ref", "--verify", "--hash", reference])
        .output()
        .map_err(|error| format!("failed to execute git show-ref in {}: {error}", workspace.display()))?;
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map(|value| Some(value.trim().to_owned()))
            .map_err(|error| format!("invalid UTF-8 from git show-ref: {error}"));
    }
    Ok(None)
}

fn create_issue_pull_request(
    workspace: &Path,
    item: &WorkItem,
    default_branch: &str,
    branch: &str,
) -> Result<(), String> {
    if let Some(existing) = existing_pr_for_head(&item.repository, branch)? {
        println!("Publication already has PR {existing}; no duplicate will be created.");
        return Ok(());
    }

    let title = truncate_chars(
        &format!("orchestrator: {} (#{} slice)", item.title, item.number),
        200,
    );
    let pr_body = format!(
        "Automated, reviewable progress on #{} produced by Memorithm Orchestrator using local OpenCode + Ollama.\n\nThis PR intentionally does not auto-close the issue; broad missions may require multiple independently validated slices.\n\nLocal orchestrator validation completed before push.",
        item.number
    );

    println!("Creating draft PR for {branch}");
    let status = Command::new("gh")
        .current_dir(workspace)
        .args(["pr", "create", "--repo"])
        .arg(&item.repository)
        .arg("--base")
        .arg(default_branch)
        .arg("--head")
        .arg(branch)
        .arg("--draft")
        .arg("--title")
        .arg(&title)
        .arg("--body")
        .arg(&pr_body)
        .status()
        .map_err(|error| format!("failed to execute gh pr create: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("gh pr create failed".to_owned())
    }
}

fn validate_recovered_publication(
    config: &RunConfig,
    workspace: &Path,
    base_branch: &str,
) -> Result<(), String> {
    let base_ref = format!("origin/{base_branch}");
    let range = format!("{base_ref}...HEAD");
    run_in_dir(workspace, "git", &["diff", "--check", range.as_str()])?;
    reject_sensitive_committed_paths(workspace, &base_ref)?;
    validate_workspace(config, workspace)
}

fn resume_issue_publication(
    config: &RunConfig,
    repository: &Repository,
    item: &WorkItem,
    store: &publication::PublicationStore,
    key: &publication::PublicationKey,
    mut pending: publication::PendingPublication,
) -> Result<(), String> {
    let default_branch = repository
        .default_branch
        .as_deref()
        .ok_or_else(|| format!("{} has no default branch", repository.name_with_owner))?;
    if pending.base_branch != default_branch {
        return Err(format!(
            "pending publication base {} no longer matches repository default {default_branch}",
            pending.base_branch
        ));
    }

    if let Some(existing) = existing_pr_for_head(&item.repository, &pending.branch)? {
        println!("Recovered publication already has PR {existing}; clearing transaction.");
        store.clear(key)?;
        return Ok(());
    }

    let workspace = ensure_clone(config, &repository.name_with_owner)?;
    clean_and_fetch(&workspace)?;
    let local_ref = format!("refs/heads/{}", pending.branch);
    let remote_tracking_ref = format!("refs/remotes/origin/{}", pending.branch);
    let remote_branch = format!("origin/{}", pending.branch);

    if pending.phase == publication::PublicationPhase::Prepared {
        let remote_sha = optional_git_ref(&workspace, &remote_tracking_ref)?;
        if remote_sha.as_deref() == Some(pending.commit.as_str()) {
            pending.phase = publication::PublicationPhase::Pushed;
            store.save(key, &pending)?;
        } else {
            let local_sha = optional_git_ref(&workspace, &local_ref)?;
            if local_sha.as_deref() != Some(pending.commit.as_str()) {
                return Err(format!(
                    "cannot resume prepared publication {}: expected local commit {} but found {:?}",
                    pending.branch, pending.commit, local_sha
                ));
            }
            run_in_dir(
                &workspace,
                "git",
                &["checkout", "-B", pending.branch.as_str(), pending.commit.as_str()],
            )?;
            run_in_dir(
                &workspace,
                "git",
                &["push", "-u", "origin", pending.branch.as_str()],
            )?;
            pending.phase = publication::PublicationPhase::Pushed;
            store.save(key, &pending)?;
        }
    }

    let remote_sha = optional_git_ref(&workspace, &remote_tracking_ref)?;
    if remote_sha.as_deref() != Some(pending.commit.as_str()) {
        return Err(format!(
            "remote publication {} does not match expected commit {} (found {:?})",
            pending.branch, pending.commit, remote_sha
        ));
    }
    run_in_dir(
        &workspace,
        "git",
        &["checkout", "-B", pending.branch.as_str(), remote_branch.as_str()],
    )?;
    validate_recovered_publication(config, &workspace, default_branch)?;
    create_issue_pull_request(&workspace, item, default_branch, &pending.branch)?;
    store.clear(key)?;
    Ok(())
}

'''
source = source.replace(marker, helpers + marker, 1)

start = source.find(marker)
end_marker = "\nfn execute_ci_fix(config: &RunConfig, item: &WorkItem) -> Result<(), String> {"
end = source.find(end_marker, start)
if start < 0 or end < 0:
    raise SystemExit("execute_issue boundaries not found")

new_execute_issue = r'''fn execute_issue(
    config: &RunConfig,
    repositories: &[Repository],
    item: &WorkItem,
) -> Result<(), String> {
    let repository = repository_by_name(repositories, &item.repository)?;
    let default_branch = repository
        .default_branch
        .as_deref()
        .ok_or_else(|| format!("{} has no default branch", item.repository))?;
    let store = issue_publication_store(config);
    let key = issue_publication_key(item);

    if let Some(pending) = store.load(&key)? {
        println!(
            "Resuming pending publication for {}#{} from {} at {}",
            item.repository, item.number, pending.branch, pending.commit
        );
        return resume_issue_publication(config, repository, item, &store, &key, pending);
    }

    if let Some(number) = open_pr_number(&item.repository)? {
        return Err(format!(
            "repository gained open PR #{number} after triage; deferring issue work to avoid parallel mutation"
        ));
    }

    let (workspace, branch) = prepare_issue_workspace(config, repository, item.number)?;
    let body = github_body(item)?;
    run_agent(config, &workspace, &agent_prompt(item, &body))?;

    if !has_changes(&workspace)? {
        println!("Agent produced no working-tree changes; nothing will be pushed.");
        return Ok(());
    }

    reject_sensitive_paths(&workspace)?;
    validate_workspace(config, &workspace)?;
    let message = format!("feat: progress issue #{}", item.number);
    let commit_sha = commit_changes(&workspace, &message)?;
    println!("Created commit {commit_sha}");

    let mut pending = publication::PendingPublication::new(
        branch.clone(),
        commit_sha,
        default_branch.to_owned(),
        publication::PublicationPhase::Prepared,
    )?;
    store.save(&key, &pending)?;
    println!("Publication transaction prepared for {branch}");

    run_in_dir(
        &workspace,
        "git",
        &["push", "-u", "origin", branch.as_str()],
    )?;
    pending.phase = publication::PublicationPhase::Pushed;
    store.save(&key, &pending)?;
    println!("Publication transaction recorded pushed commit {}", pending.commit);

    create_issue_pull_request(&workspace, item, default_branch, &branch)?;
    store.clear(&key)?;
    Ok(())
}
'''
source = source[:start] + new_execute_issue + source[end:]

replace_once(
    '''    #[test]
    fn opencode_policy_denies_direct_git_and_github_mutations() {
''',
    '''    #[test]
    fn sensitive_path_policy_covers_common_secret_material() {
        assert!(path_is_sensitive(".env"));
        assert!(path_is_sensitive("config/.env.production"));
        assert!(path_is_sensitive("keys/id_ed25519"));
        assert!(path_is_sensitive("tls/server.pem"));
        assert!(path_is_sensitive("tls/server.key"));
        assert!(!path_is_sensitive("src/lib.rs"));
        assert!(!path_is_sensitive("docs/key-management.md"));
    }

    #[test]
    fn opencode_policy_denies_direct_git_and_github_mutations() {
''',
    "sensitive path tests",
)

path.write_text(source)
