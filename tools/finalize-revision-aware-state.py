#!/usr/bin/env python3
from pathlib import Path

state_path = Path("src/state.rs")
main_path = Path("src/main.rs")
state = state_path.read_text()
main = main_path.read_text()


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return source.replace(old, new, 1)

state = replace_once(
    state,
    'const LEGACY_REVISION: &str = "legacy";\n',
    '#[cfg(test)]\nconst LEGACY_REVISION: &str = "legacy";\n',
    "legacy revision test scope",
)
for signature in (
    '    pub(crate) fn load(&self, key: &WorkKey) -> Result<AttemptState, String> {\n',
    '    pub(crate) fn begin(&self, key: &WorkKey, now: u64) -> Result<AttemptState, String> {\n',
    '    pub(crate) fn recover_interrupted(\n',
    '    pub(crate) fn record(\n',
):
    state = replace_once(state, signature, '    #[cfg(test)]\n' + signature, f"test-only {signature.strip()}")

state = replace_once(
    state,
    '        assert!(rewritten.starts_with("v2\\n"));\n',
    '        assert!(rewritten.starts_with("v3\\nrevision=legacy\\n"));\n',
    "v1 migration expected version",
)

main = replace_once(
    main,
    '''        store
            .record(
                &work_key(&earlier_alpha),
                state::AttemptOutcome::Success,
                100,
            )
            .unwrap();
''',
    '''        store
            .record_for_revision(
                &work_key(&earlier_alpha),
                "issue-v1",
                state::AttemptOutcome::Success,
                100,
            )
            .unwrap();
''',
    "fairness issue revision",
)

main = replace_once(
    main,
    '''        store
            .record(&work_key(&first), state::AttemptOutcome::Failure, 100)
            .unwrap();
''',
    '''        store
            .record_for_revision(
                &work_key(&first),
                "issue-v1",
                state::AttemptOutcome::Failure,
                100,
            )
            .unwrap();
''',
    "cooldown issue revision",
)

start_marker = '''    #[test]
    fn fair_selection_never_sacrifices_work_kind_priority() {
'''
end_marker = '''    #[test]
    fn state_aware_selection_skips_cooling_priority_item() {
'''
start = main.find(start_marker)
end = main.find(end_marker, start + 1)
if start < 0 or end < 0:
    raise SystemExit("fair priority test boundaries not found")
main = main[:start] + main[end:]

state_path.write_text(state)
main_path.write_text(main)
