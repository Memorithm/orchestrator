from pathlib import Path


def replace_once(data: str, old: str, new: str, label: str) -> str:
    count = data.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return data.replace(old, new, 1)


path = Path("src/main.rs")
data = path.read_text()

marker = '''fn run_portable_validation_plan(
'''
identity = '''struct ValidationWorktreeIdentity<'a> {
    head: &'a str,
    tree: &'a str,
}

'''
data = replace_once(data, marker, identity + marker, "validation worktree identity type")

data = replace_once(
    data,
    '''    let worktree_tree = validation_worktree_tree(config, workspace)?;
    for step in &plan.steps {
        run_portable_validation_step(
            config,
            workspace,
            item,
            snapshot,
            plan,
            step,
            worktree_head,
            &worktree_tree,
        )?;
    }
''',
    '''    let worktree_tree = validation_worktree_tree(config, workspace)?;
    let identity = ValidationWorktreeIdentity {
        head: worktree_head,
        tree: &worktree_tree,
    };
    for step in &plan.steps {
        run_portable_validation_step(
            config,
            workspace,
            item,
            snapshot,
            plan,
            step,
            &identity,
        )?;
    }
''',
    "validation plan identity grouping",
)

data = replace_once(
    data,
    '''    step: &policy::PortableValidationStep,
    worktree_head: &str,
    worktree_tree: &str,
) -> Result<(), String> {
''',
    '''    step: &policy::PortableValidationStep,
    identity: &ValidationWorktreeIdentity<'_>,
) -> Result<(), String> {
''',
    "validation step grouped identity signature",
)

data = replace_once(
    data,
    '''        &ValidationStepResult {
            worktree_head,
            worktree_tree,
            started_at,
''',
    '''        &ValidationStepResult {
            worktree_head: identity.head,
            worktree_tree: identity.tree,
            started_at,
''',
    "validation evidence grouped identity",
)

path.write_text(data)
