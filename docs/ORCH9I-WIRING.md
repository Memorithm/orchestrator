# ORCH9i scheduler wiring

The evaluator is compiled in the library crate as
`orchestrator::research_dependency_gate`. It consumes only the issue body and
the parent-resolved policy snapshot text plus identity token. Untrusted CI
evidence cannot inject or satisfy a declaration.

`PolicySnapshot::finish_task_eligibility` is inlined in `impl PolicySnapshot`.
Every `TaskEligibility::Allowed` path in `task_eligibility` routes through that
method so `execute_issue` maps a provider miss to `ActionExecution::deferred`
before OpenCode starts. Roadmap `human_only` / deny rules still run first.

Publication-time revalidation uses `unresolved_research_provider`:

| Site | Phase | On unresolved prerequisite |
| --- | --- | --- |
| `execute_issue` before `git push` | PREPARED | keep transaction, `ActionExecution::deferred` |
| `execute_issue` before `gh pr create` | PUSHED | keep transaction, fail closed for manual review |
| `resume_issue_publication` before resumed push | PREPARED | keep transaction, `ActionOutcome::Deferred` |
| `resume_issue_publication` before resumed PR | PUSHED | keep transaction, fail closed for manual review |

`src/policy.rs` and `src/main.rs` are thin `include!` drivers so each worker
slice stays under the Contents API payload. Malformed directives/policy and
GitHub/API failures stay fail-closed. Policy, validation, CI,
hardware-evidence, financial, credential and exact-head merge gates are
unchanged.
