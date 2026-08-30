#!/usr/bin/env python3
from pathlib import Path

main_path = Path("src/main.rs")
start_path = Path("scripts/start.sh")
install_path = Path("scripts/install-systemd.sh")
readme_path = Path("README.md")

main = main_path.read_text()
start = start_path.read_text()
install = install_path.read_text()
readme = readme_path.read_text()


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return source.replace(old, new, 1)


main = replace_once(
    main,
    "mod publication;\nmod state;\n",
    "mod publication;\nmod resource;\nmod state;\n",
    "resource module",
)
main = replace_once(
    main,
    '''    Tool {
        name: "opencode",
        required: true,
    },
    Tool {
        name: "codex",
''',
    '''    Tool {
        name: "opencode",
        required: true,
    },
    Tool {
        name: "bwrap",
        required: true,
    },
    Tool {
        name: "codex",
''',
    "required bwrap tool",
)
main = replace_once(
    main,
    '''    full_validation: bool,
    max_cycles: u64,
    retry_policy: state::RetryPolicy,
''',
    '''    full_validation: bool,
    max_cycles: u64,
    resource_policy: resource::ResourcePolicy,
    retry_policy: state::RetryPolicy,
''',
    "run config resource policy",
)
main = replace_once(
    main,
    '''fn check_ollama() -> bool {
    Command::new("ollama")
        .arg("list")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn is_local_ollama_model(model: &str) -> bool {
''',
    '''fn check_ollama() -> bool {
    Command::new("ollama")
        .arg("list")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn check_bwrap_sandbox() -> bool {
    Command::new("bwrap")
        .args([
            "--die-with-parent",
            "--unshare-user",
            "--uid",
            "0",
            "--gid",
            "0",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
            "--cap-drop",
            "ALL",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
            "true",
        ])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn is_local_ollama_model(model: &str) -> bool {
''',
    "bubblewrap runtime probe",
)
main = replace_once(
    main,
    '''    if check_ollama() {
        println!("OK       Ollama server reachable");
    } else {
        println!("FAILED   Ollama server unreachable");
        failure = true;
    }

    let model = env::var("ORCHESTRATOR_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
''',
    '''    if check_ollama() {
        println!("OK       Ollama server reachable");
    } else {
        println!("FAILED   Ollama server unreachable");
        failure = true;
    }
    if check_bwrap_sandbox() {
        println!("OK       bubblewrap process sandbox usable");
    } else {
        println!("FAILED   bubblewrap process sandbox unavailable");
        failure = true;
    }

    let model = env::var("ORCHESTRATOR_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
''',
    "doctor sandbox check",
)
main = replace_once(
    main,
    '''            max_cycles: max_cycles_override
                .unwrap_or_else(|| env_u64("ORCHESTRATOR_MAX_CYCLES", 0)),
            retry_policy: state::RetryPolicy {
''',
    '''            max_cycles: max_cycles_override
                .unwrap_or_else(|| env_u64("ORCHESTRATOR_MAX_CYCLES", 0)),
            resource_policy: resource::ResourcePolicy::from_env()?,
            retry_policy: state::RetryPolicy {
''',
    "resource policy from environment",
)
main = replace_once(
    main,
    '''    println!(
        "no-progress wait : {}s",
        config.retry_policy.no_progress_cooldown()
    );

    let attempt_store = state::AttemptStore::new(
''',
    '''    println!(
        "no-progress wait : {}s",
        config.retry_policy.no_progress_cooldown()
    );
    println!(
        "resource memory  : >= {} MiB available",
        config.resource_policy.min_available_memory_mb
    );
    println!(
        "resource load    : <= {:.2} per CPU",
        config.resource_policy.max_load_per_cpu
    );

    let attempt_store = state::AttemptStore::new(
''',
    "run policy resource logging",
)
main = replace_once(
    main,
    '''    println!("kind       : {}", item.kind.as_str());
    println!("repository : {}", item.repository);
    println!("reference  : #{}", item.number);
    println!("title      : {}", item.title);

    match item.kind {
''',
    '''    println!("kind       : {}", item.kind.as_str());
    println!("repository : {}", item.repository);
    println!("reference  : #{}", item.number);
    println!("title      : {}", item.title);

    let resources = resource::sample_linux().classified(state::FailureClass::Infrastructure)?;
    match config.resource_policy.evaluate(resources) {
        resource::Admission::Admitted(snapshot) => {
            println!(
                "resource gate: ADMITTED memory={}MiB load1={:.2} load/cpu={:.2} cpus={}",
                snapshot.available_memory_mb,
                snapshot.load_one,
                snapshot.load_per_cpu(),
                snapshot.cpu_count
            );
        }
        resource::Admission::Deferred { snapshot, reason } => {
            println!(
                "resource gate: DEFERRED memory={}MiB load1={:.2} load/cpu={:.2} cpus={} reason={reason}",
                snapshot.available_memory_mb,
                snapshot.load_one,
                snapshot.load_per_cpu(),
                snapshot.cpu_count
            );
            return Ok(ActionOutcome::Deferred);
        }
    }

    match item.kind {
''',
    "resource admission before execution",
)

start = replace_once(
    start,
    '''export ORCHESTRATOR_FULL_VALIDATION="${ORCHESTRATOR_FULL_VALIDATION:-0}"

ORGANIZATION="${1:-Memorithm}"
''',
    '''export ORCHESTRATOR_FULL_VALIDATION="${ORCHESTRATOR_FULL_VALIDATION:-0}"
export ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB="${ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB:-4096}"
export ORCHESTRATOR_MAX_LOAD_PER_CPU="${ORCHESTRATOR_MAX_LOAD_PER_CPU:-2.0}"

ORGANIZATION="${1:-Memorithm}"
''',
    "start resource defaults",
)
start = replace_once(
    start,
    '''printf 'full_validation=%s\\n' "$ORCHESTRATOR_FULL_VALIDATION"
printf 'opencode_bridge=isolated-ollama-launch+runtime-permissions\\n'
''',
    '''printf 'full_validation=%s\\n' "$ORCHESTRATOR_FULL_VALIDATION"
printf 'min_available_memory_mb=%s\\n' "$ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB"
printf 'max_load_per_cpu=%s\\n' "$ORCHESTRATOR_MAX_LOAD_PER_CPU"
printf 'opencode_bridge=isolated-ollama-launch+runtime-permissions\\n'
''',
    "start resource logging",
)

install = replace_once(
    install,
    '''ORCHESTRATOR_FULL_VALIDATION=1
ORCHESTRATOR_BACKEND_ERROR_MAX=3
''',
    '''ORCHESTRATOR_FULL_VALIDATION=1
ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB=4096
ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0
ORCHESTRATOR_BACKEND_ERROR_MAX=3
''',
    "systemd resource policy",
)

readme = readme.replace("Default model: `ollama/muse-glimmer:latest`.", "Default model: `ollama/qwen3.8:latest`.")
readme = readme.replace("ORCHESTRATOR_MODEL=ollama/muse-glimmer:latest", "ORCHESTRATOR_MODEL=ollama/qwen3.8:latest")
readme = replace_once(
    readme,
    '''orchestrator doctor
orchestrator scan [organization]
''',
    '''orchestrator doctor
orchestrator health
orchestrator status
orchestrator scan [organization]
''',
    "README health commands",
)
readme = replace_once(
    readme,
    '''ORCHESTRATOR_FULL_VALIDATION=0
ORCHESTRATOR_MAX_CYCLES=0
ORCHESTRATOR_DATA_ROOT=~/.local/share/memorithm-orchestrator
''',
    '''ORCHESTRATOR_FULL_VALIDATION=0
ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB=4096
ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0
ORCHESTRATOR_MAX_CYCLES=0
ORCHESTRATOR_DATA_ROOT=~/.local/share/memorithm-orchestrator
''',
    "README resource environment",
)
readme = replace_once(
    readme,
    '''`ORCHESTRATOR_MAX_CYCLES=0` means unlimited cycles.

The autonomous runner rejects any `ORCHESTRATOR_MODEL` that is not an installed `ollama/...` model.
''',
    '''`ORCHESTRATOR_MAX_CYCLES=0` means unlimited cycles.

Before executing a selected work item, the Linux runtime samples `MemAvailable` and the one-minute load average. By default it defers work when less than 4096 MiB is available or when load exceeds 2.0 per available CPU. A resource deferral is not a research failure and therefore does not increase the failure/quarantine count. Set either resource threshold to `0` to disable that gate.

The autonomous runner rejects any `ORCHESTRATOR_MODEL` that is not an installed `ollama/...` model.
''',
    "README resource behavior",
)

main_path.write_text(main)
start_path.write_text(start)
install_path.write_text(install)
readme_path.write_text(readme)
