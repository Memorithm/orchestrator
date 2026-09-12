#[cfg(test)]
pub(crate) fn test_snapshot_for_validation(
    repository: &str,
    base_branch: &str,
    base_sha: &str,
) -> PolicySnapshot {
    PolicySnapshot {
        repository: repository.to_owned(),
        base_branch: base_branch.to_owned(),
        base_sha: base_sha.to_owned(),
        bootstrap: None,
        documents: Vec::new(),
    }
}
