#!/usr/bin/env python3
"""One-shot mechanical publisher input, never merged into the product branch.

Only edits the three allowlisted product paths from one immutable base. It does
not compile, execute, or import candidate repository code.
"""
import pathlib
import subprocess

BASE = "198a80e59ca3a5d419788bfba57b678218aa1837"
MAIN_BLOB = "71fb1eb6b25a774b9565da526ca36182283d4601"
README_BLOB = "f8da20f26d1c0ff911e5610b62a8f5af1aa0a25e"


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def replace_once(text, before, after):
    if text.count(before) != 1:
        raise SystemExit("Patch anchor is not unique; source reconciliation required")
    return text.replace(before, after, 1)


if git("rev-parse", "HEAD") != BASE:
    raise SystemExit("Unexpected product base")
if git("status", "--porcelain"):
    raise SystemExit("Product workspace is not clean")
if git("rev-parse", "HEAD:src/main.rs") != MAIN_BLOB:
    raise SystemExit("Unexpected main.rs blob")
if git("rev-parse", "HEAD:README.md") != README_BLOB:
    raise SystemExit("Unexpected README blob")

source_path = pathlib.Path("src/main.rs")
source = source_path.read_text(encoding="utf-8")
start = source.index("fn validate_workspace_internal(")
end = source.index("\nfn validate_workspace(", start)
old = source[start:end]
expected_tail = '''    if workspace.join("Cargo.toml").is_file() {
        run_in_dir(workspace, "cargo", &["fmt", "--all", "--", "--check"])?;
        run_in_dir(workspace, "cargo", &["check", "--workspace"])?;
        run_in_dir(
            workspace,
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        )?;
        if config.full_validation {
            run_in_dir(workspace, "cargo", &["test", "--workspace"])?;
        }
    }
    Ok(())
}
'''
replacement_tail = '''    // A missing plan never grants authority to compile candidate code on the
    // host. Check the directory entry, not is_file(): dangling links, special
    // files and metadata errors must not silently become documentation-only.
    match fs::symlink_metadata(workspace.join("Cargo.toml")) {
        Ok(_) => {
            let detail = if config.full_validation {
                "; full validation does not authorize unsandboxed Cargo"
            } else {
                ""
            };
            Err(format!(
                "portable validation plan required for Cargo workspace{detail}"
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot inspect candidate Cargo manifest: {error}")),
    }
}
'''
new = replace_once(old, expected_tail, replacement_tail)
source = source[:start] + new + source[end:]
source = replace_once(
    source,
    "mod tests {\n    use super::*;\n",
    'mod tests {\n    use super::*;\n\n    include!("validation_plan_tests.rs");\n',
)
source_path.write_text(source, encoding="utf-8")

readme_path = pathlib.Path("README.md")
readme = readme_path.read_text(encoding="utf-8")
start = readme.index("## Validation\n")
end = readme.index("## State and isolation\n", start)
section = '''## Validation

Before Orchestrator commits agent changes it always runs `git diff --check`.
When a repository supplies a portable validation plan, every declared command
runs through `scripts/validation-sandbox`; exact-worktree evidence and recovery
reuse remain bound to that plan, policy, base and candidate identity.

A workspace containing a root `Cargo.toml` **requires an explicit portable
validation plan**. Without one, both initial validation and recovery fail closed
before Cargo is invoked. A directory, dangling symlink or other non-regular
manifest entry is also refused; metadata errors do not authorize validation.
There is no host-side fallback to `cargo fmt`, `cargo check`, `cargo clippy` or
`cargo test`. `ORCHESTRATOR_FULL_VALIDATION=1` does not bypass this boundary or
add commands outside the plan: list every required check, lint and test in the
repository's versioned plan.

Repositories without a root Cargo manifest retain the existing diff-only path
when no portable plan is declared. Diff-only success is not functional test
evidence. The agent is separately instructed to run repository-specific tests;
its report never replaces the configured validation or exact-head CI gates.

Audit regression coverage includes no-plan refusals in normal/full/recovery
modes and a real portable Cargo `build.rs` fixture that must not read a host
sentinel, inherit the parent GitHub token, or connect to a host loopback listener.
The latter runs in the existing explicitly enabled root sandbox CI step; a
non-root unit-test pass alone does not qualify operating-system isolation.

'''
readme_path.write_text(readme[:start] + section + readme[end:], encoding="utf-8")

tests = pathlib.Path(__file__).with_name("tests.rs").read_text(encoding="utf-8")
target = pathlib.Path("src/validation_plan_tests.rs")
if target.exists():
    raise SystemExit("Unexpected existing test path")
target.write_text(tests, encoding="utf-8")
subprocess.run(["git", "diff", "--check"], check=True)
print("Prepared ORCH-01 on", BASE)
print("Candidate sources were not executed")
