# Memorithm Orchestrator Agent Bootstrap Contract

Before autonomous coding, scheduler changes, coding-backend changes, publication/merge policy changes, ecosystem integration, PR creation, or merge decisions, read:

```bash
git fetch origin agent/ecosystem-roadmap:refs/remotes/origin/agent/ecosystem-roadmap && \
git show origin/agent/ecosystem-roadmap:.agent/ORCHESTRATOR_ECOSYSTEM_ROADMAP.yaml
```

If the roadmap cannot be fetched or read, fail closed for major scheduler, coding-backend, publication, cross-repository policy, or merge decisions. Read-only diagnosis is allowed.

## Repository role

Memorithm Orchestrator owns the organization's GitHub development lifecycle: repository discovery, issue/PR triage, isolated workspace selection, coding delegation, validation, commit/push/PR publication, CI observation and optional exact-head merge.

It must not absorb the inner coding-agent semantics owned by SoulSystem, CCOS context-memory semantics, or SciRust Hub runtime/product orchestration.

## Mandatory target-repository policy

Before any agent edits another repository, Orchestrator must load that repository's root `AGENTS.md` when present and every mandatory off-main roadmap it references. Repository-specific scientific, financial, security, validation and merge rules override generic scheduler defaults. Missing mandatory policy fails closed.

A coding backend must never gain Git commit, push, PR, issue, workflow, release, secret or merge authority merely because it can edit and test code. Orchestrator retains publication authority.

No PR may be merged unless all required CI for the exact head is definitively green and the validated base/head identities are still current. No force push or admin bypass.

Reread the roadmap at every session start, before scheduler/backend/publication/merge policy changes, before ecosystem integrations, and before relevant PR/merge decisions.

Do not merge the roadmap itself into `main` unless the user explicitly requests it.
