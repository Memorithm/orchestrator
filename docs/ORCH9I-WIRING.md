# ORCH9i scheduler wiring

This branch binds `research_dependency_gate::evaluate` into the issue worker.

Sites:

- after task eligibility, before OpenCode launch
- before a PREPARED publication is pushed (transaction kept, first-class deferral)
- before PR creation on a PUSHED transaction (fail-closed, no stale PR)
- the same two publication sites on resume

Unresolved prerequisites are `ActionExecution::deferred` with programme, provider, required commit, default head, compare status and policy identity. Malformed policy and GitHub resolver failures stay fail-closed.
