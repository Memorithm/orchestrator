fn parse_roadmap_task_rules(document: &PolicyDocument) -> Result<Vec<RoadmapTaskRule>, String> {
    let mut in_roadmap = false;
    let mut current: Option<RoadmapTaskRule> = None;
    let mut rules = Vec::new();
    let mut ids = BTreeSet::new();

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_roadmap {
            if raw_line == "roadmap:" {
                in_roadmap = true;
            }
            continue;
        }

        if raw_line.starts_with('\t') {
            return Err(format!(
                "tab indentation is not allowed in recognized roadmap policy origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            break;
        }

        if indent == 2 && trimmed.starts_with("- id:") {
            if let Some(rule) = current.take() {
                finish_roadmap_rule(rule, &mut ids, &mut rules)?;
            }
            let id = parse_policy_scalar(trimmed[5..].trim(), "id")?;
            canonical_policy_id(&id)?;
            current = Some(RoadmapTaskRule {
                id,
                status: None,
                agent_policy: None,
                execution_policy: None,
                source_ref: document.ref_name.clone(),
                source_path: document.path.clone(),
                source_commit: document.commit_sha.clone(),
                source_blob: document.blob_sha.clone(),
            });
            continue;
        }
        if indent == 2 && trimmed.starts_with("- ") {
            return Err(format!(
                "recognized roadmap contains a top-level list item without an id in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        if indent != 4 {
            continue;
        }

        let Some((key, raw_value)) = trimmed.split_once(':') else {
            continue;
        };
        if !matches!(key, "status" | "agent_policy" | "execution_policy") {
            continue;
        }
        let rule = current.as_mut().ok_or_else(|| {
            format!(
                "roadmap field {key} appears before an id in origin/{}:{}",
                document.ref_name, document.path
            )
        })?;
        let value = parse_policy_scalar(raw_value.trim(), key)?;
        let slot = match key {
            "status" => &mut rule.status,
            "agent_policy" => &mut rule.agent_policy,
            "execution_policy" => &mut rule.execution_policy,
            _ => unreachable!(),
        };
        if slot.replace(value).is_some() {
            return Err(format!(
                "duplicate roadmap field {key} for id {} in origin/{}:{}",
                rule.id, document.ref_name, document.path
            ));
        }
    }

    if let Some(rule) = current {
        finish_roadmap_rule(rule, &mut ids, &mut rules)?;
    }
    Ok(rules)
}

fn finish_roadmap_rule(
    rule: RoadmapTaskRule,
    ids: &mut BTreeSet<String>,
    rules: &mut Vec<RoadmapTaskRule>,
) -> Result<(), String> {
    let canonical = canonical_policy_id(&rule.id)?;
    if !ids.insert(canonical) {
        return Err(format!(
            "duplicate roadmap id {} in one mandatory policy document",
            rule.id
        ));
    }
    rules.push(rule);
    Ok(())
}

fn parse_policy_scalar(value: &str, field: &str) -> Result<String, String> {
    if value.is_empty() || matches!(value, ">" | "|") || value.len() > 256 {
        return Err(format!("invalid scalar value for roadmap field {field}"));
    }
    let value = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(format!("invalid scalar value for roadmap field {field}"));
    }
    Ok(value.to_owned())
}

fn canonical_policy_id(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 128 {
        return Err(format!("invalid roadmap id: {value:?}"));
    }
    let mut canonical = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            canonical.push(character.to_ascii_uppercase());
        } else if !matches!(character, '_' | '-' | '.') {
            return Err(format!("invalid roadmap id: {value:?}"));
        }
    }
    if canonical.is_empty() {
        return Err(format!("invalid roadmap id: {value:?}"));
    }
    Ok(canonical)
}

fn task_mentions_policy_id(text: &str, canonical_id: &str) -> bool {
    text.split(|character: char| {
        !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    })
    .filter(|token| !token.is_empty())
    .any(|token| canonical_policy_id(token).is_ok_and(|candidate| candidate == canonical_id))
}

fn body_targets_policy_id(body: &str, canonical_id: &str) -> bool {
    body.lines().any(|line| {
        body_target_value(line).is_some_and(|value| task_mentions_policy_id(value, canonical_id))
    })
}

fn body_target_value(line: &str) -> Option<&str> {
    let line = line.trim().trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '-' | '*' | '>' | '#')
    });
    let (label, value) = line.split_once(':')?;
    let label = label.trim().trim_matches('*').to_ascii_lowercase();
    if !matches!(
        label.as_str(),
        "target"
            | "current target"
            | "roadmap"
            | "roadmap item"
            | "milestone"
            | "stage"
            | "work item"
    ) {
        return None;
    }
    let value = value.trim().trim_start_matches('*').trim();
    (!value.is_empty()).then_some(value)
}

fn deny_basis(rule: &RoadmapTaskRule) -> Option<(&'static str, &str)> {
    if rule
        .agent_policy
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("forbidden_to_initiate"))
    {
        return Some((
            "agent_policy",
            rule.agent_policy.as_deref().unwrap_or_default(),
        ));
    }
    if rule
        .status
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("human_only_blocked"))
    {
        return Some(("status", rule.status.as_deref().unwrap_or_default()));
    }
    if rule
        .execution_policy
        .as_deref()
        .is_some_and(|value| value.to_ascii_lowercase().contains("human_only"))
    {
        return Some((
            "execution_policy",
            rule.execution_policy.as_deref().unwrap_or_default(),
        ));
    }
    None
}

fn rule_is_denied(rule: &RoadmapTaskRule) -> bool {
    deny_basis(rule).is_some()
}
