fn explicit_task_action_category(body: &str) -> Result<Option<AutonomousActionCategory>, String> {
    let mut selected = None;
    for line in body.lines() {
        let line = line.trim().trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, '-' | '*' | '>' | '#')
        });
        let Some((label, raw_value)) = line.split_once(':') else {
            continue;
        };
        let label = label.trim().trim_matches('*').to_ascii_lowercase();
        if !matches!(label.as_str(), "autonomous action" | "action category") {
            continue;
        }
        let value = parse_policy_scalar(raw_value.trim(), "action category")?;
        let category = AutonomousActionCategory::parse(&value)?;
        if selected.replace(category).is_some() {
            return Err("duplicate explicit autonomous action category in task body".to_owned());
        }
    }
    Ok(selected)
}

fn parse_autonomous_action_rules(
    document: &PolicyDocument,
) -> Result<Vec<AutonomousActionRule>, String> {
    let mut in_policy = false;
    let mut saw_section = false;
    let mut schema_version = None;
    let mut rules = BTreeMap::<AutonomousActionCategory, AutonomousActionRule>::new();

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_policy {
            if raw_line == "autonomous_action_policy:" {
                if saw_section {
                    return Err(format!(
                        "duplicate autonomous_action_policy section in origin/{}:{}",
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
                "tab indentation is not allowed in autonomous action policy origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            in_policy = false;
            if raw_line == "autonomous_action_policy:" {
                return Err(format!(
                    "duplicate autonomous_action_policy section in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            continue;
        }
        if indent != 2 {
            return Err(format!(
                "autonomous_action_policy only accepts scalar fields at indentation 2 in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let (key, raw_value) = trimmed.split_once(':').ok_or_else(|| {
            format!(
                "malformed autonomous action policy field in origin/{}:{}",
                document.ref_name, document.path
            )
        })?;
        let value = parse_policy_scalar(raw_value.trim(), key)?;
        if key == "schema_version" {
            if schema_version.replace(value.clone()).is_some() {
                return Err(format!(
                    "duplicate autonomous action schema_version in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            if value != "1" {
                return Err(format!(
                    "unsupported autonomous action policy schema_version {value} in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            continue;
        }
        let category = AutonomousActionCategory::parse(key)?;
        let decision = AutonomousActionDecision::parse(&value)?;
        let rule = AutonomousActionRule {
            category,
            decision,
            source_ref: document.ref_name.clone(),
            source_path: document.path.clone(),
            source_commit: document.commit_sha.clone(),
            source_blob: document.blob_sha.clone(),
        };
        if rules.insert(category, rule).is_some() {
            return Err(format!(
                "duplicate autonomous action category {key} in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
    }

    if saw_section && schema_version.as_deref() != Some("1") {
        return Err(format!(
            "autonomous_action_policy requires schema_version: 1 in origin/{}:{}",
            document.ref_name, document.path
        ));
    }
    Ok(rules.into_values().collect())
}

fn split_validation_argv_items(inner: &str) -> Result<Vec<&str>, String> {
    let mut items = Vec::new();
    let mut quote = None;
    let mut start = 0;
    for (index, character) in inner.char_indices() {
        match quote {
            Some(expected) if character == expected => quote = None,
            Some(_) => {}
            None if matches!(character, '\'' | '"') => quote = Some(character),
            None if character == ',' => {
                items.push(&inner[start..index]);
                start = index + character.len_utf8();
            }
            None => {}
        }
    }
    if quote.is_some() {
        return Err("validation argv contains an unterminated quoted scalar".to_owned());
    }
    items.push(&inner[start..]);
    Ok(items)
}

fn parse_validation_argv(raw: &str) -> Result<Vec<String>, String> {
    let raw = raw.trim();
    if !raw.starts_with('[') || !raw.ends_with(']') {
        return Err("validation argv must use a bracketed scalar list".to_owned());
    }
    let inner = &raw[1..raw.len() - 1];
    if inner.trim().is_empty() {
        return Err("validation argv must not be empty".to_owned());
    }
    let mut argv = Vec::new();
    for raw_arg in split_validation_argv_items(inner)? {
        if argv.len() >= MAX_VALIDATION_ARGV {
            return Err(format!(
                "validation argv exceeds {MAX_VALIDATION_ARGV} elements"
            ));
        }
        let arg = parse_policy_scalar(raw_arg.trim(), "validation argv")?;
        if arg.is_empty()
            || arg.chars().count() > MAX_VALIDATION_ARG_CHARS
            || arg.chars().any(char::is_control)
        {
            return Err("invalid validation argv element".to_owned());
        }
        argv.push(arg);
    }
    let executable = argv.first().expect("non-empty checked above");
    if executable.contains('/')
        || executable.contains('\\')
        || !executable.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '+')
        })
    {
        return Err(format!("unsafe validation executable: {executable:?}"));
    }
    if matches!(
        executable.as_str(),
        "git"
            | "gh"
            | "ssh"
            | "scp"
            | "curl"
            | "wget"
            | "bash"
            | "sh"
            | "zsh"
            | "fish"
            | "sudo"
            | "su"
            | "env"
            | "xargs"
            | "ollama"
            | "opencode"
    ) {
        return Err(format!(
            "validation executable is forbidden in portable plan v1: {executable}"
        ));
    }
    Ok(argv)
}

fn validate_validation_cwd(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value == "." {
        return Ok(value.to_owned());
    }
    if value.is_empty()
        || value.chars().count() > MAX_VALIDATION_CWD_CHARS
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return Err(format!("unsafe validation cwd: {value:?}"));
    }
    for component in value.split('/') {
        if component.is_empty() || matches!(component, "." | "..") {
            return Err(format!("unsafe validation cwd: {value:?}"));
        }
    }
    Ok(value.to_owned())
}
