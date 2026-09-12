# ORCH9i scheduler wiring

The evaluator is compiled in the library crate as
`orchestrator::research_dependency_gate`. It consumes only the issue body and
the parent-resolved policy snapshot text plus identity token. Untrusted CI
evidence cannot inject or satisfy a declaration.

`src/policy_orch9i.inc.rs` is the exact `finish_task_eligibility` method to
inline inside `impl PolicySnapshot` (this toolchain rejects `include!` in impl
position). Route every `TaskEligibility::Allowed` return through that method so
`execute_issue` maps a provider miss to `ActionExecution::deferred` before
OpenCode starts. Roadmap `human_only` / deny rules still run first.

Publication-time revalidation must call the same `evaluate` helper:

| Site | Phase | On unresolved prerequisite |
| --- | --- | --- |
| `execute_issue` before `git push` | PREPARED | keep transaction, `ActionExecution::deferred` |
| `execute_issue` before `gh pr create` | PUSHED | keep transaction, fail closed for manual review |
| `resume_issue_publication` before resumed push | PREPARED | keep transaction, `ActionOutcome::Deferred` |
| `resume_issue_publication` before resumed PR | PUSHED | keep transaction, fail closed for manual review |

`src/policy.rs` and `src/main.rs` still need those call sites inlined; they
exceed the Contents API payload used by this control plane, so they remain a
separate exact-head slice. Malformed directives/policy and GitHub/API failures
stay fail-closed. Policy, validation, CI, hardware-evidence, financial,
credential and exact-head merge gates are unchanged.
