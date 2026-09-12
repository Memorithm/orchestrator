fn parse_merge_policy(document: &PolicyDocument) -> Result<Option<MergePolicyRule>, String> {
    let mut in_policy = false;
    let mut saw_section = false;
    let mut schema_version = None;
    let mut decision = None;

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_policy {
            if raw_line == "autonomous_merge_policy:" {
                if saw_section {
                    return Err(format!(
                        "duplicate autonomous_merge_policy section in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
                in_policy = true;
                saw_section = true;
            }
            continue;
        }
        if raw_line.starts_with('\t') {
            return Err(format!(
                "tab indentation is not allowed in autonomous merge policy origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            in_policy = false;
            if raw_line == "autonomous_merge_policy:" {
                return Err(format!(
                    "duplicate autonomous_merge_policy section in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            continue;
        }
        if indent != 2 {
            return Err(format!(
                "autonomous_merge_policy only accepts scalar fields at indentation 2 in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let (key, raw_value) = trimmed.split_once(':').ok_or_else(|| {
            format!(
                "malformed autonomous merge policy field in origin/{}:{}",
                document.ref_name, document.path
            )
        })?;
        let value = parse_policy_scalar(raw_value.trim(), key)?;
        match key {
            "schema_version" => {
                if schema_version.replace(value.clone()).is_some() {
                    return Err(format!(
                        "duplicate autonomous merge schema_version in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
                if value != "1" {
                    return Err(format!(
                        "unsupported autonomous merge policy schema_version {value} in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            "decision" => {
                let parsed = MergeDecision::parse(&value)?;
                if decision.replace(parsed).is_some() {
                    return Err(format!(
                        "duplicate autonomous merge decision in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            other => {
                return Err(format!(
                    "unknown autonomous merge policy field {other} in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
        }
    }

    if !saw_section {
        return Ok(None);
    }
    if schema_version.as_deref() != Some("1") {
        return Err(format!(
            "autonomous_merge_policy requires schema_version: 1 in origin/{}:{}",
            document.ref_name, document.path
        ));
    }
    let decision = decision.ok_or_else(|| {
        format!(
            "autonomous_merge_policy requires decision in origin/{}:{}",
            document.ref_name, document.path
        )
    })?;
    Ok(Some(MergePolicyRule {
        decision,
        source_ref: document.ref_name.clone(),
        source_path: document.path.clone(),
        source_commit: document.commit_sha.clone(),
        source_blob: document.blob_sha.clone(),
    }))
}
