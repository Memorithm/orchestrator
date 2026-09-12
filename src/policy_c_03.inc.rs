fn validate_object_id(value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid Git object id: {value:?}"));
    }
    Ok(())
}

fn ensure_commit_exists(workspace: &Path, commit: &str) -> Result<(), String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["cat-file", "-e", &format!("{commit}^{{commit}}")])
        .output()
        .map_err(|error| format!("failed to execute git cat-file: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "selected base commit {commit} is unavailable in local clone: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn root_blob_sha(workspace: &Path, commit: &str, path: &str) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["ls-tree", commit, "--", path])
        .output()
        .map_err(|error| format!("failed to execute git ls-tree: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-tree failed for {commit}:{path}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("invalid UTF-8 from git ls-tree: {error}"))?;
    if stdout.trim().is_empty() {
        return Ok(None);
    }
    let mut lines = stdout.lines();
    let line = lines
        .next()
        .ok_or_else(|| format!("missing git ls-tree result for {commit}:{path}"))?;
    if lines.next().is_some() {
        return Err(format!("ambiguous git ls-tree result for {commit}:{path}"));
    }
    let metadata = line
        .split_once('\t')
        .ok_or_else(|| format!("malformed git ls-tree result for {commit}:{path}: {line}"))?
        .0;
    let fields = metadata.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 3 || fields[1] != "blob" {
        return Err(format!("{commit}:{path} is not a regular Git blob"));
    }
    validate_object_id(fields[2])?;
    Ok(Some(fields[2].to_owned()))
}

fn read_document(
    workspace: &Path,
    ref_name: &str,
    path: &str,
    commit_sha: &str,
    blob_sha: &str,
) -> Result<PolicyDocument, String> {
    let size = git_capture(workspace, &["cat-file", "-s", blob_sha])?
        .parse::<u64>()
        .map_err(|error| format!("invalid size for policy blob {blob_sha}: {error}"))?;
    if size > MAX_DOCUMENT_BYTES {
        return Err(format!(
            "policy document origin/{ref_name}:{path} is {size} bytes; maximum is {MAX_DOCUMENT_BYTES}"
        ));
    }
    let content = git_capture_raw(workspace, &["cat-file", "-p", blob_sha])?;
    if content.len() as u64 != size {
        return Err(format!(
            "policy blob {blob_sha} size changed while reading: expected {size}, got {}",
            content.len()
        ));
    }
    Ok(PolicyDocument {
        ref_name: ref_name.to_owned(),
        path: path.to_owned(),
        commit_sha: commit_sha.to_owned(),
        blob_sha: blob_sha.to_owned(),
        content,
    })
}

fn remote_branch_head(workspace: &Path, ref_name: &str) -> Result<String, String> {
    validate_ref_name(ref_name)?;
    let reference = format!("refs/heads/{ref_name}");
    let output = git_capture(
        workspace,
        &["ls-remote", "--heads", "origin", reference.as_str()],
    )?;
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let line = lines
        .next()
        .ok_or_else(|| format!("mandatory policy ref origin/{ref_name} is no longer advertised"))?;
    if lines.next().is_some() {
        return Err(format!(
            "mandatory policy ref origin/{ref_name} advertised multiple heads"
        ));
    }
    let mut fields = line.split_whitespace();
    let sha = fields
        .next()
        .ok_or_else(|| format!("missing object id for origin/{ref_name}"))?;
    let advertised = fields
        .next()
        .ok_or_else(|| format!("missing ref name for origin/{ref_name}"))?;
    if fields.next().is_some() || advertised != reference {
        return Err(format!(
            "unexpected remote advertisement for origin/{ref_name}: {line}"
        ));
    }
    validate_object_id(sha)?;
    Ok(sha.to_ascii_lowercase())
}

fn git_capture(workspace: &Path, args: &[&str]) -> Result<String, String> {
    git_capture_raw(workspace, args).map(|value| value.trim().to_owned())
}

fn git_capture_raw(workspace: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(args)
        .output()
        .map_err(|error| format!("failed to execute git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            workspace.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("invalid UTF-8 from git {}: {error}", args.join(" ")))
}
