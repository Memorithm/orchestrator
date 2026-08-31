# Memorithm Orchestrator

Autonomous development control plane for the Memorithm GitHub organization.

The runner is intentionally local-first and cost-controlled:

- GitHub discovery and mutation use the authenticated `gh` CLI.
- Coding is delegated to OpenCode in non-interactive `run --auto` mode.
- The default and only accepted LLM provider for autonomous runs is local Ollama.
- Default model: `ollama/qwen3.8:latest`.
- No OpenAI API key is required or used.
- Codex CLI may be installed for manual fallback, but autonomous `run` does not invoke it.

## Commands

```text
orchestrator doctor
orchestrator health
orchestrator status
orchestrator scan [organization]
orchestrator triage [organization]
orchestrator preflight [organization]
orchestrator run [organization]
orchestrator run-once [organization]
```

`scan` classifies repositories before they are eligible for autonomous work. Archived, empty, forked, self, and repositories with non-standard default branches are not automatically mutated.

`triage` is read-only. Its deterministic priority is:

1. failing PR CI;
2. trusted PRs with definitively passing checks;
3. open issues permitted by the repository PR gate;
4. pending CI, no checks, unknown CI, and external PR states are non-actionable.

Open PRs whose author does not match the currently authenticated GitHub account are classified `EXTERNAL_PR`: they block new issue work in that repository but are never repaired or merged automatically.

### PR lifecycle doctrine

Autonomous coding is PR-only. Every coding change is either a repair pushed to an already-open trusted PR or a new issue slice published on its own branch and tracked by a PR; autonomous coding never lands directly on a repository default branch. A repository may start another issue slice while trusted PRs remain open only when **every** such PR has definitively `PASSING` CI. `FAILED` CI is repaired first. `PENDING`, `NO_CHECKS`, `UNKNOWN`, and external/untrusted PRs block the next slice. With auto-merge enabled, a trusted passing PR has higher scheduler priority than new issue work and is exact-head revalidated before merge; with auto-merge disabled, a trusted passing PR remains tracked but does not block the next reviewable slice.

The same gate is re-checked immediately before an issue branch is pushed. If the gate closes while the local agent is coding, Orchestrator persists a `Prepared` publication transaction and defers the push until the repository is green again. Autonomous commits always use `ZEKRITI Tarek <194770978+CHECKUPAUTO@users.noreply.github.com>` for both author and committer and never add `Co-authored-by:` trailers.

`preflight` acquires the exclusive Orchestrator instance lock, validates the local tool/model/sandbox runtime, inspects persistent state and host resource admission, performs live read-only GitHub triage, and previews the runtime scheduler without recovering interrupted leases, launching an agent, or mutating a managed repository.

`run` continuously repeats triage and executes one actionable unit per cycle.

## Autonomous workflow

For a failing trusted PR, Orchestrator checks out the existing PR branch and binds repair work to that exact remote head. It checks live PR `state` and head around each CI read and requires the PR to remain `OPEN`, at that exact head, and `FAILED` before agent execution, after agent execution, after validation, and immediately before push. If the PR closes, the head moves, or CI is no longer failed, the repair is deferred without publication. Otherwise Orchestrator validates the resulting working tree, commits, and pushes without force.

For an issue, Orchestrator creates a dedicated branch from the repository default branch and binds execution to both the selected issue revision and the repository eligibility snapshot. The canonical repository name, default branch, archive state, fork state, and emptiness are rechecked before and after agent execution and immediately around publication mutations. A stale PREPARED transaction is safely discarded before remote mutation; once a branch is PUSHED, an issue or repository transition fails closed and retains the transaction for manual review rather than creating a stale PR. The PR deliberately does not auto-close broad research issues.

For a trusted green PR, optional automatic merge is available through `ORCHESTRATOR_AUTO_MERGE=1`. The merge uses the observed PR head SHA and revalidates canonical authorship across the PR commit range. Before any merge, the PR head must already contain the freshly fetched default-branch tip. If it does not, Orchestrator appends a canonical no-force base-sync merge commit, validates the integrated tree, pushes it normally, and waits for fresh CI. Throughout merge validation, Orchestrator also rechecks live PR `state`, `headRefOid`, and draft status. A closed PR, changed head, or unexpected draft transition is deferred rather than mutated. Immediately before the final rebase merge it confirms that the remote base tip still equals the tip that was locally validated. Because the PR head already contains that exact base tip, the rebase replays the PR commits onto the same validated base instead of integrating against a newer unvalidated base. The final merge does not use admin bypass or force push.

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

Retry, cooldown, and quarantine state is scoped to the current work revision. Pull requests use their exact head SHA. Issues use GitHub's `updatedAt` value discovered with the issue, so a newly updated issue does not inherit semantic failure cooldowns from an older version of that issue.

Issue execution also rechecks the live GitHub `state` and `updatedAt` around body capture, agent execution, validation, push, and PR creation. Prepared publication transactions persist the issue revision that produced them. A stale PREPARED transaction is discarded before push; a stale already-PUSHED transaction is kept fail-closed for explicit review rather than creating a PR for a different issue revision.

Each work item keeps at most 50 managed trajectory JSONL files by default. Pruning happens only inside that work item's trajectory directory, removes only filenames generated by Orchestrator, never follows symlinks, and preserves unmanaged JSONL files. Set `ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=0` for unlimited retention.

## Environment

```text
ORCHESTRATOR_MODEL=ollama/qwen3.8:latest
ORCHESTRATOR_INTERVAL_SECS=180
ORCHESTRATOR_AUTO_MERGE=0
ORCHESTRATOR_FULL_VALIDATION=0
ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB=4096
ORCHESTRATOR_MIN_FREE_DISK_MB=8192
ORCHESTRATOR_MAX_LOAD_PER_CPU=2.0
ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS=4
ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES=1
ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS=604800
ORCHESTRATOR_TRAJECTORY_MAX_PER_ITEM=50
ORCHESTRATOR_MAX_CYCLES=0
ORCHESTRATOR_DATA_ROOT=~/.local/share/memorithm-orchestrator
```

`ORCHESTRATOR_MAX_CYCLES=0` means unlimited cycles.

Before executing a selected work item, the Linux runtime samples `MemAvailable`, free space on the filesystem containing `ORCHESTRATOR_DATA_ROOT`, and the one-minute load average. By default it defers work when less than 4096 MiB of memory is available, less than 8192 MiB of data-root disk space is free, or load exceeds 2.0 per available CPU. A resource deferral is not a research failure and therefore does not increase the failure/quarantine count. Set any resource threshold to `0` to disable that gate.

When disk pressure alone causes the deferral, Orchestrator first may reclaim at most `ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_TARGETS` managed workspace build caches (default 4) before resampling resources. Only a real `<data_root>/workspaces/<owner>__<repo>/target` directory is eligible, and only after the workspace has a real `.git` directory and its `origin` exactly matches the encoded GitHub repository. Symlink targets, foreign origins, sources, Git metadata, state, and trajectories are never removed. Set the target reclaim limit to `0` to disable this first stage.

If disk pressure remains, Orchestrator may remove at most `ORCHESTRATOR_LOW_DISK_RECLAIM_MAX_WORKSPACES` complete managed clones (default 1) that have been unused by Orchestrator for at least `ORCHESTRATOR_WORKSPACE_MIN_IDLE_SECS` seconds (default 604800, seven days). A clone becomes eligible only after Orchestrator has verified it and written an atomic usage marker; pre-existing unadopted clones are never garbage-collected. The currently selected repository is always excluded. Before deletion, the clone must be a real directory with a real `.git`, an exact matching GitHub origin, a symbolic branch HEAD, a completely clean status including ignored/untracked files, no stash, no local tags, no submodules or linked worktrees, no Git operation/lock marker, and every local branch must be recoverable from the same-named `origin/*` branch. Any uncertainty preserves the clone. Setting either the workspace reclaim limit or minimum idle time to `0` disables complete-clone GC.

The autonomous runner rejects any `ORCHESTRATOR_MODEL` that is not an installed `ollama/...` model.

## Launch

The repository includes `scripts/start.sh`, which builds a release binary, runs `doctor`, then keeps the orchestrator in the foreground so OpenCode activity and validation remain visible.

After synchronizing the local checkout, a fully autonomous run with merge enabled can be started with:

```bash
ORCHESTRATOR_AUTO_MERGE=1 ORCHESTRATOR_FULL_VALIDATION=1 bash scripts/start.sh Memorithm
```

Keep `ORCHESTRATOR_AUTO_MERGE=0` when you want Orchestrator to prepare and repair PRs but leave the final merge manual.
