use std::env;
use std::process::{Command, ExitCode};

#[derive(Debug, Clone, Copy)]
struct Tool {
    name: &'static str,
    required: bool,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "git",
        required: true,
    },
    Tool {
        name: "gh",
        required: true,
    },
    Tool {
        name: "ollama",
        required: true,
    },
    Tool {
        name: "opencode",
        required: true,
    },
    Tool {
        name: "codex",
        required: false,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Repository {
    name_with_owner: String,
    default_branch: Option<String>,
    visibility: String,
    archived: bool,
    fork: bool,
    empty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pilotability {
    Eligible,
    BlockedArchived,
    BlockedEmpty,
    ReviewFork,
    ReviewSpecialBranch,
    SelfRepository,
}

impl Pilotability {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Eligible => "ELIGIBLE",
            Self::BlockedArchived => "BLOCKED_ARCHIVED",
            Self::BlockedEmpty => "BLOCKED_EMPTY",
            Self::ReviewFork => "REVIEW_FORK",
            Self::ReviewSpecialBranch => "REVIEW_SPECIAL_BRANCH",
            Self::SelfRepository => "SELF",
        }
    }
}

fn command_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn print_version(name: &str) {
    match Command::new(name).arg("--version").output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let line = stdout
                .lines()
                .chain(stderr.lines())
                .find(|line| !line.trim().is_empty())
                .unwrap_or("installed");

            println!("{name:<10} {line}");
        }
        Err(error) => println!("{name:<10} ERROR: {error}"),
    }
}

fn check_github_auth() -> bool {
    Command::new("gh")
        .args(["auth", "status"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn check_ollama() -> bool {
    Command::new("ollama")
        .arg("list")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn doctor() -> ExitCode {
    println!("Memorithm Orchestrator doctor");
    println!("==============================");
    println!();

    let mut failure = false;

    for tool in TOOLS {
        let available = command_available(tool.name);

        if available {
            print!("OK       ");
            print_version(tool.name);
        } else if tool.required {
            println!("MISSING  {:<10} required", tool.name);
            failure = true;
        } else {
            println!("OPTIONAL {:<10} not installed", tool.name);
        }
    }

    println!();
    println!("Runtime checks");
    println!("--------------");

    if check_github_auth() {
        println!("OK       GitHub CLI authenticated");
    } else {
        println!("FAILED   GitHub CLI authentication");
        failure = true;
    }

    if check_ollama() {
        println!("OK       Ollama server reachable");
    } else {
        println!("FAILED   Ollama server unreachable");
        failure = true;
    }

    println!();
    println!("Cost policy");
    println!("-----------");
    println!("OpenAI API : disabled");
    println!("Default AI : OpenCode + local Ollama");
    println!("Codex      : optional fallback using existing ChatGPT entitlement");
    println!("Paid API   : none");

    if failure {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn parse_repository_line(line: &str) -> Result<Repository, String> {
    let fields: Vec<&str> = line.split('\t').collect();

    if fields.len() != 6 {
        return Err(format!(
            "expected 6 tab-separated fields, got {}: {line}",
            fields.len()
        ));
    }

    let default_branch = match fields[1] {
        "-" | "" => None,
        branch => Some(branch.to_owned()),
    };

    let archived = match fields[3] {
        "active" => false,
        "archived" => true,
        other => return Err(format!("unknown repository state: {other}")),
    };

    let fork = match fields[4] {
        "source" => false,
        "fork" => true,
        other => return Err(format!("unknown repository origin: {other}")),
    };

    let empty = match fields[5] {
        "non-empty" => false,
        "empty" => true,
        other => return Err(format!("unknown repository emptiness: {other}")),
    };

    Ok(Repository {
        name_with_owner: fields[0].to_owned(),
        default_branch,
        visibility: fields[2].to_owned(),
        archived,
        fork,
        empty,
    })
}

fn discover_repositories(organization: &str) -> Result<Vec<Repository>, String> {
    let jq = r#".[] | [
        .nameWithOwner,
        (.defaultBranchRef.name // "-"),
        .visibility,
        (if .isArchived then "archived" else "active" end),
        (if .isFork then "fork" else "source" end),
        (if .isEmpty then "empty" else "non-empty" end)
    ] | @tsv"#;

    let output = Command::new("gh")
        .args([
            "repo",
            "list",
            organization,
            "--limit",
            "1000",
            "--json",
            "nameWithOwner,defaultBranchRef,visibility,isArchived,isFork,isEmpty",
            "--jq",
            jq,
        ])
        .output()
        .map_err(|error| format!("failed to execute gh: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh repo list failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("invalid UTF-8 from gh: {error}"))?;

    let mut repositories = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_repository_line)
        .collect::<Result<Vec<_>, _>>()?;

    repositories.sort_by(|left, right| left.name_with_owner.cmp(&right.name_with_owner));

    Ok(repositories)
}

fn classify_repository(repository: &Repository, organization: &str) -> Pilotability {
    let orchestrator_name = format!("{organization}/orchestrator");

    if repository.name_with_owner == orchestrator_name {
        return Pilotability::SelfRepository;
    }

    if repository.archived {
        return Pilotability::BlockedArchived;
    }

    if repository.empty {
        return Pilotability::BlockedEmpty;
    }

    if repository.fork {
        return Pilotability::ReviewFork;
    }

    match repository.default_branch.as_deref() {
        Some("main" | "master") => Pilotability::Eligible,
        Some(_) | None => Pilotability::ReviewSpecialBranch,
    }
}

fn scan(organization: &str) -> ExitCode {
    let repositories = match discover_repositories(organization) {
        Ok(repositories) => repositories,
        Err(error) => {
            eprintln!("scan failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    println!("Repository scan: {organization}");
    println!("==============================");
    println!();
    println!(
        "{:<48} {:<32} {:<10} {:<9} {:<8} PILOTABILITY",
        "REPOSITORY", "DEFAULT BRANCH", "VISIBILITY", "STATE", "TYPE"
    );
    println!("{}", "-".repeat(145));

    let mut active = 0usize;
    let mut archived = 0usize;
    let mut public = 0usize;
    let mut private = 0usize;
    let mut forks = 0usize;
    let mut without_default_branch = 0usize;
    let mut eligible = 0usize;
    let mut blocked_archived = 0usize;
    let mut blocked_empty = 0usize;
    let mut review_fork = 0usize;
    let mut review_special_branch = 0usize;
    let mut self_repository = 0usize;

    for repository in &repositories {
        if repository.archived {
            archived += 1;
        } else {
            active += 1;
        }

        match repository.visibility.as_str() {
            "PUBLIC" => public += 1,
            "PRIVATE" => private += 1,
            _ => {}
        }

        if repository.fork {
            forks += 1;
        }

        if repository.default_branch.is_none() {
            without_default_branch += 1;
        }

        let pilotability = classify_repository(repository, organization);

        match pilotability {
            Pilotability::Eligible => eligible += 1,
            Pilotability::BlockedArchived => blocked_archived += 1,
            Pilotability::BlockedEmpty => blocked_empty += 1,
            Pilotability::ReviewFork => review_fork += 1,
            Pilotability::ReviewSpecialBranch => review_special_branch += 1,
            Pilotability::SelfRepository => self_repository += 1,
        }

        println!(
            "{:<48} {:<32} {:<10} {:<9} {:<8} {}",
            repository.name_with_owner,
            repository.default_branch.as_deref().unwrap_or("-"),
            repository.visibility,
            if repository.archived {
                "archived"
            } else {
                "active"
            },
            if repository.fork { "fork" } else { "source" },
            pilotability.as_str()
        );
    }

    println!();
    println!("Summary");
    println!("-------");
    println!("Total                  : {}", repositories.len());
    println!("Active                 : {active}");
    println!("Archived               : {archived}");
    println!("Public                 : {public}");
    println!("Private                : {private}");
    println!("Forks                  : {forks}");
    println!("No default branch      : {without_default_branch}");
    println!();
    println!("Pilotability");
    println!("------------");
    println!("Eligible               : {eligible}");
    println!("Blocked archived       : {blocked_archived}");
    println!("Blocked empty          : {blocked_empty}");
    println!("Review fork            : {review_fork}");
    println!("Review special branch  : {review_special_branch}");
    println!("Self                   : {self_repository}");

    ExitCode::SUCCESS
}

fn usage(program: &str) {
    eprintln!("Usage:");
    eprintln!("  {program} doctor");
    eprintln!("  {program} scan [organization]");
}

fn main() -> ExitCode {
    let mut args = env::args();
    let program = args.next().unwrap_or_else(|| "orchestrator".to_owned());

    match args.next().as_deref() {
        Some("doctor") => doctor(),
        Some("scan") => {
            let organization = args.next().unwrap_or_else(|| "Memorithm".to_owned());
            scan(&organization)
        }
        _ => {
            usage(&program);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Pilotability, Repository, TOOLS, classify_repository, parse_repository_line};

    #[test]
    fn required_runtime_contains_local_agent_stack() {
        let required: Vec<_> = TOOLS
            .iter()
            .filter(|tool| tool.required)
            .map(|tool| tool.name)
            .collect();

        assert!(required.contains(&"git"));
        assert!(required.contains(&"gh"));
        assert!(required.contains(&"ollama"));
        assert!(required.contains(&"opencode"));
    }

    #[test]
    fn codex_is_optional() {
        let codex = TOOLS
            .iter()
            .find(|tool| tool.name == "codex")
            .expect("codex specification");

        assert!(!codex.required);
    }

    #[test]
    fn parses_repository_with_default_branch() {
        let repository =
            parse_repository_line("Memorithm/ADA\tmain\tPUBLIC\tactive\tsource\tnon-empty")
                .unwrap();

        assert_eq!(
            repository,
            Repository {
                name_with_owner: "Memorithm/ADA".to_owned(),
                default_branch: Some("main".to_owned()),
                visibility: "PUBLIC".to_owned(),
                archived: false,
                fork: false,
                empty: false,
            }
        );
    }

    #[test]
    fn parses_repository_without_default_branch() {
        let repository =
            parse_repository_line("Memorithm/scirust-automotive\t-\tPUBLIC\tactive\tsource\tempty")
                .unwrap();

        assert_eq!(repository.default_branch, None);
        assert!(repository.empty);
    }

    #[test]
    fn parses_archived_private_repository() {
        let repository =
            parse_repository_line("Memorithm/CCOS\tmain\tPRIVATE\tarchived\tsource\tnon-empty")
                .unwrap();

        assert!(repository.archived);
        assert_eq!(repository.visibility, "PRIVATE");
        assert!(!repository.empty);
    }

    #[test]
    fn rejects_malformed_repository_line() {
        let error = parse_repository_line("Memorithm/ADA\tmain").unwrap_err();

        assert!(error.contains("expected 6"));
    }

    #[test]
    fn classifies_standard_main_repository_as_eligible() {
        let repository =
            parse_repository_line("Memorithm/ADA\tmain\tPUBLIC\tactive\tsource\tnon-empty")
                .unwrap();

        assert_eq!(
            classify_repository(&repository, "Memorithm"),
            Pilotability::Eligible
        );
    }

    #[test]
    fn classifies_standard_master_repository_as_eligible() {
        let repository =
            parse_repository_line("Memorithm/scirust\tmaster\tPUBLIC\tactive\tsource\tnon-empty")
                .unwrap();

        assert_eq!(
            classify_repository(&repository, "Memorithm"),
            Pilotability::Eligible
        );
    }

    #[test]
    fn blocks_archived_repository() {
        let repository =
            parse_repository_line("Memorithm/CCOS\tmain\tPRIVATE\tarchived\tsource\tnon-empty")
                .unwrap();

        assert_eq!(
            classify_repository(&repository, "Memorithm"),
            Pilotability::BlockedArchived
        );
    }

    #[test]
    fn blocks_empty_repository() {
        let repository =
            parse_repository_line("Memorithm/scirust-automotive\t-\tPUBLIC\tactive\tsource\tempty")
                .unwrap();

        assert_eq!(
            classify_repository(&repository, "Memorithm"),
            Pilotability::BlockedEmpty
        );
    }

    #[test]
    fn reviews_nonstandard_default_branch() {
        let repository = parse_repository_line(
            "Memorithm/ExtremEngine\tagent/initial-engine\tPUBLIC\tactive\tsource\tnon-empty",
        )
        .unwrap();

        assert_eq!(
            classify_repository(&repository, "Memorithm"),
            Pilotability::ReviewSpecialBranch
        );
    }

    #[test]
    fn never_auto_selects_orchestrator_itself() {
        let repository = parse_repository_line(
            "Memorithm/orchestrator\tmain\tPRIVATE\tactive\tsource\tnon-empty",
        )
        .unwrap();

        assert_eq!(
            classify_repository(&repository, "Memorithm"),
            Pilotability::SelfRepository
        );
    }

    #[test]
    fn reviews_forks_before_autonomous_work() {
        let repository =
            parse_repository_line("Memorithm/example\tmain\tPUBLIC\tactive\tfork\tnon-empty")
                .unwrap();

        assert_eq!(
            classify_repository(&repository, "Memorithm"),
            Pilotability::ReviewFork
        );
    }
}
