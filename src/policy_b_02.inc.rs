fn parse_validation_plan(
    document: &PolicyDocument,
) -> Result<Option<PortableValidationPlan>, String> {
    #[derive(Default)]
    struct PendingStep {
        id: Option<String>,
        argv: Option<Vec<String>>,
        cwd: Option<String>,
        timeout_seconds: Option<u64>,
    }

    fn finish_step(
        pending: &mut Option<PendingStep>,
        seen_ids: &mut BTreeSet<String>,
        steps: &mut Vec<PortableValidationStep>,
        document: &PolicyDocument,
    ) -> Result<(), String> {
        let Some(step) = pending.take() else {
            return Ok(());
        };
        if steps.len() >= MAX_VALIDATION_STEPS {
            return Err(format!(
                "validation plan exceeds {MAX_VALIDATION_STEPS} steps in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let id = step
            .id
            .ok_or_else(|| "validation step is missing id".to_owned())?;
        let argv = step
            .argv
            .ok_or_else(|| format!("validation step {id} is missing argv"))?;
        let cwd = step.cwd.unwrap_or_else(|| ".".to_owned());
        let timeout_seconds = step.timeout_seconds.unwrap_or(300);
        if !seen_ids.insert(id.clone()) {
            return Err(format!("duplicate validation step id: {id}"));
        }
        steps.push(PortableValidationStep {
            id,
            argv,
            cwd,
            timeout_seconds,
        });
        Ok(())
    }

    let mut in_plan = false;
    let mut in_steps = false;
    let mut saw_section = false;
    let mut schema_version = None;
    let mut class = None;
    let mut pending_step: Option<PendingStep> = None;
    let mut seen_ids = BTreeSet::new();
    let mut steps = Vec::new();

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_plan {
            if raw_line == "validation_plan:" {
                if saw_section {
                    return Err(format!(
                        "duplicate validation_plan section in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
                in_plan = true;
                saw_section = true;
            }
            continue;
        }
        if raw_line.starts_with('\t') {
            return Err(format!(
                "tab indentation is not allowed in validation_plan origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            finish_step(&mut pending_step, &mut seen_ids, &mut steps, document)?;
            in_plan = false;
            in_steps = false;
            if raw_line == "validation_plan:" {
                return Err(format!(
                    "duplicate validation_plan section in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            continue;
        }
        if indent == 2 {
            finish_step(&mut pending_step, &mut seen_ids, &mut steps, document)?;
            in_steps = false;
            let (key, raw_value) = trimmed.split_once(':').ok_or_else(|| {
                format!(
                    "malformed validation_plan field in origin/{}:{}",
                    document.ref_name, document.path
                )
            })?;
            match key {
                "schema_version" => {
                    let value = parse_policy_scalar(raw_value.trim(), key)?;
                    if schema_version.replace(value.clone()).is_some() {
                        return Err("duplicate validation_plan schema_version".to_owned());
                    }
                    if value != "1" {
                        return Err(format!(
                            "unsupported validation_plan schema_version {value}"
                        ));
                    }
                }
                "class" => {
                    let value = parse_policy_scalar(raw_value.trim(), key)?;
                    if class.replace(value.clone()).is_some() {
                        return Err("duplicate validation_plan class".to_owned());
                    }
                    if value != "portable" {
                        return Err(format!("unsupported validation_plan class: {value}"));
                    }
                }
                "steps" if raw_value.trim().is_empty() => in_steps = true,
                other => {
                    return Err(format!("unknown validation_plan field: {other}"));
                }
            }
            continue;
        }
        if indent == 4 && trimmed.starts_with("- ") {
            if !in_steps {
                return Err("validation step declared outside steps".to_owned());
            }
            finish_step(&mut pending_step, &mut seen_ids, &mut steps, document)?;
            let rest = trimmed.trim_start_matches("- ");
            let (key, raw_value) = rest
                .split_once(':')
                .ok_or_else(|| "malformed validation step declaration".to_owned())?;
            if key != "id" {
                return Err("validation step must begin with id".to_owned());
            }
            let id = parse_policy_scalar(raw_value.trim(), "validation step id")?;
            if id.is_empty()
                || id.len() > 64
                || !id.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
                })
            {
                return Err(format!("invalid validation step id: {id:?}"));
            }
            pending_step = Some(PendingStep {
                id: Some(id),
                ..PendingStep::default()
            });
            continue;
        }
        if indent == 6 && in_steps {
            let step = pending_step
                .as_mut()
                .ok_or_else(|| "validation step field appears before id".to_owned())?;
            let (key, raw_value) = trimmed
                .split_once(':')
                .ok_or_else(|| "malformed validation step field".to_owned())?;
            match key {
                "argv" => {
                    if step.argv.is_some() {
                        return Err("duplicate validation step argv".to_owned());
                    }
                    step.argv = Some(parse_validation_argv(raw_value.trim())?);
                }
                "cwd" => {
                    if step.cwd.is_some() {
                        return Err("duplicate validation step cwd".to_owned());
                    }
                    let value = parse_policy_scalar(raw_value.trim(), key)?;
                    step.cwd = Some(validate_validation_cwd(&value)?);
                }
                "timeout_seconds" => {
                    if step.timeout_seconds.is_some() {
                        return Err("duplicate validation step timeout_seconds".to_owned());
                    }
                    let value = parse_policy_scalar(raw_value.trim(), key)?;
                    let timeout = value
                        .parse::<u64>()
                        .map_err(|_| format!("invalid validation timeout_seconds: {value}"))?;
                    if timeout == 0 || timeout > MAX_VALIDATION_TIMEOUT_SECS {
                        return Err(format!(
                            "validation timeout_seconds must be within 1..={MAX_VALIDATION_TIMEOUT_SECS}"
                        ));
                    }
                    step.timeout_seconds = Some(timeout);
                }
                other => return Err(format!("unknown validation step field: {other}")),
            }
            continue;
        }
        return Err(format!(
            "unsupported validation_plan indentation or structure in origin/{}:{}",
            document.ref_name, document.path
        ));
    }

    if !saw_section {
        return Ok(None);
    }
    finish_step(&mut pending_step, &mut seen_ids, &mut steps, document)?;
    if schema_version.as_deref() != Some("1") {
        return Err("validation_plan requires schema_version: 1".to_owned());
    }
    if class.as_deref() != Some("portable") {
        return Err("validation_plan requires class: portable".to_owned());
    }
    if steps.is_empty() {
        return Err("validation_plan requires at least one step".to_owned());
    }
    Ok(Some(PortableValidationPlan {
        steps,
        source_ref: document.ref_name.clone(),
        source_path: document.path.clone(),
        source_commit: document.commit_sha.clone(),
        source_blob: document.blob_sha.clone(),
    }))
}

fn validate_hardware_requirement_id(value: &str) -> Result<(), String> {
    const MAX_CHARS: usize = 96;
    if value.is_empty() || value.len() > MAX_CHARS {
        return Err("invalid hardware requirement_id".to_owned());
    }
    let mut bytes = value.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid hardware requirement_id".to_owned());
    }
    Ok(())
}
