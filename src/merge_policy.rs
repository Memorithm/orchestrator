use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoMergeScope {
    OrchestratorValidated,
    Trusted,
}

impl AutoMergeScope {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "orchestrator-validated" | "orchestrator" => Ok(Self::OrchestratorValidated),
            "trusted" => Ok(Self::Trusted),
            other => Err(format!(
                "invalid ORCHESTRATOR_AUTO_MERGE_SCOPE={other}; expected orchestrator-validated or trusted"
            )),
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::OrchestratorValidated => "orchestrator-validated",
            Self::Trusted => "trusted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeMetadata {
    pub(crate) author: String,
    pub(crate) head_branch: String,
    pub(crate) head_sha: String,
    pub(crate) base_branch: String,
    pub(crate) cross_repository: bool,
}

impl MergeMetadata {
    pub(crate) fn parse_tsv(line: &str) -> Result<Self, String> {
        let fields = line.trim_end().split('\t').collect::<Vec<_>>();
        if fields.len() != 5 {
            return Err(format!(
                "expected 5 PR merge metadata fields, got {}",
                fields.len()
            ));
        }
        let cross_repository = match fields[4] {
            "true" => true,
            "false" => false,
            other => return Err(format!("invalid cross-repository flag: {other}")),
        };
        validate_nonempty("PR author", fields[0])?;
        validate_ref_text("PR head branch", fields[1])?;
        validate_commit(fields[2])?;
        validate_ref_text("PR base branch", fields[3])?;
        Ok(Self {
            author: fields[0].to_owned(),
            head_branch: fields[1].to_owned(),
            head_sha: fields[2].to_ascii_lowercase(),
            base_branch: fields[3].to_owned(),
            cross_repository,
        })
    }

    pub(crate) fn validate_static(
        &self,
        trusted_login: &str,
        expected_base: &str,
    ) -> Result<(), String> {
        if self.cross_repository {
            return Err("autonomous merge refuses cross-repository PRs".to_owned());
        }
        if self.author != trusted_login {
            return Err(format!(
                "autonomous merge requires trusted author {trusted_login}, got {}",
                self.author
            ));
        }
        if self.base_branch != expected_base {
            return Err(format!(
                "autonomous merge requires base {expected_base}, got {}",
                self.base_branch
            ));
        }
        validate_commit(&self.head_sha)
    }

    pub(crate) fn orchestrator_branch(&self) -> bool {
        self.head_branch.starts_with("orchestrator/")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidationAttestation {
    pub(crate) repository: String,
    pub(crate) pr_number: u64,
    pub(crate) head_sha: String,
    pub(crate) validated_at: u64,
}

impl ValidationAttestation {
    pub(crate) fn new(
        repository: &str,
        pr_number: u64,
        head_sha: &str,
        validated_at: u64,
    ) -> Result<Self, String> {
        validate_repository(repository)?;
        if pr_number == 0 {
            return Err("PR number must be non-zero".to_owned());
        }
        validate_commit(head_sha)?;
        Ok(Self {
            repository: repository.to_owned(),
            pr_number,
            head_sha: head_sha.to_ascii_lowercase(),
            validated_at,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AttestationStore {
    root: PathBuf,
}

impl AttestationStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn save(&self, attestation: &ValidationAttestation) -> Result<(), String> {
        let path = self.path(attestation.repository.as_str(), attestation.pr_number)?;
        let parent = path
            .parent()
            .ok_or_else(|| format!("attestation path has no parent: {}", path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        let temporary = path.with_extension(format!("state.tmp.{}", std::process::id()));
        let contents = format!(
            "v1\nrepository={}\npr_number={}\nhead_sha={}\nvalidated_at={}\n",
            attestation.repository,
            attestation.pr_number,
            attestation.head_sha,
            attestation.validated_at
        );
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("failed to open {}: {error}", temporary.display()))?;
        use std::io::Write;
        file.write_all(contents.as_bytes())
            .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("failed to sync {}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, &path).map_err(|error| {
            format!(
                "failed to atomically replace attestation {} with {}: {error}",
                path.display(),
                temporary.display()
            )
        })
    }

    pub(crate) fn load(
        &self,
        repository: &str,
        pr_number: u64,
    ) -> Result<Option<ValidationAttestation>, String> {
        let path = self.path(repository, pr_number)?;
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| format!("failed to read attestation {}: {error}", path.display()))?;
        parse_attestation(&contents)
            .map(Some)
            .map_err(|error| format!("invalid attestation {}: {error}", path.display()))
    }

    pub(crate) fn matches_head(
        &self,
        repository: &str,
        pr_number: u64,
        head_sha: &str,
    ) -> Result<bool, String> {
        validate_commit(head_sha)?;
        Ok(self
            .load(repository, pr_number)?
            .is_some_and(|attestation| attestation.head_sha.eq_ignore_ascii_case(head_sha)))
    }

    fn path(&self, repository: &str, pr_number: u64) -> Result<PathBuf, String> {
        validate_repository(repository)?;
        if pr_number == 0 {
            return Err("PR number must be non-zero".to_owned());
        }
        Ok(self
            .root
            .join(repository.replace('/', "__"))
            .join(format!("pr-{pr_number}.state")))
    }
}

pub(crate) fn provenance_allows_merge(
    scope: AutoMergeScope,
    metadata: &MergeMetadata,
    attested_exact_head: bool,
) -> bool {
    match scope {
        AutoMergeScope::Trusted => true,
        AutoMergeScope::OrchestratorValidated => {
            metadata.orchestrator_branch() || attested_exact_head
        }
    }
}

fn validate_nonempty(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.bytes().any(|byte| matches!(byte, b'\n' | b'\r' | 0)) {
        return Err(format!("invalid {label}"));
    }
    Ok(())
}

fn validate_ref_text(label: &str, value: &str) -> Result<(), String> {
    validate_nonempty(label, value)?;
    if value.len() > 256 || value.starts_with('-') || value.contains("..") || value.ends_with('/') {
        return Err(format!("invalid {label}: {value}"));
    }
    Ok(())
}

fn validate_commit(commit: &str) -> Result<(), String> {
    if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid Git commit id: {commit}"));
    }
    Ok(())
}

fn validate_repository(repository: &str) -> Result<(), String> {
    validate_nonempty("repository", repository)?;
    let Some((owner, name)) = repository.split_once('/') else {
        return Err(format!("repository must be owner/name: {repository}"));
    };
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return Err(format!("repository must be owner/name: {repository}"));
    }
    Ok(())
}

fn parse_attestation(contents: &str) -> Result<ValidationAttestation, String> {
    let mut lines = contents.lines();
    if lines.next().unwrap_or_default() != "v1" {
        return Err("unsupported attestation version".to_owned());
    }
    let mut repository = None;
    let mut pr_number = None;
    let mut head_sha = None;
    let mut validated_at = None;
    let mut seen = std::collections::BTreeSet::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed attestation field: {line}"))?;
        if !seen.insert(name.to_owned()) {
            return Err(format!("duplicate attestation field: {name}"));
        }
        match name {
            "repository" => repository = Some(value.to_owned()),
            "pr_number" => {
                pr_number = Some(
                    value
                        .parse::<u64>()
                        .map_err(|error| format!("invalid PR number {value}: {error}"))?,
                );
            }
            "head_sha" => head_sha = Some(value.to_owned()),
            "validated_at" => {
                validated_at = Some(
                    value
                        .parse::<u64>()
                        .map_err(|error| format!("invalid validated_at {value}: {error}"))?,
                );
            }
            other => return Err(format!("unknown attestation field: {other}")),
        }
    }
    ValidationAttestation::new(
        repository
            .as_deref()
            .ok_or_else(|| "missing repository".to_owned())?,
        pr_number.ok_or_else(|| "missing pr_number".to_owned())?,
        head_sha
            .as_deref()
            .ok_or_else(|| "missing head_sha".to_owned())?,
        validated_at.ok_or_else(|| "missing validated_at".to_owned())?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "orchestrator-merge-policy-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn metadata(branch: &str) -> MergeMetadata {
        MergeMetadata {
            author: "CHECKUPAUTO".to_owned(),
            head_branch: branch.to_owned(),
            head_sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            base_branch: "main".to_owned(),
            cross_repository: false,
        }
    }

    #[test]
    fn strict_scope_requires_orchestrator_branch_or_exact_attestation() {
        let user_branch = metadata("feature/manual");
        assert!(!provenance_allows_merge(
            AutoMergeScope::OrchestratorValidated,
            &user_branch,
            false
        ));
        assert!(provenance_allows_merge(
            AutoMergeScope::OrchestratorValidated,
            &user_branch,
            true
        ));
        assert!(provenance_allows_merge(
            AutoMergeScope::OrchestratorValidated,
            &metadata("orchestrator/issue-7-123"),
            false
        ));
    }

    #[test]
    fn trusted_scope_still_requires_static_validation_separately() {
        assert!(provenance_allows_merge(
            AutoMergeScope::Trusted,
            &metadata("feature/manual"),
            false
        ));
    }

    #[test]
    fn static_validation_rejects_wrong_author_base_or_cross_repo() {
        let good = metadata("orchestrator/issue-7-123");
        good.validate_static("CHECKUPAUTO", "main").unwrap();

        let mut wrong_author = good.clone();
        wrong_author.author = "external".to_owned();
        assert!(wrong_author.validate_static("CHECKUPAUTO", "main").is_err());

        let mut wrong_base = good.clone();
        wrong_base.base_branch = "release".to_owned();
        assert!(wrong_base.validate_static("CHECKUPAUTO", "main").is_err());

        let mut cross = good;
        cross.cross_repository = true;
        assert!(cross.validate_static("CHECKUPAUTO", "main").is_err());
    }

    #[test]
    fn attestation_is_atomic_and_bound_to_exact_head() {
        let root = temp_root("attestation");
        let store = AttestationStore::new(root.clone());
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let attestation = ValidationAttestation::new("Memorithm/ADA", 34, sha, 123).unwrap();
        store.save(&attestation).unwrap();
        assert!(store.matches_head("Memorithm/ADA", 34, sha).unwrap());
        assert!(
            !store
                .matches_head(
                    "Memorithm/ADA",
                    34,
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                )
                .unwrap()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn metadata_parser_is_fail_closed() {
        let parsed = MergeMetadata::parse_tsv(
            "CHECKUPAUTO\torchestrator/issue-7-123\t0123456789abcdef0123456789abcdef01234567\tmain\tfalse",
        )
        .unwrap();
        assert!(parsed.orchestrator_branch());
        assert!(MergeMetadata::parse_tsv("too\tfew\tfields").is_err());
        assert!(MergeMetadata::parse_tsv("CHECKUPAUTO\tbranch\tnotasha\tmain\tfalse").is_err());
    }
}
