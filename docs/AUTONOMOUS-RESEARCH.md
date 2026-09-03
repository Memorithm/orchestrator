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

This first ORCH9 slice defines the explicit versioned contract. Subsequent slices wire the directive into issue mission generation, persist research decisions/evidence across cycles, add bounded research budgets and then make dependency-aware research scheduling a first-class Orchestrator capability.
