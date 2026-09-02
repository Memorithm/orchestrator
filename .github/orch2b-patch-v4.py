from pathlib import Path

path = Path("src/main.rs")
text = path.read_text()

anchor = '''#[derive(Debug)]
enum PortableStepOutcome {
    Passed,
    Failed(String),
    TimedOut(String),
}
'''
replacement = '''#[derive(Debug)]
enum PortableStepOutcome {
    Passed,
    Failed(String),
    TimedOut(String),
}

struct PortableValidationStepContext<'a> {
    identity: &'a ValidationWorktreeIdentity<'a>,
    plan_attempt_id: &'a str,
}
'''
assert text.count(anchor) == 1, text.count(anchor)
text = text.replace(anchor, replacement, 1)

old_call = '''        let outcome = run_portable_validation_step(
            config,
            workspace,
            item,
            snapshot,
            plan,
            step,
            &identity,
            attempt.attempt_id(),
        )?;'''
new_call = '''        let step_context = PortableValidationStepContext {
            identity: &identity,
            plan_attempt_id: attempt.attempt_id(),
        };
        let outcome = run_portable_validation_step(
            config,
            workspace,
            item,
            snapshot,
            plan,
            step,
            &step_context,
        )?;'''
assert text.count(old_call) == 1, text.count(old_call)
text = text.replace(old_call, new_call, 1)

old_sig = '''fn run_portable_validation_step(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    step: &policy::PortableValidationStep,
    identity: &ValidationWorktreeIdentity<'_>,
    plan_attempt_id: &str,
) -> Result<PortableStepOutcome, String> {'''
new_sig = '''fn run_portable_validation_step(
    config: &RunConfig,
    workspace: &Path,
    item: &WorkItem,
    snapshot: &policy::PolicySnapshot,
    plan: &policy::PortableValidationPlan,
    step: &policy::PortableValidationStep,
    context: &PortableValidationStepContext<'_>,
) -> Result<PortableStepOutcome, String> {'''
assert text.count(old_sig) == 1, text.count(old_sig)
text = text.replace(old_sig, new_sig, 1)

text = text.replace('''            plan_attempt_id,
            worktree_head: identity.head,
            worktree_tree: identity.tree,''', '''            plan_attempt_id: context.plan_attempt_id,
            worktree_head: context.identity.head,
            worktree_tree: context.identity.tree,''', 1)

path.write_text(text)
