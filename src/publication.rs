use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const PUBLICATION_VERSION: &str = "v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicationPhase {
    Prepared,
    Pushed,
}

impl PublicationPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Pushed => "pushed",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "pushed" => Ok(Self::Pushed),
            other => Err(format!("unknown publication phase: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingPublication {
    pub(crate) branch: String,
    pub(crate) commit: String,
    pub(crate) base_branch: String,
    pub(crate) phase: PublicationPhase,
}

impl PendingPublication {
    pub(crate) fn new(
        branch: String,
        commit: String,
        base_branch: String,
        phase: PublicationPhase,
    ) -> Result<Self, String> {
        validate_ref_component("branch", &branch)?;
        validate_ref_component("base branch", &base_branch)?;
        validate_commit(&commit)?;
        Ok(Self {
            branch,
            commit,
            base_branch,
            phase,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicationKey {
    repository: String,
    issue_number: u64,
}

impl PublicationKey {
    pub(crate) fn new(repository: &str, issue_number: u64) -> Self {
        Self {
            repository: repository.to_owned(),
            issue_number,
        }
    }

    fn path(&self, root: &Path) -> PathBuf {
        root.join(self.repository.replace('/', "__"))
            .join(format!("issue-{}.state", self.issue_number))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PublicationStore {
    root: PathBuf,
}

impl PublicationStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn load(&self, key: &PublicationKey) -> Result<Option<PendingPublication>, String> {
        let path = key.path(&self.root);
        if !path.exists() {
            return Ok(None);
        }
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read publication state {}: {error}", path.display()))?;
        parse_publication(&contents)
            .map(Some)
            .map_err(|error| format!("invalid publication state {}: {error}", path.display()))
    }

    pub(crate) fn save(
        &self,
        key: &PublicationKey,
        publication: &PendingPublication,
    ) -> Result<(), String> {
        validate_ref_component("branch", &publication.branch)?;
        validate_ref_component("base branch", &publication.base_branch)?;
        validate_commit(&publication.commit)?;

        let path = key.path(&self.root);
        let parent = path
            .parent()
            .ok_or_else(|| format!("publication state path has no parent: {}", path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;

        let temporary = path.with_extension(format!("state.tmp.{}", std::process::id()));
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("failed to open {}: {error}", temporary.display()))?;
        file.write_all(serialize_publication(publication).as_bytes())
            .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("failed to sync {}: {error}", temporary.display()))?;
        fs::rename(&temporary, &path).map_err(|error| {
            format!(
                "failed to atomically replace publication state {} with {}: {error}",
                path.display(),
                temporary.display()
            )
        })?;
        Ok(())
    }

    pub(crate) fn clear(&self, key: &PublicationKey) -> Result<(), String> {
        let path = key.path(&self.root);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "failed to remove publication state {}: {error}",
                path.display()
            )),
        }
    }
}

fn validate_ref_component(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.contains(['\n', '\r', '\0']) {
        return Err(format!("invalid {label}: control characters or empty value"));
    }
    Ok(())
}

fn validate_commit(commit: &str) -> Result<(), String> {
    if !(40..=64).contains(&commit.len()) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid commit id: {commit}"));
    }
    Ok(())
}

fn serialize_publication(publication: &PendingPublication) -> String {
    format!(
        "{PUBLICATION_VERSION}\nbranch={}\ncommit={}\nbase_branch={}\nphase={}\n",
        publication.branch,
        publication.commit,
        publication.base_branch,
        publication.phase.as_str()
    )
}

fn parse_publication(contents: &str) -> Result<PendingPublication, String> {
    let mut lines = contents.lines();
    let version = lines.next().unwrap_or_default();
    if version != PUBLICATION_VERSION {
        return Err(format!("unsupported publication state version: {version}"));
    }

    let mut branch = None;
    let mut commit = None;
    let mut base_branch = None;
    let mut phase = None;
    let mut seen = std::collections::BTreeSet::new();

    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed publication field: {line}"))?;
        if !seen.insert(name.to_owned()) {
            return Err(format!("duplicate publication field: {name}"));
        }
        match name {
            "branch" => branch = Some(value.to_owned()),
            "commit" => commit = Some(value.to_owned()),
            "base_branch" => base_branch = Some(value.to_owned()),
            "phase" => phase = Some(PublicationPhase::parse(value)?),
            other => return Err(format!("unknown publication field: {other}")),
        }
    }

    PendingPublication::new(
        branch.ok_or_else(|| "missing branch".to_owned())?,
        commit.ok_or_else(|| "missing commit".to_owned())?,
        base_branch.ok_or_else(|| "missing base_branch".to_owned())?,
        phase.ok_or_else(|| "missing phase".to_owned())?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "orchestrator-publication-test-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn sample(phase: PublicationPhase) -> PendingPublication {
        PendingPublication::new(
            "orchestrator/issue-7-123".to_owned(),
            "0123456789abcdef0123456789abcdef01234567".to_owned(),
            "main".to_owned(),
            phase,
        )
        .unwrap()
    }

    #[test]
    fn publication_round_trips_atomically() {
        let root = temporary_root("roundtrip");
        let store = PublicationStore::new(root.clone());
        let key = PublicationKey::new("Memorithm/ADA", 7);
        let publication = sample(PublicationPhase::Prepared);

        store.save(&key, &publication).unwrap();
        assert_eq!(store.load(&key).unwrap(), Some(publication));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pushed_phase_is_persisted() {
        let root = temporary_root("pushed");
        let store = PublicationStore::new(root.clone());
        let key = PublicationKey::new("Memorithm/ADA", 7);

        store.save(&key, &sample(PublicationPhase::Prepared)).unwrap();
        store.save(&key, &sample(PublicationPhase::Pushed)).unwrap();
        assert_eq!(
            store.load(&key).unwrap().unwrap().phase,
            PublicationPhase::Pushed
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn clear_is_idempotent() {
        let root = temporary_root("clear");
        let store = PublicationStore::new(root.clone());
        let key = PublicationKey::new("Memorithm/TDI", 57);

        store.save(&key, &sample(PublicationPhase::Prepared)).unwrap();
        store.clear(&key).unwrap();
        store.clear(&key).unwrap();
        assert_eq!(store.load(&key).unwrap(), None);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_or_future_state_fails_closed() {
        assert!(parse_publication("v99\nbranch=x\n").is_err());
        assert!(PendingPublication::new(
            "branch".to_owned(),
            "not-a-commit".to_owned(),
            "main".to_owned(),
            PublicationPhase::Prepared,
        )
        .is_err());
    }
}
