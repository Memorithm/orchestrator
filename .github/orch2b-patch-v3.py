from pathlib import Path

main = Path("src/main.rs")
text = main.read_text()
old = '''fn run_portable_validation_plan(
    config: &RunConfig,'''
new = '''#[cfg(test)]
fn run_portable_validation_plan(
    config: &RunConfig,'''
assert text.count(old) == 1, text.count(old)
text = text.replace(old, new, 1)

old_fixture = '''            &ValidationStepResult {
                worktree_head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",'''
new_fixture = '''            &ValidationStepResult {
                plan_attempt_id: "test-attempt",
                worktree_head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",'''
assert text.count(old_fixture) == 2, text.count(old_fixture)
text = text.replace(old_fixture, new_fixture)
main.write_text(text)

state = Path("src/validation_state.rs")
state_text = state.read_text()
old_method = '''
    pub(crate) fn declared_steps(&self) -> usize {
        self.declared_steps
    }
'''
assert state_text.count(old_method) == 1, state_text.count(old_method)
state.write_text(state_text.replace(old_method, "", 1))
