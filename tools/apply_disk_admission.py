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
    '''    Tool {
        name: "bwrap",
        required: true,
    },
    Tool {
        name: "codex",
''',
    '''    Tool {
        name: "bwrap",
        required: true,
    },
    Tool {
        name: "stat",
        required: true,
    },
    Tool {
        name: "codex",
''',
    "required stat tool",
)
main = replace_once(
    main,
    '''    println!(
        "resource memory  : >= {} MiB available",
        config.resource_policy.min_available_memory_mb
    );
    println!(
        "resource load    : <= {:.2} per CPU",
        config.resource_policy.max_load_per_cpu
    );
''',
    '''    println!(
        "resource memory  : >= {} MiB available",
        config.resource_policy.min_available_memory_mb
    );
    println!(
        "resource disk    : >= {} MiB free in data root",
        config.resource_policy.min_free_disk_mb
    );
    println!(
        "resource load    : <= {:.2} per CPU",
        config.resource_policy.max_load_per_cpu
    );
''',
    "resource policy logging",
)
main = replace_once(
    main,
    '''    let resources = resource::sample_linux().classified(state::FailureClass::Infrastructure)?;
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
''',
    '''    let resources = resource::sample_linux(&config.data_root)
        .classified(state::FailureClass::Infrastructure)?;
    match config.resource_policy.evaluate(resources) {
        resource::Admission::Admitted(snapshot) => {
            println!(
                "resource gate: ADMITTED memory={}MiB disk={}MiB load1={:.2} load/cpu={:.2} cpus={}",
                snapshot.available_memory_mb,
                snapshot.free_disk_mb,
                snapshot.load_one,
                snapshot.load_per_cpu(),
                snapshot.cpu_count
            );
        }
        resource::Admission::Deferred { snapshot, reason } => {
            println!(
                "resource gate: DEFERRED memory={}MiB disk={}MiB load1={:.2} load/cpu={:.2} cpus={} reason={reason}",
                snapshot.available_memory_mb,
                snapshot.free_disk_mb,
                snapshot.load_one,
                snapshot.load_per_cpu(),
                snapshot.cpu_count
            );
            return Ok(ActionOutcome::Deferred);
        }
    }
''',
    "resource sampling and decision logging",
)

start = replace_once(
    start,
    '''export ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB="${ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB:-4096}"
export ORCHESTRATOR_MAX_LOAD_PER_CPU="${ORCHESTRATOR_MAX_LOAD_PER_CPU:-2.0}"
''',
    '''export ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB="${ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB:-4096}"
export ORCHESTRATOR_MIN_FREE_DISK_MB="${ORCHESTRATOR_MIN_FREE_DISK_MB:-8192}"
export ORCHESTRATOR_MAX_LOAD_PER_CPU="${ORCHESTRATOR_MAX_LOAD_PER_CPU:-2.0}"
''',
    "start disk threshold",
)
start = replace_once(
    start,
    '''printf 'min_available_memory_mb=%s\\n' "$ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB"
printf 'max_load_per_cpu=%s\\n' "$ORCHESTRATOR_MAX_LOAD_PER_CPU"
''',
    '''printf 'min_available_memory_mb=%s\\n' "$ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB"
printf 'min_free_disk_mb=%s\\n' "$ORCHESTRATOR_MIN_FREE_DISK_MB"
printf 'max_load_per_cpu=%s\\n' "$ORCHESTRATOR_MAX_LOAD_PER_CPU"
''',
    "start disk logging",
)

install = replace_once(
    install,
    '''for command_name in git gh ollama opencode cargo rustc bwrap systemctl; do
''',
    '''for command_name in git gh ollama opencode cargo rustc bwrap stat systemctl; do
''',
    "installer stat requirement",
)
install = replace_once(
    install,
    '''ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB=4096
ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0
''',
    '''ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB=4096
ORCHESTRATOR_MIN_FREE_DISK_MB=8192
ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0
''',
    "systemd disk threshold",
)

readme = replace_once(
    readme,
    '''ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB=4096
ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0
''',
    '''ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB=4096
ORCHESTRATOR_MIN_FREE_DISK_MB=8192
ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0
''',
    "README disk environment",
)
readme = replace_once(
    readme,
    '''Before executing a selected work item, the Linux runtime samples `MemAvailable` and the one-minute load average. By default it defers work when less than 4096 MiB is available or when load exceeds 2.0 per available CPU. A resource deferral is not a research failure and therefore does not increase the failure/quarantine count. Set either resource threshold to `0` to disable that gate.
''',
    '''Before executing a selected work item, the Linux runtime samples `MemAvailable`, free space on the filesystem containing `ORCHESTRATOR_DATA_ROOT`, and the one-minute load average. By default it defers work when less than 4096 MiB of memory is available, less than 8192 MiB of data-root disk space is free, or load exceeds 2.0 per available CPU. A resource deferral is not a research failure and therefore does not increase the failure/quarantine count. Set any resource threshold to `0` to disable that gate.
''',
    "README disk behavior",
)

main_path.write_text(main)
start_path.write_text(start)
install_path.write_text(install)
readme_path.write_text(readme)
