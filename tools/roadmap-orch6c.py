from pathlib import Path

path = Path('.agent/ORCHESTRATOR_ECOSYSTEM_ROADMAP.yaml')
text = path.read_text()
old = '    next_slice: ORCH6c_authoritative_remote_evidence_discovery_and_ingestion\n'
new = '    next_slice: ORCH6d_trusted_capability_inventory_and_runner_availability\n'
if text.count(old) != 1:
    raise SystemExit(f'next_slice anchor count={text.count(old)}')
text = text.replace(old, new, 1)
anchor = '''          - no_review_threads_or_reviews_blocked_merge_and_no_staging_artifacts_entered_product_branch
    goals:
'''
block = '''          - no_review_threads_or_reviews_blocked_merge_and_no_staging_artifacts_entered_product_branch
      - id: ORCH6c
        name: Authoritative remote hardware evidence discovery and ingestion
        status: complete
        completion_evidence:
          - issue_53_defined_fail_closed_remote_hardware_evidence_ingestion_scope
          - PR_55_merged_to_main_as_83240483249aa6357a5752fdd99ebda0710fcb40
          - exact_head_19b232423b5143f9d818250aa7bdbdac8fd8bcc1_ci_179_and_contributor_attribution_46_completed_success_before_merge
          - exact_artifact_name_is_only_a_locator_and_never_authorization
          - bounded_shell_free_discovery_and_download_into_Orchestrator_owned_ingest_state
          - extracted_payload_must_be_one_bounded_regular_non_symlink_hardware_evidence_file
          - candidate_verification_is_confined_to_state_hardware_ingest_and_reuses_ORCH6a_pinned_attestation_rules
          - verified_candidate_is_promoted_without_clobber_then_canonical_ORCH6a_verification_runs_again
          - durable_ingest_state_binds_exact_candidate_identity_and_records_payload_digest_and_download_timestamps
          - staging_runtime_sandbox_workspace_tests_and_clippy_completed_success_before_clean_PR_publication
          - no_review_threads_open_at_merge_and_no_staging_artifacts_entered_product_branch
    goals:
'''
if text.count(anchor) != 1:
    raise SystemExit(f'ORCH6b completion anchor count={text.count(anchor)}')
text = text.replace(anchor, block, 1)
path.write_text(text)
