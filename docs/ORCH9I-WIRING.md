# ORCH9i scheduler wiring

The evaluator lives in `src/research_dependency_gate.rs`.

To compile it without editing the 200k `main.rs` monolith, nest it under `policy.rs`:

```rust
#[path = "research_dependency_gate.rs"]
mod research_dependency_gate;
```

Then, before every `TaskEligibility::Allowed` return in `task_eligibility`, call `finish_task_eligibility` from `src/policy_orch9i.inc.rs` (`include!` inside the `impl PolicySnapshot` block).

Existing `execute_issue` already maps `TaskEligibility::Deferred` to `ActionExecution::deferred`, so a provider miss becomes a first-class scheduler deferral before OpenCode starts. Roadmap `human_only` / deny rules still run first and are unchanged.

Publication-time revalidation (PREPARED keep / PUSHED fail-closed) still needs the four sites in `src/main.rs` listed previously.
