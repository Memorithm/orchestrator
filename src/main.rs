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

fn usage(program: &str) {
    eprintln!("Usage:");
    eprintln!("  {program} doctor");
}

fn main() -> ExitCode {
    let mut args = env::args();
    let program = args.next().unwrap_or_else(|| "orchestrator".to_owned());

    match args.next().as_deref() {
        Some("doctor") => doctor(),
        _ => {
            usage(&program);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TOOLS;

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
}
