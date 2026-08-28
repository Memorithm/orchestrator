# Memorithm Orchestrator

Autonomous development control plane for the Memorithm GitHub organization.

The runner is intentionally local-first and cost-controlled:

- GitHub discovery and mutation use the authenticated `gh` CLI.
- Coding is delegated to OpenCode in non-interactive `run --auto` mode.
- The default and only accepted LLM provider for autonomous runs is local Ollama.
- Default model: `ollama/muse-glimmer:latest`.
- No OpenAI API key is required or used.
- Codex CLI may be installed for manual fallback, but autonomous `run` does not invoke it.

## Commands

```text
orchestrator doctor
orchestrator scan [organization]
orchestrator triage [organization]
orchestrator run [organization]
orchestrator run-once [organization]
```

`scan` classifies repositories before they are eligible for autonomous work. Archived, empty, forked, self, and repositories with non-standard default branches are not automatically mutated.

`triage` is read-only. Its deterministic priority is:

1. failing PR CI;
2. PRs with passing/no checks;
3. open issues in repositories without an open PR;
4. pending CI and unknown CI states are non-actionable.

`run` continuously repeats triage and executes one actionable unit per cycle.

## Autonomous workflow

For a failing PR, Orchestrator checks out the existing PR branch, launches the local coding worker, validates the resulting working tree, commits, and pushes without force.

For an issue, Orchestrator creates a dedicated branch from the repository default branch, asks the agent for one small reviewable slice, validates it, commits, pushes, and opens a draft PR. The PR deliberately does not auto-close broad research issues.

For a green PR, optional automatic merge is available through `ORCHESTRATOR_AUTO_MERGE=1`. The merge uses the observed PR head SHA and does not use admin bypass.

## OpenCode containment

The autonomous worker receives an inline OpenCode policy that:

- enables only the `ollama` provider;
- denies access outside the dedicated repository workspace;
- denies interactive questions;
- explicitly denies `git commit`, `git push`, Git history rewriting, remote changes, PR creation/merge/edit/close, issue edits/closes, GitHub credential mutations, workflow dispatch/reruns, releases, secrets, variables, and common write forms of `gh api`.

The agent is allowed to inspect code, edit the working tree, run tests and use read-only GitHub commands. Orchestrator itself owns Git commits, pushes, PR creation, and optional merge.

## Validation

Before Orchestrator commits agent changes it always runs:

```text
git diff --check
```

For a repository with a root `Cargo.toml`, it additionally runs:

```text
cargo fmt --all -- --check
cargo check --workspace
```

Set `ORCHESTRATOR_FULL_VALIDATION=1` to also require:

```text
cargo test --workspace
```

The agent is separately instructed to run repository-specific tests relevant to its task.

## State and isolation

Dedicated clones live under:

```text
~/.local/share/memorithm-orchestrator/workspaces/
```

Override this with `ORCHESTRATOR_DATA_ROOT`.

A PID lock prevents two Orchestrator loops from mutating repositories concurrently. A stale Linux lock is recovered automatically after an unclean process exit.

## Environment

```text
ORCHESTRATOR_MODEL=ollama/muse-glimmer:latest
ORCHESTRATOR_INTERVAL_SECS=180
ORCHESTRATOR_AUTO_MERGE=0
ORCHESTRATOR_FULL_VALIDATION=0
ORCHESTRATOR_MAX_CYCLES=0
ORCHESTRATOR_DATA_ROOT=~/.local/share/memorithm-orchestrator
```

`ORCHESTRATOR_MAX_CYCLES=0` means unlimited cycles.

The autonomous runner rejects any `ORCHESTRATOR_MODEL` that is not an installed `ollama/...` model.

## Launch

The repository includes `scripts/start.sh`, which builds a release binary, runs `doctor`, then keeps the orchestrator in the foreground so OpenCode activity and validation remain visible.

After synchronizing the local checkout, a fully autonomous run with merge enabled can be started with:

```bash
ORCHESTRATOR_AUTO_MERGE=1 ORCHESTRATOR_FULL_VALIDATION=1 bash scripts/start.sh Memorithm
```

Keep `ORCHESTRATOR_AUTO_MERGE=0` when you want Orchestrator to prepare and repair PRs but leave the final merge manual.
