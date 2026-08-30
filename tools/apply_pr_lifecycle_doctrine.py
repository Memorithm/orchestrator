from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if text.count(old) != 1:
        raise SystemExit(f"expected exactly one match for {label}, found {text.count(old)}")
    return text.replace(old, new, 1)


# ---------------------------------------------------------------------------
# Rust runtime policy. Read once, transform in memory, write once.
# ---------------------------------------------------------------------------
main_path = Path("src/main.rs")
main = main_path.read_text()

main = replace_once(
    main,
    '''const DEFAULT_ORGANIZATION: &str = "Memorithm";
const DEFAULT_MODEL: &str = "ollama/qwen3.8:latest";
const DEFAULT_INTERVAL_SECS: u64 = 180;''',
    '''const DEFAULT_ORGANIZATION: &str = "Memorithm";
const DEFAULT_MODEL: &str = "ollama/qwen3.8:latest";
const DEFAULT_INTERVAL_SECS: u64 = 180;
const AUTONOMOUS_GIT_NAME: &str = "ZEKRITI Tarek";
const AUTONOMOUS_GIT_EMAIL: &str = "194770978+CHECKUPAUTO@users.noreply.github.com";''',
    "canonical git constants",
)

main = replace_once(
    main,
    '''enum WorkKind {
    FixCi,
    PullRequest,
    Issue,
    ExternalPr,
    WaitCi,
    UnknownCi,
}''',
    '''enum WorkKind {
    FixCi,
    PullRequest,
    Issue,
    ExternalPr,
    WaitCi,
    NoChecks,
    UnknownCi,
}''',
    "WorkKind enum",
)
main = replace_once(
    main,
    '''            Self::ExternalPr => "EXTERNAL_PR",
            Self::WaitCi => "WAIT_CI",
            Self::UnknownCi => "UNKNOWN_CI",''',
    '''            Self::ExternalPr => "EXTERNAL_PR",
            Self::WaitCi => "WAIT_CI",
            Self::NoChecks => "NO_CHECKS",
            Self::UnknownCi => "UNKNOWN_CI",''',
    "WorkKind as_str",
)
main = replace_once(
    main,
    '''            Self::ExternalPr => 249,
            Self::WaitCi => 250,
            Self::UnknownCi => 251,''',
    '''            Self::ExternalPr => 249,
            Self::WaitCi => 250,
            Self::NoChecks => 251,
            Self::UnknownCi => 252,''',
    "WorkKind rank",
)

main = replace_once(
    main,
    '''fn work_kind_for_ci(state: CiState) -> WorkKind {
    match state {
        CiState::Failed => WorkKind::FixCi,
        CiState::Pending => WorkKind::WaitCi,
        CiState::Passing | CiState::NoChecks => WorkKind::PullRequest,
        CiState::Unknown => WorkKind::UnknownCi,
    }
}''',
    '''fn ci_allows_issue_chaining(state: CiState) -> bool {
    state == CiState::Passing
}

fn ci_allows_merge(state: CiState) -> bool {
    state == CiState::Passing
}

fn pull_request_allows_issue_chaining(
    pull_request: &PullRequest,
    trusted_login: &str,
    ci_state: CiState,
) -> bool {
    pull_request.author == trusted_login && ci_allows_issue_chaining(ci_state)
}

fn work_kind_for_ci(state: CiState) -> WorkKind {
    match state {
        CiState::Failed => WorkKind::FixCi,
        CiState::Pending => WorkKind::WaitCi,
        CiState::Passing => WorkKind::PullRequest,
        CiState::NoChecks => WorkKind::NoChecks,
        CiState::Unknown => WorkKind::UnknownCi,
    }
}''',
    "CI doctrine helpers",
)

main = replace_once(
    main,
    '''    let pull_requests = discover_open_pull_requests(organization)?;
    let issues = discover_open_issues(organization)?;
    let mut items = Vec::new();
    let mut repositories_with_open_pr = BTreeSet::new();

    for pull_request in pull_requests
        .iter()
        .filter(|pull_request| eligible.contains(&pull_request.repository))
    {
        repositories_with_open_pr.insert(pull_request.repository.clone());

        if pull_request.author != trusted_login {
            items.push(WorkItem {
                kind: WorkKind::ExternalPr,
                repository: pull_request.repository.clone(),
                number: pull_request.number,
                title: pull_request.title.clone(),
                detail: format!("untrusted author={}", pull_request.author),
                ci_state: None,
                draft: pull_request.draft,
            });
            continue;
        }

        let ci_state = pull_request_ci_state(pull_request)?;
        let kind = work_kind_for_ci(ci_state);
        items.push(WorkItem {
            kind,
            repository: pull_request.repository.clone(),
            number: pull_request.number,
            title: pull_request.title.clone(),
            detail: format!(
                "ci={} {}",
                ci_state.as_str(),
                if pull_request.draft { "draft" } else { "ready" }
            ),
            ci_state: Some(ci_state),
            draft: pull_request.draft,
        });
    }

    for issue in issues.iter().filter(|issue| {
        eligible.contains(&issue.repository)
            && !repositories_with_open_pr.contains(&issue.repository)
    }) {''',
    '''    let pull_requests = discover_open_pull_requests(organization)?;
    let issues = discover_open_issues(organization)?;
    let mut items = Vec::new();
    let mut repositories_with_open_pr = BTreeSet::new();
    let mut repositories_blocking_issue_work = BTreeSet::new();

    for pull_request in pull_requests
        .iter()
        .filter(|pull_request| eligible.contains(&pull_request.repository))
    {
        repositories_with_open_pr.insert(pull_request.repository.clone());

        if pull_request.author != trusted_login {
            repositories_blocking_issue_work.insert(pull_request.repository.clone());
            items.push(WorkItem {
                kind: WorkKind::ExternalPr,
                repository: pull_request.repository.clone(),
                number: pull_request.number,
                title: pull_request.title.clone(),
                detail: format!("untrusted author={}", pull_request.author),
                ci_state: None,
                draft: pull_request.draft,
            });
            continue;
        }

        let ci_state = pull_request_ci_state(pull_request)?;
        if !pull_request_allows_issue_chaining(pull_request, &trusted_login, ci_state) {
            repositories_blocking_issue_work.insert(pull_request.repository.clone());
        }
        let kind = work_kind_for_ci(ci_state);
        items.push(WorkItem {
            kind,
            repository: pull_request.repository.clone(),
            number: pull_request.number,
            title: pull_request.title.clone(),
            detail: format!(
                "ci={} {}",
                ci_state.as_str(),
                if pull_request.draft { "draft" } else { "ready" }
            ),
            ci_state: Some(ci_state),
            draft: pull_request.draft,
        });
    }

    for issue in issues.iter().filter(|issue| {
        eligible.contains(&issue.repository)
            && !repositories_blocking_issue_work.contains(&issue.repository)
    }) {''',
    "triage chaining gate",
)

main = replace_once(
    main,
    '''    let waiting = snapshot
        .items
        .iter()
        .filter(|item| item.kind == WorkKind::WaitCi)
        .count();
    let unknown = snapshot
        .items
        .iter()
        .filter(|item| item.kind == WorkKind::UnknownCi)
        .count();''',
    '''    let waiting = snapshot
        .items
        .iter()
        .filter(|item| item.kind == WorkKind::WaitCi)
        .count();
    let no_checks = snapshot
        .items
        .iter()
        .filter(|item| item.kind == WorkKind::NoChecks)
        .count();
    let unknown = snapshot
        .items
        .iter()
        .filter(|item| item.kind == WorkKind::UnknownCi)
        .count();''',
    "triage no-check count",
)
main = replace_once(
    main,
    '''    println!("Waiting on CI               : {waiting}");
    println!("Unknown CI state            : {unknown}");''',
    '''    println!("Waiting on CI               : {waiting}");
    println!("No checks                   : {no_checks}");
    println!("Unknown CI state            : {unknown}");''',
    "triage no-check print",
)

# Canonical identity: always overwrite local config and hard-bind commit env.
main = replace_once(
    main,
    '''fn ensure_git_identity(workspace: &Path) -> Result<(), String> {
    if capture_in_dir(workspace, "git", &["config", "user.name"])
        .unwrap_or_default()
        .is_empty()
    {
        run_in_dir(
            workspace,
            "git",
            &["config", "user.name", "Memorithm Orchestrator"],
        )?;
    }
    if capture_in_dir(workspace, "git", &["config", "user.email"])
        .unwrap_or_default()
        .is_empty()
    {
        run_in_dir(
            workspace,
            "git",
            &["config", "user.email", "orchestrator@localhost"],
        )?;
    }
    Ok(())
}

fn commit_changes(workspace: &Path, message: &str) -> Result<String, String> {
    ensure_git_identity(workspace)?;
    run_in_dir(workspace, "git", &["add", "-A"])?;
    run_in_dir(workspace, "git", &["commit", "-m", message])?;
    capture_in_dir(workspace, "git", &["rev-parse", "HEAD"])
}''',
    '''fn ensure_git_identity(workspace: &Path) -> Result<(), String> {
    run_in_dir(
        workspace,
        "git",
        &["config", "user.name", AUTONOMOUS_GIT_NAME],
    )?;
    run_in_dir(
        workspace,
        "git",
        &["config", "user.email", AUTONOMOUS_GIT_EMAIL],
    )?;
    Ok(())
}

fn validate_autonomous_commit(workspace: &Path) -> Result<(), String> {
    let author_name = capture_in_dir(workspace, "git", &["show", "-s", "--format=%an", "HEAD"])?;
    let author_email = capture_in_dir(workspace, "git", &["show", "-s", "--format=%ae", "HEAD"])?;
    let committer_name = capture_in_dir(workspace, "git", &["show", "-s", "--format=%cn", "HEAD"])?;
    let committer_email = capture_in_dir(workspace, "git", &["show", "-s", "--format=%ce", "HEAD"])?;
    let message = capture_in_dir(workspace, "git", &["show", "-s", "--format=%B", "HEAD"])?;
    let has_coauthor = message.lines().any(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with("co-authored-by:")
    });
    if author_name != AUTONOMOUS_GIT_NAME
        || author_email != AUTONOMOUS_GIT_EMAIL
        || committer_name != AUTONOMOUS_GIT_NAME
        || committer_email != AUTONOMOUS_GIT_EMAIL
        || has_coauthor
    {
        return Err(format!(
            "autonomous commit identity/message policy violated: author={author_name} <{author_email}> committer={committer_name} <{committer_email}> coauthor={has_coauthor}"
        ));
    }
    Ok(())
}

fn commit_changes(workspace: &Path, message: &str) -> Result<String, String> {
    ensure_git_identity(workspace)?;
    run_in_dir(workspace, "git", &["add", "-A"])?;
    let status = Command::new("git")
        .current_dir(workspace)
        .env("GIT_AUTHOR_NAME", AUTONOMOUS_GIT_NAME)
        .env("GIT_AUTHOR_EMAIL", AUTONOMOUS_GIT_EMAIL)
        .env("GIT_COMMITTER_NAME", AUTONOMOUS_GIT_NAME)
        .env("GIT_COMMITTER_EMAIL", AUTONOMOUS_GIT_EMAIL)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--no-verify",
            "-m",
            message,
        ])
        .status()
        .map_err(|error| format!("failed to execute canonical git commit: {error}"))?;
    if !status.success() {
        return Err(format!("canonical git commit failed with {status}"));
    }
    validate_autonomous_commit(workspace)?;
    capture_in_dir(workspace, "git", &["rev-parse", "HEAD"])
}''',
    "canonical commit identity",
)

# Repository-specific PR discovery and the exact shared gate used by triage/publication.
insert_before = '''fn existing_pr_for_head(repository: &str, branch: &str) -> Result<Option<String>, String> {'''
if main.count(insert_before) != 1:
    raise SystemExit("existing_pr_for_head anchor changed")
repo_gate = '''fn discover_open_pull_requests_for_repository(
    repository: &str,
) -> Result<Vec<PullRequest>, String> {
    let jq = r#".[] | [
        (.number | tostring),
        (if .isDraft then "draft" else "ready" end),
        (.author.login // "-"),
        .title
    ] | @tsv"#;
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
            "100",
            "--json",
            "number,title,isDraft,author",
            "--jq",
            jq,
        ],
    )?;
    let mut pull_requests = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse_pull_request_line(&format!("{repository}\\t{line}")))
        .collect::<Result<Vec<_>, _>>()?;
    pull_requests.sort_by_key(|pull_request| pull_request.number);
    Ok(pull_requests)
}

fn issue_chain_blocker(repository: &str, trusted_login: &str) -> Result<Option<String>, String> {
    for pull_request in discover_open_pull_requests_for_repository(repository)? {
        if pull_request.author != trusted_login {
            return Ok(Some(format!(
                "external/untrusted PR #{} author={} is open",
                pull_request.number, pull_request.author
            )));
        }
        let ci_state = pull_request_ci_state(&pull_request)?;
        if !pull_request_allows_issue_chaining(&pull_request, trusted_login, ci_state) {
            return Ok(Some(format!(
                "trusted PR #{} CI is {}; only PASSING permits another autonomous slice",
                pull_request.number,
                ci_state.as_str()
            )));
        }
    }
    Ok(None)
}

'''
main = main.replace(insert_before, repo_gate + insert_before, 1)

# Remove old any-open-PR helper entirely.
old_open_pr = '''fn open_pr_number(repository: &str) -> Result<Option<u64>, String> {
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

'''
if main.count(old_open_pr) != 1:
    raise SystemExit("open_pr_number helper changed")
main = main.replace(old_open_pr, "", 1)

# Resume publication is allowed to finish already-published work, but a Prepared
# transaction that still needs a push must pass the same live gate.
main = replace_once(
    main,
    '''fn resume_issue_publication(
    config: &RunConfig,
    repository: &Repository,
    item: &WorkItem,
    store: &publication::PublicationStore,
    key: &publication::PublicationKey,
    mut pending: publication::PendingPublication,
) -> Result<(), String> {''',
    '''fn resume_issue_publication(
    config: &RunConfig,
    repository: &Repository,
    item: &WorkItem,
    trusted_login: &str,
    store: &publication::PublicationStore,
    key: &publication::PublicationKey,
    mut pending: publication::PendingPublication,
) -> Result<ActionOutcome, String> {''',
    "resume signature",
)
main = replace_once(
    main,
    '''        store.clear(key)?;
        return Ok(());
    }

    let workspace = ensure_clone(config, &repository.name_with_owner)?;''',
    '''        store.clear(key)?;
        return Ok(ActionOutcome::Progress);
    }

    let workspace = ensure_clone(config, &repository.name_with_owner)?;''',
    "resume existing PR outcome",
)
main = replace_once(
    main,
    '''            run_in_dir(
                &workspace,
                "git",
                &["push", "-u", "origin", pending.branch.as_str()],
            )?;
            pending.phase = publication::PublicationPhase::Pushed;''',
    '''            if let Some(blocker) = issue_chain_blocker(&item.repository, trusted_login)? {
                println!(
                    "Publication remains PREPARED for {}#{}: {blocker}",
                    item.repository, item.number
                );
                return Ok(ActionOutcome::Deferred);
            }
            run_in_dir(
                &workspace,
                "git",
                &["push", "-u", "origin", pending.branch.as_str()],
            )?;
            pending.phase = publication::PublicationPhase::Pushed;''',
    "resume pre-push gate",
)
main = replace_once(
    main,
    '''    create_issue_pull_request(&workspace, item, default_branch, &pending.branch)?;
    store.clear(key)?;
    Ok(())
}''',
    '''    create_issue_pull_request(&workspace, item, default_branch, &pending.branch)?;
    store.clear(key)?;
    Ok(ActionOutcome::Progress)
}''',
    "resume final outcome",
)

# Fresh issue execution: gate before agent, preserve Prepared transaction if the
# gate closes during coding, and re-check immediately before push.
main = replace_once(
    main,
    '''    let store = issue_publication_store(config);
    let key = issue_publication_key(item);

    if let Some(pending) = store
        .load(&key)
        .classified(state::FailureClass::Infrastructure)?
    {
        println!(
            "Resuming pending publication for {}#{} from {} at {}",
            item.repository, item.number, pending.branch, pending.commit
        );
        return resume_issue_publication(config, repository, item, &store, &key, pending)
            .classified(state::FailureClass::Publication)
            .map(|()| ActionOutcome::Progress);
    }

    if let Some(number) =
        open_pr_number(&item.repository).classified(state::FailureClass::Infrastructure)?
    {
        return Err(ActionFailure::new(
            state::FailureClass::Infrastructure,
            format!(
                "repository gained open PR #{number} after triage; deferring issue work to avoid parallel mutation"
            ),
        ));
    }

    let (workspace, branch) = prepare_issue_workspace(config, repository, item.number)''',
    '''    let store = issue_publication_store(config);
    let key = issue_publication_key(item);
    let trusted_login =
        authenticated_github_login().classified(state::FailureClass::Infrastructure)?;

    if let Some(pending) = store
        .load(&key)
        .classified(state::FailureClass::Infrastructure)?
    {
        println!(
            "Resuming pending publication for {}#{} from {} at {}",
            item.repository, item.number, pending.branch, pending.commit
        );
        return resume_issue_publication(
            config,
            repository,
            item,
            &trusted_login,
            &store,
            &key,
            pending,
        )
        .classified(state::FailureClass::Publication);
    }

    if let Some(blocker) = issue_chain_blocker(&item.repository, &trusted_login)
        .classified(state::FailureClass::Infrastructure)?
    {
        println!(
            "Issue {}#{} deferred before agent execution: {blocker}",
            item.repository, item.number
        );
        return Ok(ActionOutcome::Deferred);
    }

    let (workspace, branch) = prepare_issue_workspace(config, repository, item.number)''',
    "execute issue initial gate",
)
main = replace_once(
    main,
    '''    println!("Publication transaction prepared for {branch}");

    run_in_dir(
        &workspace,
        "git",
        &["push", "-u", "origin", branch.as_str()],
    )
    .classified(state::FailureClass::Publication)?;''',
    '''    println!("Publication transaction prepared for {branch}");

    if let Some(blocker) = issue_chain_blocker(&item.repository, &trusted_login)
        .classified(state::FailureClass::Infrastructure)?
    {
        println!(
            "Publication remains PREPARED for {}#{}: {blocker}",
            item.repository, item.number
        );
        return Ok(ActionOutcome::Deferred);
    }

    run_in_dir(
        &workspace,
        "git",
        &["push", "-u", "origin", branch.as_str()],
    )
    .classified(state::FailureClass::Publication)?;''',
    "execute issue immediate pre-push gate",
)

main = replace_once(
    main,
    '''    if !matches!(ci_state, CiState::Passing | CiState::NoChecks) {''',
    '''    if !ci_allows_merge(ci_state) {''',
    "merge PASSING-only gate",
)

main = replace_once(
    main,
    '''        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => {
            Ok(ActionOutcome::Deferred)
        }''',
    '''        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::NoChecks | WorkKind::UnknownCi => {
            Ok(ActionOutcome::Deferred)
        }''',
    "execute nonactionable NoChecks",
)
main = replace_once(
    main,
    '''        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => {
            Ok("non-actionable".to_owned())
        }''',
    '''        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::NoChecks | WorkKind::UnknownCi => {
            Ok("non-actionable".to_owned())
        }''',
    "revision NoChecks",
)
main = replace_once(
    main,
    '''        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::UnknownCi => false,''',
    '''        WorkKind::ExternalPr | WorkKind::WaitCi | WorkKind::NoChecks | WorkKind::UnknownCi => false,''',
    "runnable NoChecks",
)

# Regression tests for doctrine and exact commit identity.
test_anchor = '''    #[test]
    fn required_runtime_contains_local_agent_stack() {'''
if main.count(test_anchor) != 1:
    raise SystemExit("test anchor changed")
tests = '''    #[test]
    fn lifecycle_only_trusted_passing_pr_allows_issue_chaining() {
        let trusted = PullRequest {
            repository: "Memorithm/ADA".to_owned(),
            number: 7,
            title: "trusted".to_owned(),
            draft: false,
            author: "CHECKUPAUTO".to_owned(),
        };
        assert!(pull_request_allows_issue_chaining(
            &trusted,
            "CHECKUPAUTO",
            CiState::Passing
        ));
        for state in [
            CiState::Failed,
            CiState::Pending,
            CiState::NoChecks,
            CiState::Unknown,
        ] {
            assert!(!pull_request_allows_issue_chaining(
                &trusted,
                "CHECKUPAUTO",
                state
            ));
        }

        let external = PullRequest {
            author: "someone-else".to_owned(),
            ..trusted
        };
        assert!(!pull_request_allows_issue_chaining(
            &external,
            "CHECKUPAUTO",
            CiState::Passing
        ));
    }

    #[test]
    fn lifecycle_merge_requires_definitive_passing_ci() {
        assert!(ci_allows_merge(CiState::Passing));
        for state in [
            CiState::Failed,
            CiState::Pending,
            CiState::NoChecks,
            CiState::Unknown,
        ] {
            assert!(!ci_allows_merge(state));
        }
        assert_eq!(work_kind_for_ci(CiState::NoChecks), WorkKind::NoChecks);
    }

    #[test]
    fn autonomous_commit_overrides_existing_identity_without_coauthor() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = env::temp_dir().join(format!(
            "orchestrator-canonical-identity-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&repository).unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .current_dir(&repository)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {} failed", args.join(" "));
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Wrong Identity"]);
        git(&["config", "user.email", "wrong@example.invalid"]);
        fs::write(repository.join("tracked.txt"), "canonical\\n").unwrap();

        commit_changes(&repository, "test: canonical autonomous commit").unwrap();
        let author_name = capture_in_dir(&repository, "git", &["show", "-s", "--format=%an", "HEAD"]).unwrap();
        let author_email = capture_in_dir(&repository, "git", &["show", "-s", "--format=%ae", "HEAD"]).unwrap();
        let committer_name = capture_in_dir(&repository, "git", &["show", "-s", "--format=%cn", "HEAD"]).unwrap();
        let committer_email = capture_in_dir(&repository, "git", &["show", "-s", "--format=%ce", "HEAD"]).unwrap();
        let body = capture_in_dir(&repository, "git", &["show", "-s", "--format=%B", "HEAD"]).unwrap();
        assert_eq!(author_name, AUTONOMOUS_GIT_NAME);
        assert_eq!(author_email, AUTONOMOUS_GIT_EMAIL);
        assert_eq!(committer_name, AUTONOMOUS_GIT_NAME);
        assert_eq!(committer_email, AUTONOMOUS_GIT_EMAIL);
        assert!(!body.to_ascii_lowercase().contains("co-authored-by:"));
        assert_eq!(
            capture_in_dir(&repository, "git", &["config", "user.name"]).unwrap(),
            AUTONOMOUS_GIT_NAME
        );
        assert_eq!(
            capture_in_dir(&repository, "git", &["config", "user.email"]).unwrap(),
            AUTONOMOUS_GIT_EMAIL
        );
        fs::remove_dir_all(repository).unwrap();
    }

'''
main = main.replace(test_anchor, tests + test_anchor, 1)
main_path.write_text(main)

# ---------------------------------------------------------------------------
# Parent Git wrapper: canonical identity survives push-race rebase rewriting.
# ---------------------------------------------------------------------------
git_path = Path("scripts/git")
git_text = git_path.read_text()
git_text = replace_once(
    git_text,
    '''REAL_GIT="${ORCHESTRATOR_REAL_GIT:?ORCHESTRATOR_REAL_GIT is not set}"
''',
    '''REAL_GIT="${ORCHESTRATOR_REAL_GIT:?ORCHESTRATOR_REAL_GIT is not set}"
export GIT_AUTHOR_NAME='ZEKRITI Tarek'
export GIT_AUTHOR_EMAIL='194770978+CHECKUPAUTO@users.noreply.github.com'
export GIT_COMMITTER_NAME='ZEKRITI Tarek'
export GIT_COMMITTER_EMAIL='194770978+CHECKUPAUTO@users.noreply.github.com'
''',
    "parent git identity env",
)
git_path.write_text(git_text)

# ---------------------------------------------------------------------------
# Permanent bridge selftest proves the parent Git environment contract.
# ---------------------------------------------------------------------------
selftest_path = Path("scripts/selftest-runtime.sh")
selftest = selftest_path.read_text()
anchor = '''printf 'PASS agent-git read-only boundary\\n'

# ---------------------------------------------------------------------------
# Agent GitHub CLI: views/checks pass through; mutations and raw API are blocked.
# ---------------------------------------------------------------------------
'''
if selftest.count(anchor) != 1:
    raise SystemExit("runtime selftest anchor changed")
identity_test = '''printf 'PASS agent-git read-only boundary\\n'

# ---------------------------------------------------------------------------
# Parent Git bridge: every autonomous Git subprocess receives canonical identity.
# ---------------------------------------------------------------------------
cat >"$TMP_ROOT/fake-parent-git" <<'EOF'
#!/usr/bin/env bash
[[ "${GIT_AUTHOR_NAME:-}" == 'ZEKRITI Tarek' ]] || exit 61
[[ "${GIT_AUTHOR_EMAIL:-}" == '194770978+CHECKUPAUTO@users.noreply.github.com' ]] || exit 62
[[ "${GIT_COMMITTER_NAME:-}" == 'ZEKRITI Tarek' ]] || exit 63
[[ "${GIT_COMMITTER_EMAIL:-}" == '194770978+CHECKUPAUTO@users.noreply.github.com' ]] || exit 64
printf '%s\\n' "$*" >>"${SELFTEST_PARENT_GIT_TRACE:?}"
exit 0
EOF
chmod 700 "$TMP_ROOT/fake-parent-git"
export SELFTEST_PARENT_GIT_TRACE="$TMP_ROOT/parent-git.trace"
export ORCHESTRATOR_REAL_GIT="$TMP_ROOT/fake-parent-git"
bash "$ROOT/scripts/git" status --short
if ! grep -Fxq 'status --short' "$SELFTEST_PARENT_GIT_TRACE"; then
  fail 'parent git bridge did not enforce canonical identity environment'
fi
printf 'PASS parent-git canonical identity\\n'

# Restore agent-boundary fake Git for any later checks.
export ORCHESTRATOR_REAL_GIT="$TMP_ROOT/fake-git"

# ---------------------------------------------------------------------------
# Agent GitHub CLI: views/checks pass through; mutations and raw API are blocked.
# ---------------------------------------------------------------------------
'''
selftest = selftest.replace(anchor, identity_test, 1)
selftest_path.write_text(selftest)

# ---------------------------------------------------------------------------
# README lifecycle contract.
# ---------------------------------------------------------------------------
readme_path = Path("README.md")
readme = readme_path.read_text()
readme = replace_once(
    readme,
    '''Open PRs whose author does not match the currently authenticated GitHub account are classified `EXTERNAL_PR`: they block new issue work in that repository but are never repaired or merged automatically.
''',
    '''Open PRs whose author does not match the currently authenticated GitHub account are classified `EXTERNAL_PR`: they block new issue work in that repository but are never repaired or merged automatically.

### PR lifecycle doctrine

Autonomous coding is PR-only. Every coding change is either a repair pushed to an already-open trusted PR or a new issue slice published on its own branch and tracked by a PR; autonomous coding never lands directly on a repository default branch. A repository may start another issue slice while trusted PRs remain open only when **every** such PR has definitively `PASSING` CI. `FAILED` CI is repaired first. `PENDING`, `NO_CHECKS`, `UNKNOWN`, and external/untrusted PRs block the next slice. With auto-merge enabled, a trusted passing PR has higher scheduler priority than new issue work and is exact-head revalidated before merge; with auto-merge disabled, a trusted passing PR remains tracked but does not block the next reviewable slice.

The same gate is re-checked immediately before an issue branch is pushed. If the gate closes while the local agent is coding, Orchestrator persists a `Prepared` publication transaction and defers the push until the repository is green again. Autonomous commits always use `ZEKRITI Tarek <194770978+CHECKUPAUTO@users.noreply.github.com>` for both author and committer and never add `Co-authored-by:` trailers.
''',
    "README lifecycle doctrine",
)
readme_path.write_text(readme)
