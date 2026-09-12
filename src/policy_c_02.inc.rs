fn append_document(output: &mut String, kind: &str, document: &PolicyDocument) {
    output.push_str(&format!(
        "\n--- {kind}: origin/{}:{} commit={} blob={} ---\n",
        document.ref_name, document.path, document.commit_sha, document.blob_sha
    ));
    output.push_str(&document.content);
    if !document.content.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("--- end policy document ---\n");
}

pub(crate) fn load_snapshot(
    workspace: &Path,
    repository: &str,
    base_branch: &str,
    base_sha: &str,
) -> Result<PolicySnapshot, String> {
    validate_repository_name(repository)?;
    validate_ref_name(base_branch)?;
    validate_object_id(base_sha)?;
    ensure_commit_exists(workspace, base_sha)?;

    let bootstrap_blob = root_blob_sha(workspace, base_sha, "AGENTS.md")?;
    let bootstrap = match bootstrap_blob {
        Some(blob_sha) => Some(read_document(
            workspace,
            base_branch,
            "AGENTS.md",
            base_sha,
            &blob_sha,
        )?),
        None => None,
    };

    let pointers = match &bootstrap {
        Some(document) => extract_policy_pointers(&document.content)?,
        None => Vec::new(),
    };
    if pointers.len() > MAX_POLICY_DOCUMENTS {
        return Err(format!(
            "AGENTS.md references {} policy documents; maximum is {MAX_POLICY_DOCUMENTS}",
            pointers.len()
        ));
    }

    let mut documents = Vec::with_capacity(pointers.len());
    let mut total_bytes = bootstrap
        .as_ref()
        .map_or(0, |document| document.content.len());
    for pointer in pointers {
        let remote_ref = format!("refs/remotes/origin/{}", pointer.ref_name);
        let commit_sha = git_capture(
            workspace,
            &["rev-parse", "--verify", &format!("{remote_ref}^{{commit}}")],
        )?;
        validate_object_id(&commit_sha)?;
        let blob_sha = git_capture(
            workspace,
            &[
                "rev-parse",
                "--verify",
                &format!("{commit_sha}:{}", pointer.path),
            ],
        )?;
        validate_object_id(&blob_sha)?;
        let document = read_document(
            workspace,
            &pointer.ref_name,
            &pointer.path,
            &commit_sha,
            &blob_sha,
        )?;
        total_bytes = total_bytes
            .checked_add(document.content.len())
            .ok_or_else(|| "policy byte accounting overflow".to_owned())?;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(format!(
                "repository policy snapshot exceeds {MAX_TOTAL_BYTES} total bytes"
            ));
        }
        documents.push(document);
    }

    Ok(PolicySnapshot {
        repository: repository.to_owned(),
        base_branch: base_branch.to_owned(),
        base_sha: base_sha.to_owned(),
        bootstrap,
        documents,
    })
}

pub(crate) fn remote_identity_is_current(
    workspace: &Path,
    snapshot: &PolicySnapshot,
) -> Result<bool, String> {
    let mut expected = BTreeMap::<String, String>::new();
    expected.insert(snapshot.base_branch.clone(), snapshot.base_sha.clone());
    for document in &snapshot.documents {
        match expected.get(&document.ref_name) {
            Some(existing) if existing != &document.commit_sha => {
                return Err(format!(
                    "policy snapshot contains conflicting commit identities for origin/{}",
                    document.ref_name
                ));
            }
            Some(_) => {}
            None => {
                expected.insert(document.ref_name.clone(), document.commit_sha.clone());
            }
        }
    }

    for (ref_name, expected_sha) in expected {
        let live = remote_branch_head(workspace, &ref_name)?;
        if live != expected_sha {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn persist_identity(
    data_root: &Path,
    repository: &str,
    kind: &str,
    number: u64,
    timestamp: u64,
    snapshot: &PolicySnapshot,
) -> Result<PathBuf, String> {
    validate_repository_name(repository)?;
    let directory = data_root
        .join("state/policy-snapshots")
        .join(repository.replace('/', "__"))
        .join(format!("{}-{number}", safe_component(kind)));
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "failed to create policy evidence directory {}: {error}",
            directory.display()
        )
    })?;

    for sequence in 0..1_024_u16 {
        let suffix = if sequence == 0 {
            String::new()
        } else {
            format!("-{sequence}")
        };
        let path = directory.join(format!(
            "{timestamp}-{}-{}{}.txt",
            std::process::id(),
            short_object_id(snapshot.base_sha()),
            suffix
        ));
        let mut file = match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create policy evidence {}: {error}",
                    path.display()
                ));
            }
        };
        file.write_all(snapshot.identity_record().as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| {
                format!(
                    "failed to write policy evidence {}: {error}",
                    path.display()
                )
            })?;
        file.sync_all().map_err(|error| {
            format!("failed to sync policy evidence {}: {error}", path.display())
        })?;
        return Ok(path);
    }

    Err("policy evidence sequence exhausted for one timestamp".to_owned())
}

fn short_object_id(value: &str) -> &str {
    value.get(..12).unwrap_or(value)
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn extract_policy_pointers(bootstrap: &str) -> Result<Vec<PolicyPointer>, String> {
    let mut pointers = BTreeSet::new();
    for raw in bootstrap.split_whitespace() {
        let token = raw.trim_matches(|character: char| {
            matches!(
                character,
                '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '&' | '\\'
            )
        });
        let Some(rest) = token.strip_prefix("origin/") else {
            continue;
        };
        let Some((ref_name, path)) = rest.split_once(':') else {
            continue;
        };
        let path = path.trim_matches(|character: char| {
            matches!(character, '`' | '"' | '\'' | ')' | ']' | '}' | ',' | ';')
        });
        validate_ref_name(ref_name)?;
        validate_policy_path(path)?;
        pointers.insert(PolicyPointer {
            ref_name: ref_name.to_owned(),
            path: path.to_owned(),
        });
    }
    Ok(pointers.into_iter().collect())
}

fn validate_repository_name(repository: &str) -> Result<(), String> {
    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| format!("invalid repository identity: {repository:?}"))?;
    if owner.is_empty()
        || name.is_empty()
        || name.contains('/')
        || !owner.chars().chain(name.chars()).all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(format!("invalid repository identity: {repository:?}"));
    }
    Ok(())
}

fn validate_ref_name(ref_name: &str) -> Result<(), String> {
    if ref_name.is_empty() || ref_name.chars().count() > MAX_REF_CHARS {
        return Err(format!("unsafe policy ref: {ref_name:?}"));
    }
    if ref_name.starts_with('/')
        || ref_name.ends_with('/')
        || ref_name.contains("..")
        || ref_name.contains("@{")
        || ref_name.ends_with('.')
        || ref_name.ends_with(".lock")
    {
        return Err(format!("unsafe policy ref: {ref_name:?}"));
    }
    for component in ref_name.split('/') {
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with(".lock")
            || !component.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
        {
            return Err(format!("unsafe policy ref: {ref_name:?}"));
        }
    }
    Ok(())
}

fn validate_policy_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.chars().count() > MAX_PATH_CHARS
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
    {
        return Err(format!("unsafe policy path: {path:?}"));
    }
    for component in path.split('/') {
        if component.is_empty() || matches!(component, "." | "..") {
            return Err(format!("unsafe policy path: {path:?}"));
        }
    }
    Ok(())
}
