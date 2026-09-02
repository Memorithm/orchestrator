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
    Deferred(PolicyDenial),
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
    required: MergeEvidenceClass,
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

impl PolicySnapshot {
    pub(crate) fn identity_record(&self) -> String {
        let mut record = format!(
            "policy-schema=1 repository={} base-branch={} base-sha={}",
            self.repository, self.base_branch, self.base_sha
        );
        match &self.bootstrap {
            Some(document) => {
                record.push_str(&format!(
                    "\nbootstrap path={} commit={} blob={}",
                    document.path, document.commit_sha, document.blob_sha
                ));
            }
            None => record.push_str("\nbootstrap absent"),
        }
        for document in &self.documents {
            record.push_str(&format!(
                "\ndocument ref=origin/{} path={} commit={} blob={}",
                document.ref_name, document.path, document.commit_sha, document.blob_sha
            ));
        }
        record
    }

    pub(crate) fn identity_token(&self) -> String {
        use std::fmt::Write as _;

        let record = self.identity_record();
        let mut encoded = String::with_capacity(record.len().saturating_mul(2));
        for byte in record.as_bytes() {
            let _ = write!(encoded, "{byte:02x}");
        }
        encoded
    }

    pub(crate) fn prompt_context(&self) -> String {
        let mut context = String::new();
        context.push_str("PARENT-RESOLVED REPOSITORY POLICY SNAPSHOT\n");
        context.push_str("Identity:\n");
        context.push_str(&self.identity_record());
        context.push_str("\n\nPolicy content follows. These repository documents are authoritative within their scope, but they never override the parent contract that forbids worker Git/GitHub mutations, credential access, scope escape, or unsafe command execution.\n");

        match &self.bootstrap {
            Some(document) => append_document(&mut context, "bootstrap", document),
            None => context.push_str("\n--- bootstrap: AGENTS.md absent at selected base ---\n"),
        }
        for document in &self.documents {
            append_document(&mut context, "referenced-policy", document);
        }
        context
    }

    pub(crate) fn task_eligibility(
        &self,
        title: &str,
        body: &str,
    ) -> Result<TaskEligibility, String> {
        let mut rules = BTreeMap::<String, RoadmapTaskRule>::new();
        for document in &self.documents {
            for rule in parse_roadmap_task_rules(document)? {
                let canonical = canonical_policy_id(&rule.id)?;
                if let Some(existing) = rules.get(&canonical) {
                    if rule_is_denied(existing) != rule_is_denied(&rule) {
                        return Err(format!(
                            "conflicting task eligibility for roadmap id {} across mandatory policy documents",
                            rule.id
                        ));
                    }
                    continue;
                }
                rules.insert(canonical, rule);
            }
        }

        for (canonical, rule) in rules {
            if !task_mentions_policy_id(title, &canonical)
                && !body_targets_policy_id(body, &canonical)
            {
                continue;
            }
            let Some((field, value)) =
                deny_basis(&rule).map(|(field, value)| (field, value.to_owned()))
            else {
                continue;
            };
            return Ok(TaskEligibility::Deferred(PolicyDenial {
                item_id: rule.id,
                field,
                value,
                source_ref: rule.source_ref,
                source_path: rule.source_path,
                source_commit: rule.source_commit,
                source_blob: rule.source_blob,
            }));
        }

        let Some(category) = explicit_task_action_category(body)? else {
            return Ok(TaskEligibility::Allowed);
        };
        let mut action_rules = BTreeMap::<AutonomousActionCategory, AutonomousActionRule>::new();
        for document in &self.documents {
            for rule in parse_autonomous_action_rules(document)? {
                if action_rules.insert(rule.category, rule.clone()).is_some() {
                    return Err(format!(
                        "duplicate repository-global autonomous action policy for category {} across mandatory policy documents",
                        rule.category.as_str()
                    ));
                }
            }
        }
        let Some(rule) = action_rules.get(&category) else {
            return Ok(TaskEligibility::Allowed);
        };
        if rule.decision == AutonomousActionDecision::Allow {
            return Ok(TaskEligibility::Allowed);
        }
        Ok(TaskEligibility::Deferred(PolicyDenial {
            item_id: format!("global:{}", category.as_str()),
            field: "autonomous_action_policy",
            value: rule.decision.as_str().to_owned(),
            source_ref: rule.source_ref.clone(),
            source_path: rule.source_path.clone(),
            source_commit: rule.source_commit.clone(),
            source_blob: rule.source_blob.clone(),
        }))
    }

    pub(crate) fn merge_eligibility(&self) -> Result<MergeEligibility, String> {
        let mut selected: Option<MergePolicyRule> = None;
        for document in &self.documents {
            let Some(rule) = parse_merge_policy(document)? else {
                continue;
            };
            if selected.replace(rule).is_some() {
                return Err(
                    "duplicate autonomous merge policy across mandatory policy documents"
                        .to_owned(),
                );
            }
        }
        let Some(rule) = selected else {
            return Ok(MergeEligibility::Inherit);
        };
        if rule.decision == MergeDecision::Allow {
            return Ok(MergeEligibility::Allowed);
        }
        Ok(MergeEligibility::Deferred(PolicyDenial {
            item_id: "global:auto_merge".to_owned(),
            field: "autonomous_merge_policy",
            value: rule.decision.as_str().to_owned(),
            source_ref: rule.source_ref,
            source_path: rule.source_path,
            source_commit: rule.source_commit,
            source_blob: rule.source_blob,
        }))
    }

    pub(crate) fn merge_evidence_eligibility(&self) -> Result<MergeEvidenceEligibility, String> {
        let mut selected: Option<MergeEvidenceRule> = None;
        for document in &self.documents {
            let Some(rule) = parse_merge_evidence_policy(document)? else {
                continue;
            };
            if selected.replace(rule).is_some() {
                return Err(
                    "duplicate merge evidence policy across mandatory policy documents".to_owned(),
                );
            }
        }
        let Some(rule) = selected else {
            return Ok(MergeEvidenceEligibility::Inherit);
        };
        if rule.required == MergeEvidenceClass::PortableCi {
            return Ok(MergeEvidenceEligibility::PortableCi);
        }
        Ok(MergeEvidenceEligibility::Deferred(PolicyDenial {
            item_id: format!("evidence:{}", rule.required.as_str()),
            field: "merge_evidence_policy",
            value: rule.required.as_str().to_owned(),
            source_ref: rule.source_ref,
            source_path: rule.source_path,
            source_commit: rule.source_commit,
            source_blob: rule.source_blob,
        }))
    }

    pub(crate) fn portable_validation_plan(
        &self,
    ) -> Result<Option<PortableValidationPlan>, String> {
        let mut selected = None;
        for document in &self.documents {
            let Some(plan) = parse_validation_plan(document)? else {
                continue;
            };
            if selected.replace(plan).is_some() {
                return Err(
                    "duplicate validation_plan across mandatory policy documents".to_owned(),
                );
            }
        }
        Ok(selected)
    }

    pub(crate) fn base_branch(&self) -> &str {
        &self.base_branch
    }

    pub(crate) fn base_sha(&self) -> &str {
        &self.base_sha
    }
}

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

fn parse_validation_plan(
    document: &PolicyDocument,
) -> Result<Option<PortableValidationPlan>, String> {
    #[derive(Default)]
    struct PendingStep {
        id: Option<String>,
        argv: Option<Vec<String>>,
        cwd: Option<String>,
        timeout_seconds: Option<u64>,
    }

    fn finish_step(
        pending: &mut Option<PendingStep>,
        seen_ids: &mut BTreeSet<String>,
        steps: &mut Vec<PortableValidationStep>,
        document: &PolicyDocument,
    ) -> Result<(), String> {
        let Some(step) = pending.take() else {
            return Ok(());
        };
        if steps.len() >= MAX_VALIDATION_STEPS {
            return Err(format!(
                "validation plan exceeds {MAX_VALIDATION_STEPS} steps in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let id = step
            .id
            .ok_or_else(|| "validation step is missing id".to_owned())?;
        let argv = step
            .argv
            .ok_or_else(|| format!("validation step {id} is missing argv"))?;
        let cwd = step.cwd.unwrap_or_else(|| ".".to_owned());
        let timeout_seconds = step.timeout_seconds.unwrap_or(300);
        if !seen_ids.insert(id.clone()) {
            return Err(format!("duplicate validation step id: {id}"));
        }
        steps.push(PortableValidationStep {
            id,
            argv,
            cwd,
            timeout_seconds,
        });
        Ok(())
    }

    let mut in_plan = false;
    let mut in_steps = false;
    let mut saw_section = false;
    let mut schema_version = None;
    let mut class = None;
    let mut pending_step: Option<PendingStep> = None;
    let mut seen_ids = BTreeSet::new();
    let mut steps = Vec::new();

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_plan {
            if raw_line == "validation_plan:" {
                if saw_section {
                    return Err(format!(
                        "duplicate validation_plan section in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
                in_plan = true;
                saw_section = true;
            }
            continue;
        }
        if raw_line.starts_with('\t') {
            return Err(format!(
                "tab indentation is not allowed in validation_plan origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            finish_step(&mut pending_step, &mut seen_ids, &mut steps, document)?;
            in_plan = false;
            in_steps = false;
            if raw_line == "validation_plan:" {
                return Err(format!(
                    "duplicate validation_plan section in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            continue;
        }
        if indent == 2 {
            finish_step(&mut pending_step, &mut seen_ids, &mut steps, document)?;
            in_steps = false;
            let (key, raw_value) = trimmed.split_once(':').ok_or_else(|| {
                format!(
                    "malformed validation_plan field in origin/{}:{}",
                    document.ref_name, document.path
                )
            })?;
            match key {
                "schema_version" => {
                    let value = parse_policy_scalar(raw_value.trim(), key)?;
                    if schema_version.replace(value.clone()).is_some() {
                        return Err("duplicate validation_plan schema_version".to_owned());
                    }
                    if value != "1" {
                        return Err(format!(
                            "unsupported validation_plan schema_version {value}"
                        ));
                    }
                }
                "class" => {
                    let value = parse_policy_scalar(raw_value.trim(), key)?;
                    if class.replace(value.clone()).is_some() {
                        return Err("duplicate validation_plan class".to_owned());
                    }
                    if value != "portable" {
                        return Err(format!("unsupported validation_plan class: {value}"));
                    }
                }
                "steps" if raw_value.trim().is_empty() => in_steps = true,
                other => {
                    return Err(format!("unknown validation_plan field: {other}"));
                }
            }
            continue;
        }
        if indent == 4 && trimmed.starts_with("- ") {
            if !in_steps {
                return Err("validation step declared outside steps".to_owned());
            }
            finish_step(&mut pending_step, &mut seen_ids, &mut steps, document)?;
            let rest = trimmed.trim_start_matches("- ");
            let (key, raw_value) = rest
                .split_once(':')
                .ok_or_else(|| "malformed validation step declaration".to_owned())?;
            if key != "id" {
                return Err("validation step must begin with id".to_owned());
            }
            let id = parse_policy_scalar(raw_value.trim(), "validation step id")?;
            if id.is_empty()
                || id.len() > 64
                || !id.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
                })
            {
                return Err(format!("invalid validation step id: {id:?}"));
            }
            pending_step = Some(PendingStep {
                id: Some(id),
                ..PendingStep::default()
            });
            continue;
        }
        if indent == 6 && in_steps {
            let step = pending_step
                .as_mut()
                .ok_or_else(|| "validation step field appears before id".to_owned())?;
            let (key, raw_value) = trimmed
                .split_once(':')
                .ok_or_else(|| "malformed validation step field".to_owned())?;
            match key {
                "argv" => {
                    if step.argv.is_some() {
                        return Err("duplicate validation step argv".to_owned());
                    }
                    step.argv = Some(parse_validation_argv(raw_value.trim())?);
                }
                "cwd" => {
                    if step.cwd.is_some() {
                        return Err("duplicate validation step cwd".to_owned());
                    }
                    let value = parse_policy_scalar(raw_value.trim(), key)?;
                    step.cwd = Some(validate_validation_cwd(&value)?);
                }
                "timeout_seconds" => {
                    if step.timeout_seconds.is_some() {
                        return Err("duplicate validation step timeout_seconds".to_owned());
                    }
                    let value = parse_policy_scalar(raw_value.trim(), key)?;
                    let timeout = value
                        .parse::<u64>()
                        .map_err(|_| format!("invalid validation timeout_seconds: {value}"))?;
                    if timeout == 0 || timeout > MAX_VALIDATION_TIMEOUT_SECS {
                        return Err(format!(
                            "validation timeout_seconds must be within 1..={MAX_VALIDATION_TIMEOUT_SECS}"
                        ));
                    }
                    step.timeout_seconds = Some(timeout);
                }
                other => return Err(format!("unknown validation step field: {other}")),
            }
            continue;
        }
        return Err(format!(
            "unsupported validation_plan indentation or structure in origin/{}:{}",
            document.ref_name, document.path
        ));
    }

    if !saw_section {
        return Ok(None);
    }
    finish_step(&mut pending_step, &mut seen_ids, &mut steps, document)?;
    if schema_version.as_deref() != Some("1") {
        return Err("validation_plan requires schema_version: 1".to_owned());
    }
    if class.as_deref() != Some("portable") {
        return Err("validation_plan requires class: portable".to_owned());
    }
    if steps.is_empty() {
        return Err("validation_plan requires at least one step".to_owned());
    }
    Ok(Some(PortableValidationPlan {
        steps,
        source_ref: document.ref_name.clone(),
        source_path: document.path.clone(),
        source_commit: document.commit_sha.clone(),
        source_blob: document.blob_sha.clone(),
    }))
}

fn parse_merge_evidence_policy(
    document: &PolicyDocument,
) -> Result<Option<MergeEvidenceRule>, String> {
    let mut in_policy = false;
    let mut saw_section = false;
    let mut schema_version = None;
    let mut required = None;

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_policy {
            if raw_line == "merge_evidence_policy:" {
                if saw_section {
                    return Err(format!(
                        "duplicate merge_evidence_policy section in origin/{}:{}",
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
                "tab indentation is not allowed in merge evidence policy origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            in_policy = false;
            if raw_line == "merge_evidence_policy:" {
                return Err(format!(
                    "duplicate merge_evidence_policy section in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            continue;
        }
        if indent != 2 {
            return Err(format!(
                "merge_evidence_policy only accepts scalar fields at indentation 2 in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let (key, raw_value) = trimmed.split_once(':').ok_or_else(|| {
            format!(
                "malformed merge evidence policy field in origin/{}:{}",
                document.ref_name, document.path
            )
        })?;
        let value = parse_policy_scalar(raw_value.trim(), key)?;
        match key {
            "schema_version" => {
                if schema_version.replace(value.clone()).is_some() {
                    return Err(format!(
                        "duplicate merge evidence schema_version in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
                if value != "1" {
                    return Err(format!(
                        "unsupported merge evidence policy schema_version {value} in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            "required" => {
                let parsed = MergeEvidenceClass::parse(&value)?;
                if required.replace(parsed).is_some() {
                    return Err(format!(
                        "duplicate merge evidence requirement in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            other => {
                return Err(format!(
                    "unknown merge evidence policy field {other} in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
        }
    }

    if !saw_section {
        return Ok(None);
    }
    if schema_version.as_deref() != Some("1") {
        return Err(format!(
            "merge_evidence_policy requires schema_version: 1 in origin/{}:{}",
            document.ref_name, document.path
        ));
    }
    let required = required.ok_or_else(|| {
        format!(
            "merge_evidence_policy requires required in origin/{}:{}",
            document.ref_name, document.path
        )
    })?;
    Ok(Some(MergeEvidenceRule {
        required,
        source_ref: document.ref_name.clone(),
        source_path: document.path.clone(),
        source_commit: document.commit_sha.clone(),
        source_blob: document.blob_sha.clone(),
    }))
}

fn parse_merge_policy(document: &PolicyDocument) -> Result<Option<MergePolicyRule>, String> {
    let mut in_policy = false;
    let mut saw_section = false;
    let mut schema_version = None;
    let mut decision = None;

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_policy {
            if raw_line == "autonomous_merge_policy:" {
                if saw_section {
                    return Err(format!(
                        "duplicate autonomous_merge_policy section in origin/{}:{}",
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
                "tab indentation is not allowed in autonomous merge policy origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            in_policy = false;
            if raw_line == "autonomous_merge_policy:" {
                return Err(format!(
                    "duplicate autonomous_merge_policy section in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
            continue;
        }
        if indent != 2 {
            return Err(format!(
                "autonomous_merge_policy only accepts scalar fields at indentation 2 in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let (key, raw_value) = trimmed.split_once(':').ok_or_else(|| {
            format!(
                "malformed autonomous merge policy field in origin/{}:{}",
                document.ref_name, document.path
            )
        })?;
        let value = parse_policy_scalar(raw_value.trim(), key)?;
        match key {
            "schema_version" => {
                if schema_version.replace(value.clone()).is_some() {
                    return Err(format!(
                        "duplicate autonomous merge schema_version in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
                if value != "1" {
                    return Err(format!(
                        "unsupported autonomous merge policy schema_version {value} in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            "decision" => {
                let parsed = MergeDecision::parse(&value)?;
                if decision.replace(parsed).is_some() {
                    return Err(format!(
                        "duplicate autonomous merge decision in origin/{}:{}",
                        document.ref_name, document.path
                    ));
                }
            }
            other => {
                return Err(format!(
                    "unknown autonomous merge policy field {other} in origin/{}:{}",
                    document.ref_name, document.path
                ));
            }
        }
    }

    if !saw_section {
        return Ok(None);
    }
    if schema_version.as_deref() != Some("1") {
        return Err(format!(
            "autonomous_merge_policy requires schema_version: 1 in origin/{}:{}",
            document.ref_name, document.path
        ));
    }
    let decision = decision.ok_or_else(|| {
        format!(
            "autonomous_merge_policy requires decision in origin/{}:{}",
            document.ref_name, document.path
        )
    })?;
    Ok(Some(MergePolicyRule {
        decision,
        source_ref: document.ref_name.clone(),
        source_path: document.path.clone(),
        source_commit: document.commit_sha.clone(),
        source_blob: document.blob_sha.clone(),
    }))
}

fn parse_roadmap_task_rules(document: &PolicyDocument) -> Result<Vec<RoadmapTaskRule>, String> {
    let mut in_roadmap = false;
    let mut current: Option<RoadmapTaskRule> = None;
    let mut rules = Vec::new();
    let mut ids = BTreeSet::new();

    for raw_line in document.content.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_roadmap {
            if raw_line == "roadmap:" {
                in_roadmap = true;
            }
            continue;
        }

        if raw_line.starts_with('\t') {
            return Err(format!(
                "tab indentation is not allowed in recognized roadmap policy origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 {
            break;
        }

        if indent == 2 && trimmed.starts_with("- id:") {
            if let Some(rule) = current.take() {
                finish_roadmap_rule(rule, &mut ids, &mut rules)?;
            }
            let id = parse_policy_scalar(trimmed[5..].trim(), "id")?;
            canonical_policy_id(&id)?;
            current = Some(RoadmapTaskRule {
                id,
                status: None,
                agent_policy: None,
                execution_policy: None,
                source_ref: document.ref_name.clone(),
                source_path: document.path.clone(),
                source_commit: document.commit_sha.clone(),
                source_blob: document.blob_sha.clone(),
            });
            continue;
        }
        if indent == 2 && trimmed.starts_with("- ") {
            return Err(format!(
                "recognized roadmap contains a top-level list item without an id in origin/{}:{}",
                document.ref_name, document.path
            ));
        }
        if indent != 4 {
            continue;
        }

        let Some((key, raw_value)) = trimmed.split_once(':') else {
            continue;
        };
        if !matches!(key, "status" | "agent_policy" | "execution_policy") {
            continue;
        }
        let rule = current.as_mut().ok_or_else(|| {
            format!(
                "roadmap field {key} appears before an id in origin/{}:{}",
                document.ref_name, document.path
            )
        })?;
        let value = parse_policy_scalar(raw_value.trim(), key)?;
        let slot = match key {
            "status" => &mut rule.status,
            "agent_policy" => &mut rule.agent_policy,
            "execution_policy" => &mut rule.execution_policy,
            _ => unreachable!(),
        };
        if slot.replace(value).is_some() {
            return Err(format!(
                "duplicate roadmap field {key} for id {} in origin/{}:{}",
                rule.id, document.ref_name, document.path
            ));
        }
    }

    if let Some(rule) = current {
        finish_roadmap_rule(rule, &mut ids, &mut rules)?;
    }
    Ok(rules)
}

fn finish_roadmap_rule(
    rule: RoadmapTaskRule,
    ids: &mut BTreeSet<String>,
    rules: &mut Vec<RoadmapTaskRule>,
) -> Result<(), String> {
    let canonical = canonical_policy_id(&rule.id)?;
    if !ids.insert(canonical) {
        return Err(format!(
            "duplicate roadmap id {} in one mandatory policy document",
            rule.id
        ));
    }
    rules.push(rule);
    Ok(())
}

fn parse_policy_scalar(value: &str, field: &str) -> Result<String, String> {
    if value.is_empty() || matches!(value, ">" | "|") || value.len() > 256 {
        return Err(format!("invalid scalar value for roadmap field {field}"));
    }
    let value = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(format!("invalid scalar value for roadmap field {field}"));
    }
    Ok(value.to_owned())
}

fn canonical_policy_id(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 128 {
        return Err(format!("invalid roadmap id: {value:?}"));
    }
    let mut canonical = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            canonical.push(character.to_ascii_uppercase());
        } else if !matches!(character, '_' | '-' | '.') {
            return Err(format!("invalid roadmap id: {value:?}"));
        }
    }
    if canonical.is_empty() {
        return Err(format!("invalid roadmap id: {value:?}"));
    }
    Ok(canonical)
}

fn task_mentions_policy_id(text: &str, canonical_id: &str) -> bool {
    text.split(|character: char| {
        !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    })
    .filter(|token| !token.is_empty())
    .any(|token| canonical_policy_id(token).is_ok_and(|candidate| candidate == canonical_id))
}

fn body_targets_policy_id(body: &str, canonical_id: &str) -> bool {
    body.lines().any(|line| {
        body_target_value(line).is_some_and(|value| task_mentions_policy_id(value, canonical_id))
    })
}

fn body_target_value(line: &str) -> Option<&str> {
    let line = line.trim().trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '-' | '*' | '>' | '#')
    });
    let (label, value) = line.split_once(':')?;
    let label = label.trim().trim_matches('*').to_ascii_lowercase();
    if !matches!(
        label.as_str(),
        "target"
            | "current target"
            | "roadmap"
            | "roadmap item"
            | "milestone"
            | "stage"
            | "work item"
    ) {
        return None;
    }
    let value = value.trim().trim_start_matches('*').trim();
    (!value.is_empty()).then_some(value)
}

fn deny_basis(rule: &RoadmapTaskRule) -> Option<(&'static str, &str)> {
    if rule
        .agent_policy
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("forbidden_to_initiate"))
    {
        return Some((
            "agent_policy",
            rule.agent_policy.as_deref().unwrap_or_default(),
        ));
    }
    if rule
        .status
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("human_only_blocked"))
    {
        return Some(("status", rule.status.as_deref().unwrap_or_default()));
    }
    if rule
        .execution_policy
        .as_deref()
        .is_some_and(|value| value.to_ascii_lowercase().contains("human_only"))
    {
        return Some((
            "execution_policy",
            rule.execution_policy.as_deref().unwrap_or_default(),
        ));
    }
    None
}

fn rule_is_denied(rule: &RoadmapTaskRule) -> bool {
    deny_basis(rule).is_some()
}

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

#[cfg(test)]
pub(crate) fn test_snapshot_for_validation(
    repository: &str,
    base_branch: &str,
    base_sha: &str,
) -> PolicySnapshot {
    PolicySnapshot {
        repository: repository.to_owned(),
        base_branch: base_branch.to_owned(),
        base_sha: base_sha.to_owned(),
        bootstrap: None,
        documents: Vec::new(),
    }
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
            "orchestrator-policy-test-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn git(directory: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(directory)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn commit(directory: &Path, message: &str) -> String {
        git(directory, &["add", "-A"]);
        git(
            directory,
            &[
                "-c",
                "user.name=policy-test",
                "-c",
                "user.email=policy-test@example.invalid",
                "commit",
                "-q",
                "-m",
                message,
            ],
        );
        git(directory, &["rev-parse", "HEAD"])
    }

    #[test]
    fn pointer_parser_deduplicates_and_rejects_unsafe_values() {
        let bootstrap = "read `origin/agent/roadmap:.agent/ROADMAP.yaml` then git show origin/agent/roadmap:.agent/ROADMAP.yaml";
        let pointers = extract_policy_pointers(bootstrap).unwrap();
        assert_eq!(pointers.len(), 1);
        assert_eq!(pointers[0].ref_name, "agent/roadmap");
        assert_eq!(pointers[0].path, ".agent/ROADMAP.yaml");

        assert!(extract_policy_pointers("origin/../bad:.agent/x.yaml").is_err());
        assert!(extract_policy_pointers("origin/agent/ok:../secret").is_err());
        assert!(extract_policy_pointers("origin/agent/*:.agent/x.yaml").is_err());
    }

    #[test]
    fn snapshot_loads_exact_bootstrap_and_referenced_identity() {
        let root = temporary_root("load");
        let origin = root.join("origin.git");
        let work = root.join("work");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "--bare", origin.to_str().unwrap()]);
        git(&root, &["init", "-q", "-b", "main", work.to_str().unwrap()]);
        git(
            &work,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );

        fs::write(work.join("base.txt"), "base\n").unwrap();
        commit(&work, "base");
        git(&work, &["push", "-q", "-u", "origin", "main"]);

        git(&work, &["checkout", "-q", "-b", "agent/roadmap"]);
        fs::create_dir_all(work.join(".agent")).unwrap();
        fs::write(work.join(".agent/ROADMAP.yaml"), "status: active\n").unwrap();
        let roadmap_commit = commit(&work, "roadmap");
        git(&work, &["push", "-q", "-u", "origin", "agent/roadmap"]);

        git(&work, &["checkout", "-q", "main"]);
        fs::write(
            work.join("AGENTS.md"),
            "Mandatory: `origin/agent/roadmap:.agent/ROADMAP.yaml`\n",
        )
        .unwrap();
        let base_sha = commit(&work, "bootstrap");
        git(&work, &["push", "-q", "origin", "main"]);
        git(&work, &["fetch", "-q", "origin", "--prune"]);

        let snapshot = load_snapshot(&work, "Memorithm/Test", "main", &base_sha).unwrap();
        assert_eq!(snapshot.base_sha(), base_sha);
        assert_eq!(snapshot.base_branch(), "main");
        assert_eq!(snapshot.documents.len(), 1);
        assert_eq!(snapshot.documents[0].commit_sha, roadmap_commit);
        assert!(snapshot.prompt_context().contains("status: active"));
        assert!(remote_identity_is_current(&work, &snapshot).unwrap());

        git(&work, &["checkout", "-q", "agent/roadmap"]);
        fs::write(work.join(".agent/ROADMAP.yaml"), "status: changed\n").unwrap();
        commit(&work, "advance roadmap");
        git(&work, &["push", "-q", "origin", "agent/roadmap"]);
        assert!(!remote_identity_is_current(&work, &snapshot).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn repository_without_bootstrap_is_allowed() {
        let root = temporary_root("no-bootstrap");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        fs::write(root.join("README.md"), "test\n").unwrap();
        let base_sha = commit(&root, "base");
        let snapshot = load_snapshot(&root, "Memorithm/Test", "main", &base_sha).unwrap();
        assert!(snapshot.bootstrap.is_none());
        assert!(snapshot.documents.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    fn snapshot_with_policy_documents(contents: &[&str]) -> PolicySnapshot {
        PolicySnapshot {
            repository: "Memorithm/Test".to_owned(),
            base_branch: "main".to_owned(),
            base_sha: "0".repeat(40),
            bootstrap: None,
            documents: contents
                .iter()
                .enumerate()
                .map(|(index, content)| PolicyDocument {
                    ref_name: format!("agent/policy-{index}"),
                    path: format!(".agent/POLICY-{index}.yaml"),
                    commit_sha: format!("{:040x}", index + 1),
                    blob_sha: format!("{:040x}", index + 101),
                    content: (*content).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn task_eligibility_blocks_explicit_human_only_roadmap_item() {
        let snapshot = snapshot_with_policy_documents(&[r#"schema_version: 1
roadmap:
  - id: TDI7_1
    status: complete_pending_human_confirmatory_execution
  - id: TDI7_2
    status: human_only_blocked
    execution_policy: explicit_human_only_confirmation_at_execution_time
    agent_policy: forbidden_to_initiate
  - id: TDIX
    status: planned_parallel
"#]);
        for spelling in ["TDI7_2", "TDI7.2", "TDI-7.2"] {
            let decision = snapshot
                .task_eligibility(&format!("Run {spelling} final holdout"), "")
                .unwrap();
            let TaskEligibility::Deferred(denial) = decision else {
                panic!("expected {spelling} to be denied");
            };
            assert_eq!(denial.item_id, "TDI7_2");
            assert_eq!(denial.field, "agent_policy");
        }
        assert_eq!(
            snapshot
                .task_eligibility("Advance TDIX evidence bridge", "")
                .unwrap(),
            TaskEligibility::Allowed
        );
        assert_eq!(
            snapshot
                .task_eligibility("Audit TDI7_20 fixture", "")
                .unwrap(),
            TaskEligibility::Allowed
        );
    }

    #[test]
    fn contextual_body_mentions_do_not_target_a_prohibited_item() {
        let snapshot = snapshot_with_policy_documents(&[r#"roadmap:
  - id: TDI7_1
    status: active
  - id: TDI7_2
    status: human_only_blocked
    agent_policy: forbidden_to_initiate
"#]);
        let broad_issue_body = r#"# Active research programme — TDI-7.x
## TDI-7.1 — deterministic evaluator
CI proves normal tests cannot produce TDI-7.2 results.
TDI-7.1 must stop before final holdout execution.
## TDI-7.2 — confirmatory result
The final TDI-7.2 holdout remains blocked.
Current development target:
- TDI-7.1 deterministic evaluator.
"#;
        assert_eq!(
            snapshot
                .task_eligibility(
                    "TDI-7.x — dynamic recovery diagnostics for attention",
                    broad_issue_body,
                )
                .unwrap(),
            TaskEligibility::Allowed
        );

        let targeted = snapshot
            .task_eligibility(
                "Run confirmatory evaluation",
                "Target: TDI-7.2 final holdout",
            )
            .unwrap();
        assert!(matches!(targeted, TaskEligibility::Deferred(_)));
    }

    #[test]
    fn task_eligibility_rejects_duplicate_or_conflicting_structured_policy() {
        let duplicate = snapshot_with_policy_documents(&[r#"roadmap:
  - id: X1
    status: human_only_blocked
    status: active
"#]);
        assert!(duplicate.task_eligibility("X1", "").is_err());

        let conflicting = snapshot_with_policy_documents(&[
            "roadmap:\n  - id: X2\n    status: active\n",
            "roadmap:\n  - id: X2\n    agent_policy: forbidden_to_initiate\n",
        ]);
        assert!(conflicting.task_eligibility("X2", "").is_err());
    }

    #[test]
    fn global_action_policy_denies_only_explicit_matching_category() {
        let snapshot = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 1
  financial_execution: deny
  custody_mutation: allow
"#]);
        let denied = snapshot
            .task_eligibility(
                "Implement payout adapter",
                "Action category: financial_execution",
            )
            .unwrap();
        let TaskEligibility::Deferred(denial) = denied else {
            panic!("expected global financial deny");
        };
        assert_eq!(denial.item_id, "global:financial_execution");
        assert_eq!(denial.field, "autonomous_action_policy");
        assert!(
            denial
                .reason("Memorithm/Test", &snapshot)
                .contains("origin/agent/policy-0:.agent/POLICY-0.yaml")
        );

        assert_eq!(
            snapshot
                .task_eligibility("Update custody docs", "Autonomous action: custody_mutation",)
                .unwrap(),
            TaskEligibility::Allowed
        );
        assert_eq!(
            snapshot
                .task_eligibility("Analyze payout design", "")
                .unwrap(),
            TaskEligibility::Allowed
        );
    }

    #[test]
    fn task_scoped_deny_precedes_global_allow() {
        let snapshot = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 1
  financial_execution: allow
roadmap:
  - id: FIN1
    agent_policy: forbidden_to_initiate
"#]);
        assert!(matches!(
            snapshot
                .task_eligibility("Run FIN1 payout", "Action category: financial_execution",)
                .unwrap(),
            TaskEligibility::Deferred(_)
        ));
    }

    #[test]
    fn global_action_policy_is_strict_and_does_not_promote_free_text() {
        let free_text = snapshot_with_policy_documents(&[r#"notes: >-
  autonomous_action_policy: financial_execution deny
  custody and wallet changes are risky words only
"#]);
        assert_eq!(
            free_text
                .task_eligibility("Finance analysis", "Action category: financial_execution",)
                .unwrap(),
            TaskEligibility::Allowed
        );

        let unknown = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 1
  arbitrary_new_category: deny
"#]);
        assert!(
            unknown
                .task_eligibility("Work", "Action category: financial_execution")
                .is_err()
        );

        let future = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 2
  financial_execution: deny
"#]);
        assert!(
            future
                .task_eligibility("Work", "Action category: financial_execution")
                .is_err()
        );

        let conflicting = snapshot_with_policy_documents(&[
            "autonomous_action_policy:\n  schema_version: 1\n  financial_execution: deny\n",
            "autonomous_action_policy:\n  schema_version: 1\n  financial_execution: allow\n",
        ]);
        assert!(
            conflicting
                .task_eligibility("Work", "Action category: financial_execution")
                .is_err()
        );

        let duplicate_task = snapshot_with_policy_documents(&[r#"autonomous_action_policy:
  schema_version: 1
  financial_execution: deny
"#]);
        assert!(
            duplicate_task
                .task_eligibility(
                    "Work",
                    "Action category: financial_execution\nAutonomous action: custody_mutation",
                )
                .is_err()
        );
    }

    #[test]
    fn portable_validation_plan_is_structured_and_bounded() {
        let snapshot = snapshot_with_policy_documents(&[r#"validation_plan:
  schema_version: 1
  class: portable
  steps:
    - id: fmt
      argv: [cargo, fmt, --all, --, --check]
      cwd: .
      timeout_seconds: 120
    - id: tests
      argv: [cargo, test, --workspace]
      cwd: crates/core
      timeout_seconds: 300
"#]);
        let plan = snapshot
            .portable_validation_plan()
            .unwrap()
            .expect("portable plan");
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].id, "fmt");
        assert_eq!(plan.steps[0].argv[0], "cargo");
        assert_eq!(plan.steps[1].cwd, "crates/core");
        assert_eq!(plan.steps[1].timeout_seconds, 300);
        assert_eq!(plan.source_ref, "agent/policy-0");
    }

    #[test]
    fn validation_plan_preserves_non_shell_argument_bytes() {
        let snapshot = snapshot_with_policy_documents(&[r#"validation_plan:
  schema_version: 1
  class: portable
  steps:
    - id: literal
      argv: [touch, literal;touch injected]
"#]);
        let plan = snapshot
            .portable_validation_plan()
            .unwrap()
            .expect("portable plan");
        assert_eq!(plan.steps[0].argv, ["touch", "literal;touch injected"]);

        let quoted = snapshot_with_policy_documents(&[r#"validation_plan:
  schema_version: 1
  class: portable
  steps:
    - id: features
      argv: [cargo, test, --features, "foo,bar"]
"#]);
        let quoted_plan = quoted
            .portable_validation_plan()
            .unwrap()
            .expect("quoted portable plan");
        assert_eq!(
            quoted_plan.steps[0].argv,
            ["cargo", "test", "--features", "foo,bar"]
        );
    }

    #[test]
    fn validation_plan_rejects_shell_unsafe_or_ambiguous_structure() {
        for content in [
            "validation_plan:\n  schema_version: 2\n  class: portable\n  steps:\n    - id: x\n      argv: [cargo, check]\n",
            "validation_plan:\n  schema_version: 1\n  class: hardware\n  steps:\n    - id: x\n      argv: [cargo, check]\n",
            "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [bash, -c, echo bad]\n",
            "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [../tool]\n",
            "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [cargo, check]\n      cwd: ../outside\n",
            "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [cargo, check]\n    - id: x\n      argv: [cargo, test]\n",
            "validation_plan:\n  schema_version: 1\n  class: portable\n  unknown: value\n  steps:\n    - id: x\n      argv: [cargo, check]\n",
        ] {
            assert!(
                snapshot_with_policy_documents(&[content])
                    .portable_validation_plan()
                    .is_err()
            );
        }
        let duplicate = snapshot_with_policy_documents(&[
            "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: x\n      argv: [cargo, check]\n",
            "validation_plan:\n  schema_version: 1\n  class: portable\n  steps:\n    - id: y\n      argv: [cargo, test]\n",
        ]);
        assert!(duplicate.portable_validation_plan().is_err());
    }

    #[test]
    fn validation_plan_is_not_inferred_from_free_text() {
        let snapshot = snapshot_with_policy_documents(&[r#"notes: >-
  validation_plan: cargo test --workspace
"#]);
        assert!(snapshot.portable_validation_plan().unwrap().is_none());
    }

    #[test]
    fn merge_evidence_policy_distinguishes_portable_hardware_and_human() {
        let portable = snapshot_with_policy_documents(&[r#"merge_evidence_policy:
  schema_version: 1
  required: portable_ci
"#]);
        assert_eq!(
            portable.merge_evidence_eligibility().unwrap(),
            MergeEvidenceEligibility::PortableCi
        );
        assert_eq!(
            snapshot_with_policy_documents(&[])
                .merge_evidence_eligibility()
                .unwrap(),
            MergeEvidenceEligibility::Inherit
        );

        for required in ["hardware_required", "human_required"] {
            let content =
                format!("merge_evidence_policy:\n  schema_version: 1\n  required: {required}\n");
            let snapshot = snapshot_with_policy_documents(&[&content]);
            let MergeEvidenceEligibility::Deferred(denial) =
                snapshot.merge_evidence_eligibility().unwrap()
            else {
                panic!("expected {required} to defer merge");
            };
            assert_eq!(denial.value, required);
            let reason = denial.merge_reason("Memorithm/Test", &snapshot);
            assert!(reason.contains(required));
            assert!(reason.contains("source=origin/agent/policy-0:.agent/POLICY-0.yaml"));
        }
    }

    #[test]
    fn merge_evidence_policy_is_strict_and_not_inferred_from_prose() {
        let prose = snapshot_with_policy_documents(&[r#"notes: >-
  merge_evidence_policy hardware_required
  physical GPU evidence is required in prose only
"#]);
        assert_eq!(
            prose.merge_evidence_eligibility().unwrap(),
            MergeEvidenceEligibility::Inherit
        );

        let duplicate = snapshot_with_policy_documents(&[
            "merge_evidence_policy:\n  schema_version: 1\n  required: portable_ci\n",
            "merge_evidence_policy:\n  schema_version: 1\n  required: hardware_required\n",
        ]);
        assert!(duplicate.merge_evidence_eligibility().is_err());

        for content in [
            "merge_evidence_policy:\n  schema_version: 2\n  required: portable_ci\n",
            "merge_evidence_policy:\n  schema_version: 1\n  required: self_reported_gpu\n",
            "merge_evidence_policy:\n  schema_version: 1\n  unknown: hardware_required\n",
            "merge_evidence_policy:\n  schema_version: 1\n  required: portable_ci\n  required: human_required\n",
        ] {
            assert!(
                snapshot_with_policy_documents(&[content])
                    .merge_evidence_eligibility()
                    .is_err()
            );
        }
    }

    #[test]
    fn merge_policy_is_explicit_versioned_and_source_bound() {
        let denied = snapshot_with_policy_documents(&[r#"autonomous_merge_policy:
  schema_version: 1
  decision: deny
"#]);
        let MergeEligibility::Deferred(denial) = denied.merge_eligibility().unwrap() else {
            panic!("expected explicit merge deny");
        };
        assert_eq!(denial.item_id, "global:auto_merge");
        assert_eq!(denial.field, "autonomous_merge_policy");
        let reason = denial.merge_reason("Memorithm/Test", &denied);
        assert!(reason.contains("source=origin/agent/policy-0:.agent/POLICY-0.yaml"));
        assert!(reason.contains("policy_identity="));

        let allowed = snapshot_with_policy_documents(&[r#"autonomous_merge_policy:
  schema_version: 1
  decision: allow
"#]);
        assert_eq!(
            allowed.merge_eligibility().unwrap(),
            MergeEligibility::Allowed
        );
        assert_eq!(
            snapshot_with_policy_documents(&[])
                .merge_eligibility()
                .unwrap(),
            MergeEligibility::Inherit
        );
    }

    #[test]
    fn merge_policy_rejects_ambiguous_or_unknown_structure() {
        let duplicate = snapshot_with_policy_documents(&[
            "autonomous_merge_policy:\n  schema_version: 1\n  decision: deny\n",
            "autonomous_merge_policy:\n  schema_version: 1\n  decision: allow\n",
        ]);
        assert!(duplicate.merge_eligibility().is_err());

        for content in [
            "autonomous_merge_policy:\n  schema_version: 2\n  decision: deny\n",
            "autonomous_merge_policy:\n  schema_version: 1\n  decision: maybe\n",
            "autonomous_merge_policy:\n  schema_version: 1\n  unknown: deny\n",
            "autonomous_merge_policy:\n  schema_version: 1\n  decision: deny\n  decision: allow\n",
        ] {
            assert!(
                snapshot_with_policy_documents(&[content])
                    .merge_eligibility()
                    .is_err()
            );
        }
    }

    #[test]
    fn merge_policy_is_not_inferred_from_free_text() {
        let snapshot = snapshot_with_policy_documents(&[r#"notes: >-
  autonomous_merge_policy: decision deny
  do not auto merge financial work
"#]);
        assert_eq!(
            snapshot.merge_eligibility().unwrap(),
            MergeEligibility::Inherit
        );
    }

    #[test]
    fn free_text_is_not_promoted_into_task_policy() {
        let snapshot = snapshot_with_policy_documents(&[r#"schema_version: 1
notes: >-
  agent_policy: forbidden_to_initiate and human_only_blocked are words here,
  not a roadmap item.
financial_rule: agents must never authorize custody from model output
"#]);
        assert_eq!(
            snapshot
                .task_eligibility("financial custody analysis", "forbidden_to_initiate")
                .unwrap(),
            TaskEligibility::Allowed
        );
    }

    #[test]
    fn base_branch_validation_accepts_master_and_nonstandard_defaults() {
        for branch in ["master", "release/stable"] {
            let root = temporary_root(&branch.replace('/', "-"));
            fs::create_dir_all(&root).unwrap();
            git(&root, &["init", "-q", "-b", branch]);
            fs::write(root.join("README.md"), "test\n").unwrap();
            let base_sha = commit(&root, "base");
            let snapshot = load_snapshot(&root, "Memorithm/Test", branch, &base_sha).unwrap();
            assert_eq!(snapshot.base_branch(), branch);
            assert_eq!(snapshot.base_sha(), base_sha);
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn identity_evidence_is_append_only_per_attempt() {
        let root = temporary_root("persist");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        fs::write(root.join("README.md"), "test\n").unwrap();
        let base_sha = commit(&root, "base");
        let snapshot = load_snapshot(&root, "Memorithm/Test", "main", &base_sha).unwrap();
        let first = persist_identity(&root, "Memorithm/Test", "ISSUE", 7, 100, &snapshot).unwrap();
        let second = persist_identity(&root, "Memorithm/Test", "ISSUE", 7, 100, &snapshot).unwrap();
        assert_ne!(first, second);
        for evidence in [first, second] {
            let contents = fs::read_to_string(&evidence).unwrap();
            assert!(contents.contains(&format!("base-sha={base_sha}")));
        }
        let _ = fs::remove_dir_all(root);
    }
}
