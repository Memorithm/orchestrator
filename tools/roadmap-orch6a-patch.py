from pathlib import Path

path = Path('.agent/ORCHESTRATOR_ECOSYSTEM_ROADMAP.yaml')
text = path.read_text()

facts_anchor = '''  - publication_recovery_can_reuse_only_an_exact_complete_pass_without_bypassing_other_publication_or_merge_gates
'''
facts_new = '''  - publication_recovery_can_reuse_only_an_exact_complete_pass_without_bypassing_other_publication_or_merge_gates
  - ORCH6a_PR_50_merged_at_main_b1bcaed3eb94c88338de359aa946210730e32019_with_exact_head_e666c1be1b9a564ad7cbe14c7a98bb46e2c13b80_green_CI
  - authoritative_hardware_evidence_uses_local_pinned_signer_trust_and_exact_repository_PR_head_base_policy_requirement_binding
  - merge_evidence_policy_schema_2_is_reserved_for_hardware_required_with_explicit_requirement_id
  - missing_hardware_trust_evidence_or_verifier_defers_without_success_while_malformed_or_mismatched_state_fails_closed
  - human_required_remains_non_automatable_and_check_names_runner_labels_emulation_compile_success_and_prose_are_not_hardware_evidence
'''
if text.count(facts_anchor) != 1:
    raise SystemExit(f'facts anchor count={text.count(facts_anchor)}')
text = text.replace(facts_anchor, facts_new, 1)

old = '''  - id: ORCH6
    name: CI capability and hardware scheduling
    status: active_priority
    next_slice: ORCH6a_authoritative_hardware_evidence_contract
    goals:
      - distinguish_hosted_self_hosted_GPU_ARM_and_manual_gate_requirements
      - define_authoritative_hardware_evidence_provenance_that_cannot_be_satisfied_by_check_names_labels_or_repository_prose
      - defer_without_failure_when_required_runner_or_authoritative_evidence_is_unavailable
      - revalidate_exact_head_when_runner_or_evidence_returns
      - no_merge_until_all_required_gates_are_real_successes
'''
new = '''  - id: ORCH6
    name: CI capability and hardware scheduling
    status: active_priority
    next_slice: ORCH6b_capability_aware_hardware_evidence_discovery_and_dispatch
    completed_slices:
      - id: ORCH6a
        name: Authoritative hardware evidence contract with pinned attestations
        status: complete
        completion_evidence:
          - issue_49_defined_authoritative_hardware_evidence_contract
          - PR_50_merged_to_main_as_b1bcaed3eb94c88338de359aa946210730e32019
          - exact_head_e666c1be1b9a564ad7cbe14c7a98bb46e2c13b80_ci_174_and_contributor_attribution_41_completed_success_before_merge
          - workspace_Rust_tests_149_of_149_and_root_portable_validation_tests_10_of_10
          - merge_evidence_policy_v1_behavior_preserved_and_v2_hardware_requirement_id_strictly_scoped
          - local_Orchestrator_owned_signer_workflow_and_digest_trust_root_is_not_repository_policy
          - exact_manifest_repository_PR_head_base_policy_requirement_device_and_time_binding
          - gh_attestation_verify_pins_repository_signer_workflow_signer_digest_source_digest_and_SLSA_v1_predicate_type
          - missing_trust_evidence_or_verifier_defers_without_success_and_malformed_or_identity_mismatched_state_fails_closed
          - hardware_gate_revalidated_before_remote_mutations_and_immediately_before_final_merge
          - no_review_threads_open_at_merge_and_no_staging_artifacts_in_product_branch
    goals:
      - distinguish_hosted_self_hosted_GPU_ARM_and_manual_gate_requirements
      - define_authoritative_hardware_evidence_provenance_that_cannot_be_satisfied_by_check_names_labels_or_repository_prose
      - defer_without_failure_when_required_runner_or_authoritative_evidence_is_unavailable
      - discover_or_dispatch_only_through_Orchestrator_owned_capability_configuration_not_repository_claims
      - revalidate_exact_head_when_runner_or_evidence_returns
      - no_merge_until_all_required_gates_are_real_successes
'''
if text.count(old) != 1:
    raise SystemExit(f'ORCH6 anchor count={text.count(old)}')
text = text.replace(old, new, 1)
path.write_text(text)
