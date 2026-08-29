#!/usr/bin/env python3
from pathlib import Path

path = Path("src/main.rs")
service_path = Path("scripts/install-systemd.sh")
source = path.read_text()
service = service_path.read_text()


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)

source = replace_once(
    source,
    "mod publication;\nmod state;\nmod trajectory;\n",
    "mod merge_policy;\nmod publication;\nmod state;\nmod trajectory;\n",
    "merge policy module",
)

source = replace_once(
    source,
    '''    auto_merge: bool,
    full_validation: bool,
''',
    '''    auto_merge: bool,
    auto_merge_scope: merge_policy::AutoMergeScope,
    full_validation: bool,
''',
    "run config merge scope",
)

source = replace_once(
    source,
    '''        Ok(Self {
            organization,
            model,
''',
    '''        let auto_merge_scope = merge_policy::AutoMergeScope::parse(
            &env::var("ORCHESTRATOR_AUTO_MERGE_SCOPE")
                .unwrap_or_else(|_| "orchestrator-validated".to_owned()),
        )?;

        Ok(Self {
            organization,
            model,
''',
    "parse merge scope",
)
source = replace_once(
    source,
    '''            auto_merge: env_flag("ORCHESTRATOR_AUTO_MERGE", false),
            full_validation: env_flag("ORCHESTRATOR_FULL_VALIDATION", false),
''',
    '''            auto_merge: env_flag("ORCHESTRATOR_AUTO_MERGE", false),
            auto_merge_scope,
            full_validation: env_flag("ORCHESTRATOR_FULL_VALIDATION", false),
''',
    "store merge scope",
)

# Helpers live next to the existing PR head resolver.
anchor = '''fn pr_head_sha(repository: &str, number: u64) -> Result<String, String> {
'''
if source.count(anchor) != 1:
    raise SystemExit(f"PR head anchor: expected one, found {source.count(anchor)}")
helpers = r'''fn merge_attestation_store(config: &RunConfig) -> merge_policy::AttestationStore {
    merge_policy::AttestationStore::new(config.data_root.join("state/merge-attestations"))
}

fn pr_merge_metadata(
    repository: &str,
    number: u64,
) -> Result<merge_policy::MergeMetadata, String> {
    let number = number.to_string();
    let output = capture(
        "gh",
        &[
            "pr",
            "view",
            number.as_str(),
            "--repo",
            repository,
            "--json",
            "author,headRefName,headRefOid,baseRefName,isCrossRepository",
            "--jq",
            r#"[.author.login // "", .headRefName // "", .headRefOid // "", .baseRefName // "", (.isCrossRepository | tostring)] | @tsv"#,
        ],
    )?;
    merge_policy::MergeMetadata::parse_tsv(&output)
}

fn attest_repaired_pr_head(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
) -> Result<(), ActionFailure> {
    let local_head = capture_in_dir(workspace, "git", &["rev-parse", "HEAD"])
        .classified(state::FailureClass::Repository)?;
    let remote_head = pr_head_sha(&item.repository, item.number)
        .classified(state::FailureClass::Infrastructure)?;
    if local_head != remote_head {
        return Err(ActionFailure::new(
            state::FailureClass::Publication,
            format!(
                "validated local head {local_head} differs from remote PR head {remote_head} after push"
            ),
        ));
    }
    let attestation = merge_policy::ValidationAttestation::new(
        &item.repository,
        item.number,
        &remote_head,
        unix_timestamp(),
    )
    .classified(state::FailureClass::Infrastructure)?;
    merge_attestation_store(config)
        .save(&attestation)
        .classified(state::FailureClass::Infrastructure)?;
    println!(
        "Recorded exact-head validation attestation for {}#{} at {}",
        item.repository, item.number, remote_head
    );
    Ok(())
}

'''
source = source.replace(anchor, helpers + anchor, 1)

old_ci_tail = '''    println!("Created commit {commit_sha}");
    run_in_dir(&workspace, "git", &["push", "origin", "HEAD"])
        .classified(state::FailureClass::Publication)
}
'''
new_ci_tail = '''    println!("Created commit {commit_sha}");
    run_in_dir(&workspace, "git", &["push", "origin", "HEAD"])
        .classified(state::FailureClass::Publication)?;
    attest_repaired_pr_head(config, &workspace, item)
}
'''
source = replace_once(source, old_ci_tail, new_ci_tail, "attest repaired PR")

start = source.index("fn handle_pr_attention(")
end = source.index("\nfn runtime_preflight(", start)
new_attention = r'''fn handle_pr_attention(
    config: &RunConfig,
    repository: &Repository,
    item: &WorkItem,
) -> Result<(), ActionFailure> {
    if !config.auto_merge {
        println!(
            "{}#{} is ready for attention, but ORCHESTRATOR_AUTO_MERGE is disabled.",
            item.repository, item.number
        );
        return Ok(());
    }

    let ci_state = item.ci_state.unwrap_or(CiState::Unknown);
    if !matches!(ci_state, CiState::Passing | CiState::NoChecks) {
        return Err(ActionFailure::new(
            state::FailureClass::Validation,
            format!(
                "refusing merge for {}#{} with CI state {}",
                item.repository,
                item.number,
                ci_state.as_str()
            ),
        ));
    }

    let default_branch = repository.default_branch.as_deref().ok_or_else(|| {
        ActionFailure::new(
            state::FailureClass::Repository,
            format!("{} has no default branch", repository.name_with_owner),
        )
    })?;
    let trusted_login = authenticated_github_login()
        .classified(state::FailureClass::Infrastructure)?;
    let metadata = pr_merge_metadata(&item.repository, item.number)
        .classified(state::FailureClass::Infrastructure)?;
    metadata
        .validate_static(&trusted_login, default_branch)
        .classified(state::FailureClass::Validation)?;

    let attested_exact_head = merge_attestation_store(config)
        .matches_head(&item.repository, item.number, &metadata.head_sha)
        .classified(state::FailureClass::Infrastructure)?;
    if !merge_policy::provenance_allows_merge(
        config.auto_merge_scope,
        &metadata,
        attested_exact_head,
    ) {
        println!(
            "{}#{} is green but outside autonomous merge scope {} (head={}); leaving it for manual review.",
            item.repository,
            item.number,
            config.auto_merge_scope.as_str(),
            metadata.head_sha
        );
        return Ok(());
    }

    println!(
        "Revalidating exact merge candidate {}#{} head={} base={} scope={}",
        item.repository,
        item.number,
        metadata.head_sha,
        metadata.base_branch,
        config.auto_merge_scope.as_str()
    );
    let workspace = prepare_pr_workspace(config, &item.repository, item.number)
        .classified(state::FailureClass::Repository)?;
    validate_recovered_publication(config, &workspace, default_branch)
        .classified(state::FailureClass::Validation)?;

    let local_head = capture_in_dir(&workspace, "git", &["rev-parse", "HEAD"])
        .classified(state::FailureClass::Repository)?;
    if local_head != metadata.head_sha {
        return Err(ActionFailure::new(
            state::FailureClass::Publication,
            format!(
                "checked-out PR head changed during validation: expected {}, got {local_head}",
                metadata.head_sha
            ),
        ));
    }

    let after_validation = pr_merge_metadata(&item.repository, item.number)
        .classified(state::FailureClass::Infrastructure)?;
    if after_validation != metadata {
        return Err(ActionFailure::new(
            state::FailureClass::Publication,
            format!(
                "PR metadata changed during local validation: before={metadata:?} after={after_validation:?}"
            ),
        ));
    }

    let number = item.number.to_string();
    if item.draft {
        println!(
            "Marking {}#{} ready only after exact-head local validation",
            item.repository, item.number
        );
        let status = Command::new("gh")
            .args(["pr", "ready"])
            .arg(&number)
            .arg("--repo")
            .arg(&item.repository)
            .status()
            .map_err(|error| {
                ActionFailure::new(
                    state::FailureClass::Publication,
                    format!("failed to execute gh pr ready: {error}"),
                )
            })?;
        if !status.success() {
            return Err(ActionFailure::new(
                state::FailureClass::Publication,
                format!("gh pr ready failed for {}#{}", item.repository, item.number),
            ));
        }
    }

    let final_metadata = pr_merge_metadata(&item.repository, item.number)
        .classified(state::FailureClass::Infrastructure)?;
    if final_metadata.head_sha != metadata.head_sha
        || final_metadata.author != metadata.author
        || final_metadata.base_branch != metadata.base_branch
        || final_metadata.cross_repository != metadata.cross_repository
    {
        return Err(ActionFailure::new(
            state::FailureClass::Publication,
            format!(
                "PR changed after validation and before merge: validated={metadata:?} final={final_metadata:?}"
            ),
        ));
    }

    println!(
        "Merging {}#{} at exact validated head {}",
        item.repository, item.number, metadata.head_sha
    );
    let status = Command::new("gh")
        .args(["pr", "merge"])
        .arg(&number)
        .arg("--repo")
        .arg(&item.repository)
        .args(["--squash", "--match-head-commit"])
        .arg(&metadata.head_sha)
        .status()
        .map_err(|error| {
            ActionFailure::new(
                state::FailureClass::Publication,
                format!("failed to execute gh pr merge: {error}"),
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(ActionFailure::new(
            state::FailureClass::Publication,
            format!("gh pr merge failed for {}#{}", item.repository, item.number),
        ))
    }
}
'''
source = source[:start] + new_attention + source[end:]

source = replace_once(
    source,
    '''        WorkKind::FixCi => execute_ci_fix(config, item),
        WorkKind::PullRequest => handle_pr_attention(config, item),
        WorkKind::Issue => execute_issue(config, &snapshot.repositories, item),
''',
    '''        WorkKind::FixCi => execute_ci_fix(config, item),
        WorkKind::PullRequest => {
            let repository = repository_by_name(&snapshot.repositories, &item.repository)
                .classified(state::FailureClass::Repository)?;
            handle_pr_attention(config, repository, item)
        }
        WorkKind::Issue => execute_issue(config, &snapshot.repositories, item),
''',
    "pass repository to merge attention",
)

source = replace_once(
    source,
    '''    println!("auto merge       : {}", config.auto_merge);
    println!("full validation  : {}", config.full_validation);
''',
    '''    println!("auto merge       : {}", config.auto_merge);
    println!("auto merge scope : {}", config.auto_merge_scope.as_str());
    println!("full validation  : {}", config.full_validation);
''',
    "merge scope startup log",
)

service = replace_once(
    service,
    'ORCHESTRATOR_AUTO_MERGE=0\n',
    'ORCHESTRATOR_AUTO_MERGE=0\nORCHESTRATOR_AUTO_MERGE_SCOPE=orchestrator-validated\n',
    "service merge scope",
)

path.write_text(source)
service_path.write_text(service)
