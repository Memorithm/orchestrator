# Autonomous research programmes

## Purpose

Memorithm Orchestrator should let the human operate as project manager rather than as an approval step inside every research iteration.

The project manager owns the programme objective, priorities, resource constraints, stop/veto decisions and any repository policy that intentionally reserves an action for a human. Once a programme issue explicitly opts in, the research agent owns the permitted scientific iteration: inspect evidence, formulate or revise hypotheses, select the next bounded experiment/control/ablation, implement it, execute permitted validation, analyze the result and decide whether to continue, revise or abandon the line.

Executed evidence remains authoritative. Research autonomy is not permission to manufacture evidence, make unsupported claims or reinterpret a failed/null/inconclusive result as success.

## Explicit opt-in

Research authority is never inferred from ordinary issue prose. A programme issue opts in with the exact whole-line directive:

```text
<!-- orchestrator-research-mode: autonomous-v1 -->
```

An optional stable programme identifier may follow:

```text
<!-- orchestrator-research-programme: TDI-8 -->
```

The parser is deliberately fail-closed. Duplicate directives, unsupported modes, malformed reserved comments, unknown reserved research keys and invalid programme identifiers are errors. A marker embedded in normal prose is inert and does not grant authority.

## Agent authority in `autonomous-v1`

For each independently publishable slice, the agent may:

- inspect current source, roadmap state, merged work, tests, benchmarks and executed evidence;
- formulate, rank, revise or reject hypotheses;
- choose the highest-value bounded evidence-producing next experiment, control or ablation that repository policy permits;
- implement code and local experiment/validation surfaces in its isolated workspace;
- analyze executed results, including negative, null and inconclusive outcomes;
- choose whether the permitted line should continue, be revised or be abandoned;
- proceed without asking the project manager to approve ordinary choices among already-permitted research actions.

The GitHub mutation unit remains one coherent reviewable PR/publication transaction. Broad programme issues remain open across slices unless their own completion contract says otherwise.

## Authority that does not move to the research agent

Target-repository policy remains authoritative and is resolved by Orchestrator before worker execution. Autonomous research cannot override:

- `human_only` or `forbidden_to_initiate` actions;
- final-holdout or other irreversible evidence-access rules;
- financial execution, custody, credentials or external-side-effect restrictions;
- hardware-evidence requirements;
- repository-specific validation plans;
- exact-head CI requirements;
- publication and merge policy;
- Orchestrator's prohibition on worker Git/GitHub mutation authority.

If the scientifically preferred next action is gated, the agent should advance the best permitted precursor or report the exact blocker. It must not infer authorization from the research-mode directive.

## Roadmap dependency contract

ORCH9g defines a strict machine-readable roadmap section for cross-repository research prerequisites. Dependencies are never inferred from prose, agent reports, repository names mentioned in documentation, CI labels or benchmark output.

```yaml
research_dependencies:
  schema_version: 1
  requires:
    - programme: TDI-8
      repository: Memorithm/SciRust
      merged_commit: 0123456789abcdef0123456789abcdef01234567
```

Each requirement binds one exact autonomous-research programme to one provider repository and one full Git object identifier that the parent scheduler must prove reachable from the provider's current default branch. The parser accepts only the versioned top-level section, bounds the number of requirements, rejects duplicate requirements and fails closed on malformed programme identifiers, repository identities, commits, indentation or unknown fields.

The declaration is a requested scheduling prerequisite, not proof that the commit is merged. Repository-controlled roadmap text, the research agent and persisted research-cycle reports cannot self-attest satisfaction.

## Parent-owned provider resolution

ORCH9h enforces declared provider prerequisites before an autonomous research worker is launched. The parent-side resolver consumes only the canonical issue directive and parent-resolved repository policy snapshot already bound into the worker prompt. Untrusted CI evidence lies outside that boundary and cannot inject or satisfy a dependency declaration.

For each requirement matching the exact research programme, the parent resolver:

1. asks GitHub for the provider repository's current default branch;
2. resolves the exact current default-branch head;
3. compares the declared full provider commit to that exact head; and
4. accepts the prerequisite only when GitHub reports the default head as identical to, or ahead of, the required commit.

A declaration is therefore never self-attesting. `behind`, `diverged`, unknown comparison states, malformed provider metadata and GitHub/API failures do not count as success. Infrastructure failures remain fail-closed. If a valid prerequisite is not yet on the provider's default-branch history, the OpenCode worker is not started and no research-cycle handoff is recorded.

ORCH9i promotes that condition to a first-class scheduler `deferred` outcome. The parent issue worker evaluates the same parent-resolved policy snapshot and live GitHub default-branch comparison before launching the coding agent. An unresolved prerequisite records a non-failure trajectory detail that names the programme, provider repository, required commit, observed default head, compare status and consumer policy identity. The OpenCode bridge remains defense-in-depth: it still refuses to start the worker, but the scheduler no longer treats the skip as an ordinary no-change success.

The same gate is revalidated before a PREPARED publication is pushed and again before PR creation. If the provider falls off the default-branch history after a local commit already exists, Orchestrator keeps the PREPARED transaction and defers without remote mutation. If the branch is already PUSHED when the prerequisite regresses, the transaction is retained fail-closed for manual review instead of opening a stale PR. Policy, validation, CI, hardware-evidence, financial, credential and exact-head merge gates remain unchanged.

## Project-manager model

The intended operating split is:

```text
PROJECT MANAGER
    objective / priorities / resource envelope / veto
                  |
                  v
ORCHESTRATOR POLICY + EVIDENCE GATES
                  |
                  v
AUTONOMOUS RESEARCH LOOP
    observe -> hypothesize -> design -> execute -> analyze -> decide
                  |
                  v
ONE REVIEWABLE PR -> VALIDATION -> CI -> MERGE GATES
                  |
                  +------> next permitted research slice
```

The ORCH9 programme now has explicit opt-in and mission generation, durable research-cycle state, bounded research budgets, state-derived cycle guidance, structured research-line identity, a strict roadmap dependency contract, parent-owned pre-worker provider resolution, first-class scheduler deferral and publication-time dependency revalidation. Later slices may add richer cross-repository scheduling; every existing policy, validation, CI, hardware-evidence, publication and exact-head merge gate remains authoritative.
