from pathlib import Path


def replace_once(data: str, old: str, new: str, label: str) -> str:
    count = data.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return data.replace(old, new, 1)


ci_path = Path(".github/workflows/ci.yml")
ci = ci_path.read_text()
old = '''          runner_cargo_home="${CARGO_HOME:-$HOME/.cargo}"\n          runner_rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"\n          sudo chmod a+rx "$runner_cargo_home"\n'''
new = '''          runner_cargo_home="${CARGO_HOME:-$HOME/.cargo}"\n          runner_rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"\n          sudo chmod o+x "$HOME"\n          sudo chmod a+rx "$runner_cargo_home"\n'''
ci = replace_once(ci, old, new, "runner home traversal")
ci_path.write_text(ci)
