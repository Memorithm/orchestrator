# ORCH9i scheduler wiring

`research_dependency_gate` is present on `main` but not compiled into the binary (`src/main.rs` does not `mod` it). This slice binds it.

## Required binary edits

1. Add `mod research_dependency_gate;` next to the other `src/` modules.
2. After `TaskEligibility` succeeds and before `run_agent`:

```rust
match research_dependency_gate::evaluate(&body, &policy_snapshot) {
    Ok(research_dependency_gate::ResearchDependencyGate::Allow) => {}
    Ok(research_dependency_gate::ResearchDependencyGate::Defer { reason }) => {
        println!("research dependency gate: DEFERRED {reason}");
        return Ok(ActionExecution::deferred(reason));
    }
    Err(message) => {
        return Err(ActionFailure::new(state::FailureClass::Validation, message));
    }
}
```

3. Before pushing a PREPARED publication: same match; on `Defer`, keep the transaction and return `ActionExecution::deferred(reason)`.
4. Before `create_issue_pull_request` on a PUSHED transaction: same match; on `Defer`, return a publication error and retain the transaction.
5. Repeat 3-4 inside `resume_issue_publication` after loading `github_body(item)`.

Infrastructure messages from the GitHub resolver stay fail-closed. Ordinary issues and unmatched programmes remain inert (`Allow` without provider calls).
