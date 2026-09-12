fn finish_task_eligibility(&self, body: &str) -> Result<TaskEligibility, String> {
    match research_dependency_gate::evaluate(body, self)? {
        research_dependency_gate::ResearchDependencyGate::Allow => Ok(TaskEligibility::Allowed),
        research_dependency_gate::ResearchDependencyGate::Defer { reason } => {
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
