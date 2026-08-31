# Memorithm Orchestrator repository agent instructions

Before repository changes, fetch and read the persistent off-main roadmap:

```bash
git fetch origin agent/ecosystem-roadmap && \
git show origin/agent/ecosystem-roadmap:.agent/ORCHESTRATOR_ECOSYSTEM_ROADMAP.yaml
```

Treat root `AGENTS.md` as mandatory bootstrap policy. If the roadmap is unavailable, fail closed for major scheduler, coding-backend, publication, cross-repository policy, or merge decisions.

Target-repository `AGENTS.md` and mandatory off-main roadmaps must be loaded before coding. Coding backends may edit/test but never inherit GitHub publication or merge authority. Exact-head required CI remains mandatory before merge.
