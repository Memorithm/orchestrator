from pathlib import Path


def replace_once(data: str, old: str, new: str, label: str) -> str:
    count = data.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return data.replace(old, new, 1)


main_path = Path("src/main.rs")
main = main_path.read_text()
main = replace_once(
    main,
    '&["test", "!", "-e", "/root/.ssh"],\n',
    '&["test", "!", "-e", "/root/.ssh/orchestrator-validation-ci-secret"],\n',
    "credential sentinel test",
)
main_path.write_text(main)

ci_path = Path(".github/workflows/ci.yml")
ci = ci_path.read_text()
old = '''          sudo mkdir -p /root/.ssh\n          sudo touch /root/.ssh/orchestrator-validation-ci-secret\n          sudo env \\\n            PATH="$PATH" \\\n            HOME="/root" \\\n            CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \\\n            RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \\\n'''
new = '''          sudo mkdir -p /root/.ssh\n          sudo touch /root/.ssh/orchestrator-validation-ci-secret\n          runner_cargo_home="${CARGO_HOME:-$HOME/.cargo}"\n          runner_rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"\n          sudo chmod a+rx "$runner_cargo_home"\n          if [[ -d "$runner_cargo_home/bin" ]]; then sudo chmod -R a+rX "$runner_cargo_home/bin"; fi\n          if [[ -d "$runner_cargo_home/registry" ]]; then sudo chmod -R a+rX "$runner_cargo_home/registry"; fi\n          if [[ -d "$runner_cargo_home/git" ]]; then sudo chmod -R a+rX "$runner_cargo_home/git"; fi\n          sudo chmod -R a+rX "$runner_rustup_home"\n          sudo env \\\n            PATH="$PATH" \\\n            HOME="/root" \\\n            CARGO_HOME="$runner_cargo_home" \\\n            RUSTUP_HOME="$runner_rustup_home" \\\n'''
ci = replace_once(ci, old, new, "runner toolchain visibility")
ci_path.write_text(ci)
