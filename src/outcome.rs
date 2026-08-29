#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    AgentBackend,
    AgentNoProgress,
    Workspace,
    Validation,
    Policy,
    GitRace,
    GitHub,
    ExternalDependency,
}

impl FailureClass {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AgentBackend => "agent_backend",
            Self::AgentNoProgress => "agent_no_progress",
            Self::Workspace => "workspace",
            Self::Validation => "validation",
            Self::Policy => "policy",
            Self::GitRace => "git_race",
            Self::GitHub => "github",
            Self::ExternalDependency => "external_dependency",
        }
    }

    pub(crate) const fn retry_weight(self) -> u32 {
        match self {
            Self::AgentBackend | Self::ExternalDependency => 1,
            Self::AgentNoProgress | Self::GitRace | Self::GitHub => 2,
            Self::Workspace | Self::Validation => 3,
            Self::Policy => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProgressOutcome {
    Published {
        commit: String,
        pull_request: Option<u64>,
    },
    Merged {
        pull_request: u64,
    },
    NoChange,
    Deferred,
}

impl ProgressOutcome {
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Published { .. } => "published",
            Self::Merged { .. } => "merged",
            Self::NoChange => "no_change",
            Self::Deferred => "deferred",
        }
    }

    pub(crate) const fn made_progress(&self) -> bool {
        matches!(self, Self::Published { .. } | Self::Merged { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionFailure {
    pub(crate) class: FailureClass,
    pub(crate) message: String,
}

impl ExecutionFailure {
    pub(crate) fn new(class: FailureClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_failures_have_stronger_retry_weight_than_transient_backend_errors() {
        assert!(FailureClass::Policy.retry_weight() > FailureClass::AgentBackend.retry_weight());
        assert!(
            FailureClass::Validation.retry_weight()
                > FailureClass::ExternalDependency.retry_weight()
        );
    }

    #[test]
    fn only_published_or_merged_outcomes_count_as_progress() {
        assert!(ProgressOutcome::Published {
            commit: "abc".to_owned(),
            pull_request: Some(1),
        }
        .made_progress());
        assert!(ProgressOutcome::Merged { pull_request: 1 }.made_progress());
        assert!(!ProgressOutcome::NoChange.made_progress());
        assert!(!ProgressOutcome::Deferred.made_progress());
    }
}
