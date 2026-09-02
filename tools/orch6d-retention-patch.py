from pathlib import Path

p = Path("src/hardware_capability.rs")
s = p.read_text()

old = "const MAX_AUDIT_BYTES: usize = 96 * 1024;\n"
new = old + "const MAX_AUDIT_RECORDS_PER_BINDING: usize = 256;\nconst MAX_AUDIT_SCAN_ENTRIES: usize = 512;\n"
assert s.count(old) == 1
s = s.replace(old, new, 1)

old = "    ensure_managed_directory(record.request.data_root, &state_root, &directory)?;\n\n    let configured_runners = if record.config.runners.is_empty() {"
new = "    ensure_managed_directory(record.request.data_root, &state_root, &directory)?;\n    prune_audit_records(\n        &directory,\n        MAX_AUDIT_RECORDS_PER_BINDING.saturating_sub(1),\n    )?;\n\n    let configured_runners = if record.config.runners.is_empty() {"
assert s.count(old) == 1
s = s.replace(old, new, 1)

marker = "\nfn ensure_managed_directory(\n"
assert s.count(marker) == 1
helper = r'''
fn prune_audit_records(directory: &Path, keep: usize) -> Result<(), String> {
    let mut records = Vec::new();
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read hardware capability audit directory: {error}"))?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_AUDIT_SCAN_ENTRIES {
            return Err("hardware capability audit directory exceeds bounded scan limit".to_owned());
        }
        let entry = entry
            .map_err(|error| format!("failed to inspect hardware capability audit entry: {error}"))?;
        let file_type = entry.file_type().map_err(|error| {
            format!("failed to inspect hardware capability audit entry type: {error}")
        })?;
        if file_type.is_symlink() || !file_type.is_file() {
            return Err("hardware capability audit directory contains a non-regular entry".to_owned());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "hardware capability audit filename is not UTF-8".to_owned())?;
        let key = parse_audit_filename(&name)?;
        records.push((key, entry.path()));
    }
    records.sort_by_key(|(key, _)| *key);
    let remove_count = records.len().saturating_sub(keep);
    for (_, path) in records.into_iter().take(remove_count) {
        fs::remove_file(&path).map_err(|error| {
            format!(
                "failed to prune retained hardware capability audit record {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn parse_audit_filename(name: &str) -> Result<(u64, u32, u32), String> {
    let stem = name
        .strip_suffix(".state")
        .ok_or_else(|| "invalid hardware capability audit filename extension".to_owned())?;
    let mut parts = stem.split('-');
    let observed_at = parts
        .next()
        .ok_or_else(|| "invalid hardware capability audit filename".to_owned())?
        .parse::<u64>()
        .map_err(|_| "invalid hardware capability audit timestamp filename component".to_owned())?;
    let pid = parts
        .next()
        .ok_or_else(|| "invalid hardware capability audit filename".to_owned())?
        .parse::<u32>()
        .map_err(|_| "invalid hardware capability audit pid filename component".to_owned())?;
    let sequence = parts
        .next()
        .ok_or_else(|| "invalid hardware capability audit filename".to_owned())?
        .parse::<u32>()
        .map_err(|_| "invalid hardware capability audit sequence filename component".to_owned())?;
    if parts.next().is_some() || observed_at == 0 || pid == 0 || sequence >= 128 {
        return Err("invalid hardware capability audit filename".to_owned());
    }
    Ok((observed_at, pid, sequence))
}
'''
s = s.replace(marker, "\n" + helper + marker, 1)

marker = "    #[test]\n    fn runner_parser_rejects_ambiguous_or_malformed_encoded_metadata() {"
assert s.count(marker) == 1
tests = r'''    #[test]
    fn audit_retention_prunes_oldest_records_to_bound() {
        let root = temp_root("audit-retention");
        let directory = root.join("audit");
        fs::create_dir_all(&directory).unwrap();
        for observed_at in 1..=4u64 {
            fs::write(
                directory.join(format!("{observed_at}-1-0.state")),
                "fixture",
            )
            .unwrap();
        }
        prune_audit_records(&directory, 2).unwrap();
        let mut names = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, ["3-1-0.state", "4-1-0.state"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn audit_retention_rejects_symlink_entries() {
        use std::os::unix::fs::symlink;

        let root = temp_root("audit-retention-symlink");
        let directory = root.join("audit");
        fs::create_dir_all(&directory).unwrap();
        fs::write(root.join("outside"), "fixture").unwrap();
        symlink(root.join("outside"), directory.join("1-1-0.state")).unwrap();
        let error = prune_audit_records(&directory, 1).unwrap_err();
        assert!(error.contains("non-regular"));
        fs::remove_dir_all(root).unwrap();
    }

'''
s = s.replace(marker, tests + marker, 1)
p.write_text(s)
