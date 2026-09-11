# Orchestrator holdouts and denylist — V0

Complements `AGENTS.md`. Repository-specific rules override scheduler defaults.

## Never AUTO_MERGE

| Repository | Reason |
|---|---|
| TDI | freeze 11.2 fields, labeled holdouts, issue #151 |
| itd-simulator | V29.18 frozen simulator |
| Replikans | custody / secrets gate before any demo |
| nonlocal-relativity-v2 | SciRust fork snapshot (see FORK.md) |
| SoulSystem | vendor freeze; no merge of sibling monorepo copies |
| scirust-automotive | private empty stub |

`ORCHESTRATOR_AUTO_MERGE` must stay `0` for the rows above even if the environment flag is `1`. Implement as a repository-name denylist in code in V1; until then this file is the fail-closed policy.

## Do not schedule feature work

- `nonlocal-relativity-v2` except documentation of the fork notice
- any path matching SoulSystem `VENDOR_POLICY.md` frozen prefixes
- new repository creation (no 31st repo until `scirust-hub/CATALOG.md` is CI-backed)

## Do not invent a second lab

KV experiments stay in KVLab. Agent adversity stays in GOT. Formal work stays in ProofLab. Attention discovery stays in ADA / FLAT graduation crates.

## Exact-head rule unchanged

No merge unless required CI is green on the exact PR head. No force push. No admin bypass.
