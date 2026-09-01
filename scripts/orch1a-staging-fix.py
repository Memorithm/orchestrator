from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    data = file.read_text()
    count = data.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}: {old[:120]!r}")
    file.write_text(data.replace(old, new, 1))


replace_once(
    "src/policy.rs",
    '''            let Some((field, value)) = deny_basis(&rule) else {
                continue;
            };
            return Ok(TaskEligibility::Deferred(PolicyDenial {
                item_id: rule.id,
                field,
                value: value.to_owned(),
''',
    '''            let Some((field, value)) = deny_basis(&rule)
                .map(|(field, value)| (field, value.to_owned()))
            else {
                continue;
            };
            return Ok(TaskEligibility::Deferred(PolicyDenial {
                item_id: rule.id,
                field,
                value,
''',
)

replace_once(
    "src/main.rs",
    '''        return resume_issue_publication(
            config,
            repository,
            item,
            &trusted_login,
            &store,
            &key,
            pending,
        )
        .classified(state::FailureClass::Publication);
''',
    '''        return resume_issue_publication(
            config,
            repository,
            item,
            &trusted_login,
            &store,
            &key,
            pending,
        )
        .classified(state::FailureClass::Publication)
        .map(ActionExecution::completed);
''',
)
