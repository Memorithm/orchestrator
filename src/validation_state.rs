use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const VALIDATION_PLAN_STATE_VERSION: &str = "v1";
const MAX_DECLARED_STEPS: usize = 24;
const MAX_REPOSITORY_CHARS: usize = 256;
const MAX_WORK_KIND_CHARS: usize = 64;
const MAX_POLICY_IDENTITY_CHARS: usize = 65_536;
const MAX_SOURCE_CHARS: usize = 1_024;
const MAX_ATTEMPT_ID_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalStatus {
    Passed,
    Failed,
    TimedOut,
}

impl TerminalStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            "timed_out" => Ok(Self::TimedOut),
            other => Err(format!("unknown validation plan terminal status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptPhase {
    InProgress,
    Terminal(TerminalStatus),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanBinding {
    repository: String,
    work_kind: String,
    work_number: u64,
    binding_identity: String,
    plan_identity: String,
    policy_identity: String,
    base_sha: String,
    worktree_head: String,
    worktree_tree: String,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
    declared_steps: usize,
}

impl PlanBinding {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        repository: String,
        work_kind: String,
        work_number: u64,
        binding_identity: String,
        plan_identity: String,
        policy_identity: String,
        base_sha: String,
        worktree_head: String,
        worktree_tree: String,
        source_ref: String,
        source_path: String,
        source_commit: String,
        source_blob: String,
        declared_steps: usize,
    ) -> Result<Self, String> {
        validate_text("repository", &repository, MAX_REPOSITORY_CHARS)?;
        validate_path_token("work kind", &work_kind, MAX_WORK_KIND_CHARS)?;
        validate_hash("binding identity", &binding_identity)?;
        validate_hash("plan identity", &plan_identity)?;
        validate_hex(
            "policy identity",
            &policy_identity,
            MAX_POLICY_IDENTITY_CHARS,
        )?;
        validate_hash("base SHA", &base_sha)?;
        validate_hash("worktree HEAD", &worktree_head)?;
        validate_hash("worktree tree", &worktree_tree)?;
        validate_text("source ref", &source_ref, MAX_SOURCE_CHARS)?;
        validate_text("source path", &source_path, MAX_SOURCE_CHARS)?;
        validate_hash("source commit", &source_commit)?;
        validate_hash("source blob", &source_blob)?;
        if !(1..=MAX_DECLARED_STEPS).contains(&declared_steps) {
            return Err(format!(
                "invalid validation plan declared step count: {declared_steps}"
            ));
        }
        Ok(Self {
            repository,
            work_kind,
            work_number,
            binding_identity,
            plan_identity,
            policy_identity,
            base_sha,
            worktree_head,
            worktree_tree,
            source_ref,
            source_path,
            source_commit,
            source_blob,
            declared_steps,
        })
    }

    pub(crate) fn binding_identity(&self) -> &str {
        &self.binding_identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanAttempt {
    binding: PlanBinding,
    attempt_id: String,
    started_at: u64,
    finished_at: Option<u64>,
    completed_steps: usize,
    phase: AttemptPhase,
}

impl PlanAttempt {
    pub(crate) fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ValidationPlanStore {
    root: PathBuf,
}

impl ValidationPlanStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn reusable_passed(&self, binding: &PlanBinding) -> Result<bool, String> {
        validate_binding(binding)?;
        let path = self.passed_path(binding);
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "failed to read portable validation passed result {}: {error}",
                    path.display()
                ));
            }
        };
        let attempt = parse_attempt(&contents).map_err(|error| {
            format!(
                "invalid portable validation passed result {}: {error}",
                path.display()
            )
        })?;
        if attempt.binding != *binding {
            return Err(format!(
                "portable validation passed result {} does not match its binding identity",
                path.display()
            ));
        }
        if attempt.phase != AttemptPhase::Terminal(TerminalStatus::Passed)
            || attempt.completed_steps != binding.declared_steps
            || attempt.finished_at.is_none()
        {
            return Err(format!(
                "portable validation passed result {} is not an authoritative complete pass",
                path.display()
            ));
        }
        Ok(true)
    }

    pub(crate) fn begin(
        &self,
        binding: PlanBinding,
        attempt_id: String,
        started_at: u64,
    ) -> Result<PlanAttempt, String> {
        validate_binding(&binding)?;
        validate_attempt_id(&attempt_id)?;
        let current_path = self.current_path(&binding);
        if current_path.exists() {
            let contents = fs::read_to_string(&current_path).map_err(|error| {
                format!(
                    "failed to read portable validation current attempt {}: {error}",
                    current_path.display()
                )
            })?;
            let previous = parse_attempt(&contents).map_err(|error| {
                format!(
                    "invalid portable validation current attempt {}: {error}",
                    current_path.display()
                )
            })?;
            if previous.phase == AttemptPhase::InProgress {
                self.archive(&previous, "interrupted")?;
            }
        }
        match fs::remove_file(self.passed_path(&binding)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to invalidate prior portable validation pass: {error}"
                ));
            }
        }
        let attempt = PlanAttempt {
            binding,
            attempt_id,
            started_at,
            finished_at: None,
            completed_steps: 0,
            phase: AttemptPhase::InProgress,
        };
        self.save_current(&attempt)?;
        Ok(attempt)
    }

    pub(crate) fn update_progress(
        &self,
        attempt: &mut PlanAttempt,
        completed_steps: usize,
    ) -> Result<(), String> {
        validate_attempt(attempt)?;
        if attempt.phase != AttemptPhase::InProgress {
            return Err("cannot update terminal portable validation attempt".to_owned());
        }
        if completed_steps < attempt.completed_steps
            || completed_steps > attempt.binding.declared_steps
        {
            return Err(format!(
                "invalid portable validation completed step count transition: {} -> {completed_steps}",
                attempt.completed_steps
            ));
        }
        attempt.completed_steps = completed_steps;
        self.save_current(attempt)
    }

    pub(crate) fn finish(
        &self,
        attempt: &mut PlanAttempt,
        status: TerminalStatus,
        completed_steps: usize,
        finished_at: u64,
    ) -> Result<PathBuf, String> {
        validate_attempt(attempt)?;
        if attempt.phase != AttemptPhase::InProgress {
            return Err("cannot finish terminal portable validation attempt twice".to_owned());
        }
        if completed_steps < attempt.completed_steps
            || completed_steps > attempt.binding.declared_steps
        {
            return Err(format!(
                "invalid portable validation terminal step count: {completed_steps}"
            ));
        }
        if status == TerminalStatus::Passed && completed_steps != attempt.binding.declared_steps {
            return Err("portable validation pass requires all declared steps".to_owned());
        }
        if status != TerminalStatus::Passed && completed_steps == 0 {
            return Err(
                "portable validation failure/timeout requires one attempted step".to_owned(),
            );
        }
        if finished_at < attempt.started_at {
            return Err("portable validation terminal timestamp predates start".to_owned());
        }
        attempt.completed_steps = completed_steps;
        attempt.finished_at = Some(finished_at);
        attempt.phase = AttemptPhase::Terminal(status);
        validate_attempt(attempt)?;

        let history = self.archive(attempt, "terminal")?;
        self.save_current(attempt)?;
        if status == TerminalStatus::Passed {
            atomic_write(
                &self.passed_path(&attempt.binding),
                &serialize_attempt(attempt),
            )?;
        }
        Ok(history)
    }

    fn work_root(&self, binding: &PlanBinding) -> PathBuf {
        self.root
            .join(hex_component(&binding.repository))
            .join(format!("{}-{}", binding.work_kind, binding.work_number))
    }

    fn current_path(&self, binding: &PlanBinding) -> PathBuf {
        self.work_root(binding).join("current.state")
    }

    fn history_root(&self, binding: &PlanBinding) -> PathBuf {
        self.work_root(binding).join("history")
    }

    fn passed_path(&self, binding: &PlanBinding) -> PathBuf {
        self.work_root(binding)
            .join("passed")
            .join(format!("{}.state", binding.binding_identity))
    }

    fn save_current(&self, attempt: &PlanAttempt) -> Result<(), String> {
        atomic_write(
            &self.current_path(&attempt.binding),
            &serialize_attempt(attempt),
        )
    }

    fn archive(&self, attempt: &PlanAttempt, suffix: &str) -> Result<PathBuf, String> {
        let directory = self.history_root(&attempt.binding);
        fs::create_dir_all(&directory).map_err(|error| {
            format!(
                "failed to create portable validation history {}: {error}",
                directory.display()
            )
        })?;
        let record = serialize_attempt(attempt);
        for sequence in 0..1_024_u16 {
            let path = directory.join(format!("{}-{suffix}-{sequence}.state", attempt.attempt_id));
            let mut file = match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "failed to create portable validation history {}: {error}",
                        path.display()
                    ));
                }
            };
            file.write_all(record.as_bytes()).map_err(|error| {
                format!(
                    "failed to write portable validation history {}: {error}",
                    path.display()
                )
            })?;
            file.sync_all().map_err(|error| {
                format!(
                    "failed to sync portable validation history {}: {error}",
                    path.display()
                )
            })?;
            return Ok(path);
        }
        Err("portable validation history sequence exhausted".to_owned())
    }
}

fn validate_binding(binding: &PlanBinding) -> Result<(), String> {
    PlanBinding::new(
        binding.repository.clone(),
        binding.work_kind.clone(),
        binding.work_number,
        binding.binding_identity.clone(),
        binding.plan_identity.clone(),
        binding.policy_identity.clone(),
        binding.base_sha.clone(),
        binding.worktree_head.clone(),
        binding.worktree_tree.clone(),
        binding.source_ref.clone(),
        binding.source_path.clone(),
        binding.source_commit.clone(),
        binding.source_blob.clone(),
        binding.declared_steps,
    )
    .map(|_| ())
}

fn validate_attempt(attempt: &PlanAttempt) -> Result<(), String> {
    validate_binding(&attempt.binding)?;
    validate_attempt_id(&attempt.attempt_id)?;
    if attempt.completed_steps > attempt.binding.declared_steps {
        return Err("portable validation completed step count exceeds declared steps".to_owned());
    }
    match attempt.phase {
        AttemptPhase::InProgress => {
            if attempt.finished_at.is_some() {
                return Err("in-progress portable validation attempt has finished_at".to_owned());
            }
        }
        AttemptPhase::Terminal(status) => {
            let finished_at = attempt.finished_at.ok_or_else(|| {
                "terminal portable validation attempt is missing finished_at".to_owned()
            })?;
            if finished_at < attempt.started_at {
                return Err("portable validation finished_at predates started_at".to_owned());
            }
            if status == TerminalStatus::Passed
                && attempt.completed_steps != attempt.binding.declared_steps
            {
                return Err("terminal passed validation did not complete all steps".to_owned());
            }
            if status != TerminalStatus::Passed && attempt.completed_steps == 0 {
                return Err("terminal failed validation completed zero steps".to_owned());
            }
        }
    }
    Ok(())
}

fn serialize_attempt(attempt: &PlanAttempt) -> String {
    let (phase, status) = match attempt.phase {
        AttemptPhase::InProgress => ("in_progress", "none"),
        AttemptPhase::Terminal(status) => ("terminal", status.as_str()),
    };
    format!(
        "{VALIDATION_PLAN_STATE_VERSION}\nrepository={}\nwork_kind={}\nwork_number={}\nbinding_identity={}\nplan_identity={}\npolicy_identity={}\nbase_sha={}\nworktree_head={}\nworktree_tree={}\nsource_ref={}\nsource_path={}\nsource_commit={}\nsource_blob={}\ndeclared_steps={}\nattempt_id={}\nstarted_at={}\nfinished_at={}\ncompleted_steps={}\nphase={}\nstatus={}\n",
        attempt.binding.repository,
        attempt.binding.work_kind,
        attempt.binding.work_number,
        attempt.binding.binding_identity,
        attempt.binding.plan_identity,
        attempt.binding.policy_identity,
        attempt.binding.base_sha,
        attempt.binding.worktree_head,
        attempt.binding.worktree_tree,
        attempt.binding.source_ref,
        attempt.binding.source_path,
        attempt.binding.source_commit,
        attempt.binding.source_blob,
        attempt.binding.declared_steps,
        attempt.attempt_id,
        attempt.started_at,
        attempt
            .finished_at
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        attempt.completed_steps,
        phase,
        status
    )
}

fn parse_attempt(contents: &str) -> Result<PlanAttempt, String> {
    if contents.len() > 96 * 1024 {
        return Err("portable validation state exceeds size bound".to_owned());
    }
    let mut lines = contents.lines();
    let version = lines.next().unwrap_or_default();
    if version != VALIDATION_PLAN_STATE_VERSION {
        return Err(format!(
            "unsupported portable validation state version: {version}"
        ));
    }

    let mut repository = None;
    let mut work_kind = None;
    let mut work_number = None;
    let mut binding_identity = None;
    let mut plan_identity = None;
    let mut policy_identity = None;
    let mut base_sha = None;
    let mut worktree_head = None;
    let mut worktree_tree = None;
    let mut source_ref = None;
    let mut source_path = None;
    let mut source_commit = None;
    let mut source_blob = None;
    let mut declared_steps = None;
    let mut attempt_id = None;
    let mut started_at = None;
    let mut finished_at = None;
    let mut completed_steps = None;
    let mut phase = None;
    let mut status = None;
    let mut seen = BTreeSet::new();

    for line in lines {
        if line.is_empty() {
            return Err("empty field line in portable validation state".to_owned());
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed portable validation field: {line}"))?;
        if !seen.insert(name.to_owned()) {
            return Err(format!("duplicate portable validation field: {name}"));
        }
        match name {
            "repository" => repository = Some(value.to_owned()),
            "work_kind" => work_kind = Some(value.to_owned()),
            "work_number" => work_number = Some(parse_u64(name, value)?),
            "binding_identity" => binding_identity = Some(value.to_owned()),
            "plan_identity" => plan_identity = Some(value.to_owned()),
            "policy_identity" => policy_identity = Some(value.to_owned()),
            "base_sha" => base_sha = Some(value.to_owned()),
            "worktree_head" => worktree_head = Some(value.to_owned()),
            "worktree_tree" => worktree_tree = Some(value.to_owned()),
            "source_ref" => source_ref = Some(value.to_owned()),
            "source_path" => source_path = Some(value.to_owned()),
            "source_commit" => source_commit = Some(value.to_owned()),
            "source_blob" => source_blob = Some(value.to_owned()),
            "declared_steps" => declared_steps = Some(parse_usize(name, value)?),
            "attempt_id" => attempt_id = Some(value.to_owned()),
            "started_at" => started_at = Some(parse_u64(name, value)?),
            "finished_at" => {
                finished_at = Some(if value == "none" {
                    None
                } else {
                    Some(parse_u64(name, value)?)
                })
            }
            "completed_steps" => completed_steps = Some(parse_usize(name, value)?),
            "phase" => phase = Some(value.to_owned()),
            "status" => status = Some(value.to_owned()),
            other => return Err(format!("unknown portable validation field: {other}")),
        }
    }

    let phase_value = phase.ok_or_else(|| "missing phase".to_owned())?;
    let status_value = status.ok_or_else(|| "missing status".to_owned())?;
    let parsed_phase = match (phase_value.as_str(), status_value.as_str()) {
        ("in_progress", "none") => AttemptPhase::InProgress,
        ("terminal", value) if value != "none" => {
            AttemptPhase::Terminal(TerminalStatus::parse(value)?)
        }
        _ => {
            return Err(format!(
                "invalid portable validation phase/status pair: {phase_value}/{status_value}"
            ));
        }
    };

    let binding = PlanBinding::new(
        repository.ok_or_else(|| "missing repository".to_owned())?,
        work_kind.ok_or_else(|| "missing work_kind".to_owned())?,
        work_number.ok_or_else(|| "missing work_number".to_owned())?,
        binding_identity.ok_or_else(|| "missing binding_identity".to_owned())?,
        plan_identity.ok_or_else(|| "missing plan_identity".to_owned())?,
        policy_identity.ok_or_else(|| "missing policy_identity".to_owned())?,
        base_sha.ok_or_else(|| "missing base_sha".to_owned())?,
        worktree_head.ok_or_else(|| "missing worktree_head".to_owned())?,
        worktree_tree.ok_or_else(|| "missing worktree_tree".to_owned())?,
        source_ref.ok_or_else(|| "missing source_ref".to_owned())?,
        source_path.ok_or_else(|| "missing source_path".to_owned())?,
        source_commit.ok_or_else(|| "missing source_commit".to_owned())?,
        source_blob.ok_or_else(|| "missing source_blob".to_owned())?,
        declared_steps.ok_or_else(|| "missing declared_steps".to_owned())?,
    )?;
    let attempt = PlanAttempt {
        binding,
        attempt_id: attempt_id.ok_or_else(|| "missing attempt_id".to_owned())?,
        started_at: started_at.ok_or_else(|| "missing started_at".to_owned())?,
        finished_at: finished_at.ok_or_else(|| "missing finished_at".to_owned())?,
        completed_steps: completed_steps.ok_or_else(|| "missing completed_steps".to_owned())?,
        phase: parsed_phase,
    };
    validate_attempt(&attempt)?;
    Ok(attempt)
}

fn parse_u64(label: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid portable validation integer {label}: {value}"))
}

fn parse_usize(label: &str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid portable validation integer {label}: {value}"))
}

fn validate_hash(label: &str, value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid portable validation {label}: {value:?}"));
    }
    Ok(())
}

fn validate_hex(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("invalid portable validation {label}"));
    }
    Ok(())
}

fn validate_text(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || value.bytes().any(|byte| matches!(byte, b'\n' | b'\r' | 0))
    {
        return Err(format!("invalid portable validation {label}"));
    }
    Ok(())
}

fn validate_path_token(label: &str, value: &str, max: usize) -> Result<(), String> {
    validate_text(label, value, max)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("invalid portable validation {label} path token"));
    }
    Ok(())
}

fn validate_attempt_id(value: &str) -> Result<(), String> {
    validate_text("attempt id", value, MAX_ATTEMPT_ID_CHARS)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid portable validation attempt id token".to_owned());
    }
    Ok(())
}

fn hex_component(value: &str) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(value.len().saturating_mul(2));
    for byte in value.bytes() {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| {
        format!(
            "portable validation state path has no parent: {}",
            path.display()
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    for sequence in 0..1_024_u16 {
        let temporary = path.with_extension(format!("tmp.{}.{}", std::process::id(), sequence));
        let mut file = match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create temporary portable validation state {}: {error}",
                    temporary.display()
                ));
            }
        };
        let write_result = (|| {
            file.write_all(contents.as_bytes()).map_err(|error| {
                format!(
                    "failed to write portable validation state {}: {error}",
                    temporary.display()
                )
            })?;
            file.sync_all().map_err(|error| {
                format!(
                    "failed to sync portable validation state {}: {error}",
                    temporary.display()
                )
            })?;
            fs::rename(&temporary, path).map_err(|error| {
                format!(
                    "failed to atomically replace portable validation state {}: {error}",
                    path.display()
                )
            })?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return write_result;
    }
    Err("portable validation temporary state sequence exhausted".to_owned())
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
            "orchestrator-validation-state-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn hash(character: char) -> String {
        std::iter::repeat_n(character, 40).collect()
    }

    fn binding(identity_character: char) -> PlanBinding {
        PlanBinding::new(
            "Memorithm/Test".to_owned(),
            "ISSUE".to_owned(),
            47,
            hash(identity_character),
            hash('b'),
            "706f6c696379".to_owned(),
            hash('c'),
            hash('d'),
            hash('e'),
            "agent/roadmap".to_owned(),
            ".agent/ROADMAP.yaml".to_owned(),
            hash('f'),
            hash('a'),
            2,
        )
        .unwrap()
    }

    #[test]
    fn complete_pass_is_durable_and_reusable() {
        let root = temporary_root("pass");
        let store = ValidationPlanStore::new(root.clone());
        let binding = binding('1');
        let mut attempt = store
            .begin(binding.clone(), "attempt-1".to_owned(), 10)
            .unwrap();
        store.update_progress(&mut attempt, 1).unwrap();
        store.update_progress(&mut attempt, 2).unwrap();
        let history = store
            .finish(&mut attempt, TerminalStatus::Passed, 2, 12)
            .unwrap();
        assert!(history.is_file());
        assert!(store.reusable_passed(&binding).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failure_and_timeout_are_never_reusable() {
        for (name, status) in [
            ("failure", TerminalStatus::Failed),
            ("timeout", TerminalStatus::TimedOut),
        ] {
            let root = temporary_root(name);
            let store = ValidationPlanStore::new(root.clone());
            let binding = binding('2');
            let mut attempt = store
                .begin(binding.clone(), format!("attempt-{name}"), 20)
                .unwrap();
            store.update_progress(&mut attempt, 1).unwrap();
            store.finish(&mut attempt, status, 1, 21).unwrap();
            assert!(!store.reusable_passed(&binding).unwrap());
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn interrupted_attempt_is_archived_and_never_reused() {
        let root = temporary_root("interrupted");
        let store = ValidationPlanStore::new(root.clone());
        let binding = binding('3');
        let mut first = store
            .begin(binding.clone(), "attempt-first".to_owned(), 30)
            .unwrap();
        store.update_progress(&mut first, 1).unwrap();
        let _second = store
            .begin(binding.clone(), "attempt-second".to_owned(), 31)
            .unwrap();
        assert!(!store.reusable_passed(&binding).unwrap());
        let history = store.history_root(&binding);
        assert!(fs::read_dir(history).unwrap().count() >= 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn changed_binding_does_not_reuse_another_pass() {
        let root = temporary_root("binding-change");
        let store = ValidationPlanStore::new(root.clone());
        let first_binding = binding('4');
        let mut attempt = store
            .begin(first_binding.clone(), "attempt-binding".to_owned(), 40)
            .unwrap();
        store.update_progress(&mut attempt, 2).unwrap();
        store
            .finish(&mut attempt, TerminalStatus::Passed, 2, 41)
            .unwrap();
        assert!(store.reusable_passed(&first_binding).unwrap());
        assert!(!store.reusable_passed(&binding('5')).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_or_mismatched_passed_index_fails_closed() {
        let root = temporary_root("corrupt");
        let store = ValidationPlanStore::new(root.clone());
        let binding = binding('6');
        let mut attempt = store
            .begin(binding.clone(), "attempt-corrupt".to_owned(), 50)
            .unwrap();
        store.update_progress(&mut attempt, 2).unwrap();
        store
            .finish(&mut attempt, TerminalStatus::Passed, 2, 51)
            .unwrap();
        let passed = store.passed_path(&binding);
        let valid = fs::read_to_string(&passed).unwrap();
        fs::write(
            &passed,
            valid.replace("repository=Memorithm/Test", "repository=Memorithm/Other"),
        )
        .unwrap();
        assert!(store.reusable_passed(&binding).is_err());
        fs::write(&passed, "v99\n").unwrap();
        assert!(store.reusable_passed(&binding).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parser_rejects_duplicate_unknown_status_and_bad_counts() {
        let binding = binding('7');
        let attempt = PlanAttempt {
            binding,
            attempt_id: "attempt-parser".to_owned(),
            started_at: 60,
            finished_at: None,
            completed_steps: 0,
            phase: AttemptPhase::InProgress,
        };
        let valid = serialize_attempt(&attempt);
        assert!(parse_attempt(&(valid.clone() + "phase=in_progress\n")).is_err());
        assert!(parse_attempt(&valid.replace("status=none", "status=unknown")).is_err());
        let terminal_bad = valid
            .replace("finished_at=none", "finished_at=61")
            .replace("completed_steps=0", "completed_steps=1")
            .replace("phase=in_progress", "phase=terminal")
            .replace("status=none", "status=passed");
        assert!(parse_attempt(&terminal_bad).is_err());
    }
}
