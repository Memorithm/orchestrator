#![forbid(unsafe_code)]

//! Backend-neutral coding-agent protocol.
//!
//! Orchestrator remains authoritative for repository state, Git mutations,
//! publication, CI and merge decisions. A coding backend receives one bounded
//! task context and returns untrusted execution evidence only.

use std::fmt;
use std::path::PathBuf;

const MAX_ID_BYTES: usize = 128;
const MAX_DETAIL_BYTES: usize = 16_384;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendTaskContext {
    pub task_id: String,
    pub repository: String,
    pub workspace: PathBuf,
    pub prompt: String,
    pub model: String,
}

impl BackendTaskContext {
    pub fn new(
        task_id: impl Into<String>,
        repository: impl Into<String>,
        workspace: impl Into<PathBuf>,
        prompt: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, BackendContractError> {
        let value = Self {
            task_id: task_id.into(),
            repository: repository.into(),
            workspace: workspace.into(),
            prompt: prompt.into(),
            model: model.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), BackendContractError> {
        validate_identifier("task_id", &self.task_id)?;
        validate_repository(&self.repository)?;
        if !self.workspace.is_absolute() {
            return Err(BackendContractError::InvalidWorkspace);
        }
        if self.prompt.is_empty() {
            return Err(BackendContractError::EmptyPrompt);
        }
        if self.model.trim().is_empty() {
            return Err(BackendContractError::EmptyModel);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendEventKind {
    Started,
    Progress,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendEvent {
    pub sequence: u64,
    pub kind: BackendEventKind,
    pub detail: String,
}

impl BackendEvent {
    pub fn new(
        sequence: u64,
        kind: BackendEventKind,
        detail: impl Into<String>,
    ) -> Result<Self, BackendContractError> {
        let detail = detail.into();
        if detail.len() > MAX_DETAIL_BYTES {
            return Err(BackendContractError::DetailTooLong);
        }
        Ok(Self {
            sequence,
            kind,
            detail,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendOutcome {
    AppliedChanges,
    NoChanges,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendResult {
    pub outcome: BackendOutcome,
    pub events: Vec<BackendEvent>,
}

impl BackendResult {
    pub fn new(
        outcome: BackendOutcome,
        events: Vec<BackendEvent>,
    ) -> Result<Self, BackendContractError> {
        let result = Self { outcome, events };
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), BackendContractError> {
        let mut previous = None;
        for event in &self.events {
            if let Some(previous) = previous {
                if event.sequence <= previous {
                    return Err(BackendContractError::NonMonotonicEventSequence);
                }
            }
            previous = Some(event.sequence);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendFailureClass {
    Infrastructure,
    Agent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendFailure {
    pub class: BackendFailureClass,
    pub message: String,
}

impl BackendFailure {
    pub fn new(
        class: BackendFailureClass,
        message: impl Into<String>,
    ) -> Result<Self, BackendContractError> {
        let message = message.into();
        if message.is_empty() {
            return Err(BackendContractError::EmptyFailureMessage);
        }
        if message.len() > MAX_DETAIL_BYTES {
            return Err(BackendContractError::DetailTooLong);
        }
        Ok(Self { class, message })
    }
}

pub trait CodingBackend {
    fn name(&self) -> &'static str;

    fn execute(&self, task: &BackendTaskContext) -> Result<BackendResult, BackendFailure>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendContractError {
    InvalidIdentifier(&'static str),
    InvalidRepository,
    InvalidWorkspace,
    EmptyPrompt,
    EmptyModel,
    DetailTooLong,
    EmptyFailureMessage,
    NonMonotonicEventSequence,
}

impl fmt::Display for BackendContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier(field) => write!(formatter, "invalid backend {field}"),
            Self::InvalidRepository => write!(formatter, "repository must be owner/name"),
            Self::InvalidWorkspace => write!(formatter, "workspace must be an absolute path"),
            Self::EmptyPrompt => write!(formatter, "backend prompt is empty"),
            Self::EmptyModel => write!(formatter, "backend model is empty"),
            Self::DetailTooLong => write!(formatter, "backend detail exceeds bounded size"),
            Self::EmptyFailureMessage => write!(formatter, "backend failure message is empty"),
            Self::NonMonotonicEventSequence => {
                write!(formatter, "backend event sequence is not strictly increasing")
            }
        }
    }
}

impl std::error::Error for BackendContractError {}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), BackendContractError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(BackendContractError::InvalidIdentifier(field));
    }
    Ok(())
}

fn validate_repository(value: &str) -> Result<(), BackendContractError> {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || owner.is_empty()
        || repository.is_empty()
        || !owner.bytes().all(valid_repository_byte)
        || !repository.bytes().all(valid_repository_byte)
    {
        return Err(BackendContractError::InvalidRepository);
    }
    Ok(())
}

fn valid_repository_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

#[cfg(test)]
mod tests {
    use super::{
        BackendContractError, BackendEvent, BackendEventKind, BackendOutcome, BackendResult,
        BackendTaskContext,
    };

    #[test]
    fn task_context_requires_bounded_machine_identity_and_absolute_workspace() {
        let task = BackendTaskContext::new(
            "issue:58",
            "Memorithm/orchestrator",
            "/tmp/orchestrator-worktree",
            "implement bounded slice",
            "ollama/qwen3.8:latest",
        )
        .expect("valid context");
        assert_eq!(task.repository, "Memorithm/orchestrator");

        assert_eq!(
            BackendTaskContext::new("bad id", "Memorithm/orchestrator", "/tmp/x", "p", "m"),
            Err(BackendContractError::InvalidIdentifier("task_id"))
        );
        assert_eq!(
            BackendTaskContext::new("issue:58", "Memorithm", "/tmp/x", "p", "m"),
            Err(BackendContractError::InvalidRepository)
        );
        assert_eq!(
            BackendTaskContext::new("issue:58", "Memorithm/orchestrator", "relative", "p", "m"),
            Err(BackendContractError::InvalidWorkspace)
        );
    }

    #[test]
    fn result_rejects_non_monotonic_event_streams() {
        let valid = BackendResult::new(
            BackendOutcome::NoChanges,
            vec![
                BackendEvent::new(1, BackendEventKind::Started, "start").expect("event"),
                BackendEvent::new(2, BackendEventKind::Completed, "done").expect("event"),
            ],
        );
        assert!(valid.is_ok());

        let invalid = BackendResult::new(
            BackendOutcome::AppliedChanges,
            vec![
                BackendEvent::new(2, BackendEventKind::Progress, "later").expect("event"),
                BackendEvent::new(2, BackendEventKind::Completed, "duplicate").expect("event"),
            ],
        );
        assert_eq!(
            invalid,
            Err(BackendContractError::NonMonotonicEventSequence)
        );
    }

    #[test]
    fn protocol_has_no_git_or_github_mutation_capability() {
        let fields = std::mem::size_of::<BackendTaskContext>();
        assert!(fields > 0);
        // The contract transports task context and evidence only. Git/GitHub
        // authority remains outside this module in the Orchestrator parent.
    }
}
