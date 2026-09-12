use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_POLICY_DOCUMENTS: usize = 12;
const MAX_DOCUMENT_BYTES: u64 = 128 * 1024;
const MAX_TOTAL_BYTES: usize = 512 * 1024;
const MAX_REF_CHARS: usize = 240;
const MAX_PATH_CHARS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PolicyPointer {
    ref_name: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicyDocument {
    ref_name: String,
    path: String,
    commit_sha: String,
    blob_sha: String,
    content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicySnapshot {
    repository: String,
    base_branch: String,
    base_sha: String,
    bootstrap: Option<PolicyDocument>,
    documents: Vec<PolicyDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskEligibility {
    Allowed,
    Deferred(PolicyDenial),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicyDenial {
    item_id: String,
    field: &'static str,
    value: String,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
}

impl PolicyDenial {
    pub(crate) fn reason(&self, repository: &str, snapshot: &PolicySnapshot) -> String {
        format!(
            "repository={repository} policy item={} denies autonomous initiation via {}={} source=origin/{}:{} commit={} blob={} policy_identity={}",
            self.item_id,
            self.field,
            self.value,
            self.source_ref,
            self.source_path,
            self.source_commit,
            self.source_blob,
            snapshot.identity_token()
        )
    }

    pub(crate) fn merge_reason(&self, repository: &str, snapshot: &PolicySnapshot) -> String {
        format!(
            "repository={repository} policy item={} denies autonomous merge via {}={} source=origin/{}:{} commit={} blob={} policy_identity={}",
            self.item_id,
            self.field,
            self.value,
            self.source_ref,
            self.source_path,
            self.source_commit,
            self.source_blob,
            snapshot.identity_token()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoadmapTaskRule {
    id: String,
    status: Option<String>,
    agent_policy: Option<String>,
    execution_policy: Option<String>,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AutonomousActionCategory {
    FinancialExecution,
    CustodyMutation,
    CredentialMutation,
    ExternalSideEffect,
}

impl AutonomousActionCategory {
    const fn as_str(self) -> &'static str {
        match self {
            Self::FinancialExecution => "financial_execution",
            Self::CustodyMutation => "custody_mutation",
            Self::CredentialMutation => "credential_mutation",
            Self::ExternalSideEffect => "external_side_effect",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "financial_execution" => Ok(Self::FinancialExecution),
            "custody_mutation" => Ok(Self::CustodyMutation),
            "credential_mutation" => Ok(Self::CredentialMutation),
            "external_side_effect" => Ok(Self::ExternalSideEffect),
            other => Err(format!("unknown autonomous action category: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutonomousActionDecision {
    Allow,
    Deny,
}

impl AutonomousActionDecision {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            other => Err(format!("invalid autonomous action decision: {other}")),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutonomousActionRule {
    category: AutonomousActionCategory,
    decision: AutonomousActionDecision,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MergeEligibility {
    Inherit,
    Allowed,
    Deferred(PolicyDenial),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeDecision {
    Allow,
    Deny,
}

impl MergeDecision {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            other => Err(format!("invalid autonomous merge decision: {other}")),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MergePolicyRule {
    decision: MergeDecision,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MergeEvidenceEligibility {
    Inherit,
    PortableCi,
    HardwareRequired(HardwareEvidenceRequirement),
    Deferred(PolicyDenial),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HardwareEvidenceRequirement {
    requirement_id: String,
}

impl HardwareEvidenceRequirement {
    pub(crate) fn requirement_id(&self) -> &str {
        &self.requirement_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeEvidenceClass {
    PortableCi,
    HardwareRequired,
    HumanRequired,
}

impl MergeEvidenceClass {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "portable_ci" => Ok(Self::PortableCi),
            "hardware_required" => Ok(Self::HardwareRequired),
            "human_required" => Ok(Self::HumanRequired),
            other => Err(format!("unknown merge evidence class: {other}")),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::PortableCi => "portable_ci",
            Self::HardwareRequired => "hardware_required",
            Self::HumanRequired => "human_required",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MergeEvidenceRule {
    schema_version: u8,
    required: MergeEvidenceClass,
    requirement_id: Option<String>,
    source_ref: String,
    source_path: String,
    source_commit: String,
    source_blob: String,
}

const MAX_VALIDATION_STEPS: usize = 24;
const MAX_VALIDATION_ARGV: usize = 32;
const MAX_VALIDATION_ARG_CHARS: usize = 512;
const MAX_VALIDATION_CWD_CHARS: usize = 256;
const MAX_VALIDATION_TIMEOUT_SECS: u64 = 3_600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PortableValidationStep {
    pub(crate) id: String,
    pub(crate) argv: Vec<String>,
    pub(crate) cwd: String,
    pub(crate) timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PortableValidationPlan {
    pub(crate) steps: Vec<PortableValidationStep>,
    pub(crate) source_ref: String,
    pub(crate) source_path: String,
    pub(crate) source_commit: String,
    pub(crate) source_blob: String,
}
