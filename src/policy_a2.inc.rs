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
            return self.finish_task_eligibility(body);
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
            return self.finish_task_eligibility(body);
        };
        if rule.decision == AutonomousActionDecision::Allow {
            return self.finish_task_eligibility(body);
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

    pub(crate) fn finish_task_eligibility(&self, body: &str) -> Result<TaskEligibility, String> {
        match orchestrator::research_dependency_gate::evaluate(
            body,
            &self.prompt_context(),
            &self.identity_token(),
        )? {
            orchestrator::research_dependency_gate::ResearchDependencyGate::Allow => {
                Ok(TaskEligibility::Allowed)
            }
            orchestrator::research_dependency_gate::ResearchDependencyGate::Defer { reason } => {
                Ok(TaskEligibility::Deferred(PolicyDenial {
                    item_id: "research_dependency".to_owned(),
                    field: "provider_prerequisite",
                    value: reason,
                    source_ref: self.base_branch.clone(),
                    source_path: "research_dependencies".to_owned(),
                    source_commit: self.base_sha.clone(),
                    source_blob: self.identity_token(),
                }))
            }
        }
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
        if rule.schema_version == 2 {
            if rule.required != MergeEvidenceClass::HardwareRequired {
                return Err("merge evidence schema v2 is reserved for hardware_required".to_owned());
            }
            let requirement_id = rule.requirement_id.ok_or_else(|| {
                "merge evidence schema v2 hardware_required is missing requirement_id".to_owned()
            })?;
            return Ok(MergeEvidenceEligibility::HardwareRequired(
                HardwareEvidenceRequirement { requirement_id },
            ));
        }
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
