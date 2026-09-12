#![forbid(unsafe_code)]

//! Managed OpenCode implementation of the backend-neutral coding contract.
//!
//! This adapter intentionally owns no Git or GitHub capability. The caller must
//! supply the already-managed OpenCode wrapper path used by Orchestrator's
//! sandbox boundary. Repository mutation, diff classification, publication,
//! CI and merge decisions remain parent responsibilities.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::backend::{
    BackendEvent, BackendEventKind, BackendFailure, BackendFailureClass, BackendOutcome,
    BackendResult, BackendTaskContext, CodingBackend,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenCodeBackend {
    executable: PathBuf,
}

impl OpenCodeBackend {
    /// Construct an adapter bound to an absolute managed wrapper path.
    ///
    /// # Errors
    ///
    /// Returns an infrastructure-class failure when the path is not absolute.
    pub fn new(executable: impl Into<PathBuf>) -> Result<Self, BackendFailure> {
        let executable = executable.into();
        if !executable.is_absolute() {
            return Err(infrastructure_failure(
                "OpenCode backend executable must be an absolute managed wrapper path",
            ));
        }
        Ok(Self { executable })
    }

    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }
}

impl CodingBackend for OpenCodeBackend {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn execute(&self, task: &BackendTaskContext) -> Result<BackendResult, BackendFailure> {
        task.validate()
            .map_err(|error| infrastructure_failure(&format!("invalid backend task: {error}")))?;

        let mut child = Command::new(&self.executable)
            .current_dir(&task.workspace)
            .args(["run", "--auto", "--model"])
            .arg(&task.model)
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| infrastructure_failure(&format!("failed to start OpenCode: {error}")))?;

        let Some(mut stdin) = child.stdin.take() else {
            return Err(infrastructure_failure("OpenCode stdin pipe unavailable"));
        };
        stdin
            .write_all(task.prompt.as_bytes())
            .map_err(|error| infrastructure_failure(&format!("failed to send prompt: {error}")))?;
        drop(stdin);

        let status = child
            .wait()
            .map_err(|error| infrastructure_failure(&format!("failed to wait for OpenCode: {error}")))?;
        if !status.success() {
            return Err(agent_failure(&format!(
                "OpenCode exited unsuccessfully: {status}"
            )));
        }

        // The backend cannot authoritatively classify the repository diff.
        // `NoChanges` here is only untrusted backend evidence; the parent must
        // inspect the worktree before deciding whether changes were applied.
        BackendResult::new(
            BackendOutcome::NoChanges,
            vec![
                BackendEvent::new(1, BackendEventKind::Started, "managed OpenCode started")
                    .map_err(|error| infrastructure_failure(&error.to_string()))?,
                BackendEvent::new(2, BackendEventKind::Completed, "managed OpenCode completed")
                    .map_err(|error| infrastructure_failure(&error.to_string()))?,
            ],
        )
        .map_err(|error| infrastructure_failure(&error.to_string()))
    }
}

fn infrastructure_failure(message: &str) -> BackendFailure {
    BackendFailure::new(BackendFailureClass::Infrastructure, message)
        .expect("bounded static/backend error message")
}

fn agent_failure(message: &str) -> BackendFailure {
    BackendFailure::new(BackendFailureClass::Agent, message)
        .expect("bounded process status message")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_requires_absolute_managed_executable() {
        let error = OpenCodeBackend::new("opencode").expect_err("relative path must fail");
        assert_eq!(error.class, BackendFailureClass::Infrastructure);
    }

    #[test]
    fn adapter_preserves_configured_managed_path() {
        let backend = OpenCodeBackend::new("/managed/bin/opencode").expect("absolute path");
        assert_eq!(backend.executable(), Path::new("/managed/bin/opencode"));
        assert_eq!(backend.name(), "opencode");
    }
}
