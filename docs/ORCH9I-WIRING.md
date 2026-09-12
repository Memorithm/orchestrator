# ORCH9i scheduler wiring

The evaluator lives in `src/research_dependency_gate.rs` and is nested under
`policy.rs` so the 200k `main.rs` monolith does not have to declare the module:

```rust
#[path = "research_dependency_gate.rs"]
mod research_dependency_gate;
```

`task_eligibility` routes every `Allowed` path through `finish_task_eligibility`.
Existing `execute_issue` already maps `TaskEligibility::Deferred` to
`ActionExecution::deferred`, so a provider miss is a first-class scheduler
deferral before OpenCode starts. Roadmap `human_only` / deny rules still run
first and are unchanged.

Publication-time revalidation uses `research_provider_gate` in `src/main.rs`:

| Site | Phase | On unresolved prerequisite |
| --- | --- | --- |
| `execute_issue` before `git push` | PREPARED | keep transaction, `ActionExecution::deferred` |
| `execute_issue` before `gh pr create` | PUSHED | keep transaction, fail closed for manual review |
| `resume_issue_publication` before resumed push | PREPARED | keep transaction, `ActionOutcome::Deferred` |
| `resume_issue_publication` before resumed PR | PUSHED | keep transaction, fail closed for manual review |

Malformed directives/policy and GitHub/API failures stay fail-closed.
Policy, validation, CI, hardware-evidence, financial, credential and
exact-head merge gates are unchanged.
