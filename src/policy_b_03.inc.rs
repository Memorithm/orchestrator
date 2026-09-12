fn parse_merge_evidence_policy(
    document: &PolicyDocument,
) -> Result<Option<MergeEvidenceRule>, String> {
    let mut in_policy = false;
    let mut saw_section = false;
    let mut schema_version = None;
    let mut required = None;
    let mut requirement_id = None;

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_policy {
            if raw_line == "merge_evidence_policy:" {
                if saw_section {
                    return Err(format!(
                        "duplicate merge_evidence_policy section in origin/{}:{}",
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
                "tab indentation is not allowed in merge evidence policy origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            in_policy = false;
            if raw_line == "merge_evidence_policy:" {
                return Err(format!(
                    "duplicate merge_evidence_policy section in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            continue;
        }
        if indent != 2 {
            return Err(format!(
                "merge_evidence_policy only accepts scalar fields at indentation 2 in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let (key, raw_value) = trimmed.split_once(':').ok_or_else(|| {
            format!(
                "malformed merge evidence policy field in origin/{}:{}",
                document.ref_name, document.path
            )
        })?;
        let value = parse_policy_scalar(raw_value.trim(), key)?;
        match key {
            "schema_version" => {
                let parsed = match value.as_str() {
                    "1" => 1_u8,
                    "2" => 2_u8,
                    other => {
                        return Err(format!(
                            "unsupported merge evidence policy schema_version {other} in origin/{}:{}",
                            document.ref_name, document.path
                        ));
                    }
                };
                if schema_version.replace(parsed).is_some() {
                    return Err(format!(
                        "duplicate merge evidence schema_version in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            "required" => {
                let parsed = MergeEvidenceClass::parse(&value)?;
                if required.replace(parsed).is_some() {
                    return Err(format!(
                        "duplicate merge evidence requirement in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            "requirement_id" => {
                validate_hardware_requirement_id(&value)?;
                if requirement_id.replace(value).is_some() {
                    return Err(format!(
                        "duplicate hardware requirement_id in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            other => {
                return Err(format!(
                    "unknown merge evidence policy field {other} in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
        }
    }

    if !saw_section {
        return Ok(None);
    }
    let schema_version = schema_version.ok_or_else(|| {
        format!(
            "merge_evidence_policy requires schema_version in origin/{}:{}",
            document.ref_name, document.path
        )
    })?;
    let required = required.ok_or_else(|| {
        format!(
            "merge_evidence_policy requires required in origin/{}:{}",
            document.ref_name, document.path
        )
    })?;
    match schema_version {
        1 => {
            if requirement_id.is_some() {
                return Err(format!(
                    "merge_evidence_policy schema v1 does not accept requirement_id in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
        }
        2 => {
            if required != MergeEvidenceClass::HardwareRequired {
                return Err(format!(
                    "merge_evidence_policy schema v2 only supports hardware_required in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            if requirement_id.is_none() {
                return Err(format!(
                    "merge_evidence_policy schema v2 hardware_required requires requirement_id in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
        }
        _ => unreachable!(),
    }
    Ok(Some(MergeEvidenceRule {
        schema_version,
        required,
        requirement_id,
        source_ref: document.ref_name.clone(),
        source_path: document.path.clone(),
        source_commit: document.commit_sha.clone(),
        source_blob: document.blob_sha.clone(),
    }))
}
