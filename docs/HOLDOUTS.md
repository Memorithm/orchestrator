# Orchestrator holdouts and denylist — V0

Complements `AGENTS.md`. Repository-specific rules override scheduler defaults.

## Never AUTO_MERGE

Enforced two ways:

1. Target-repo ecosystem roadmap `autonomous_merge_policy` (schema_version 1, decision deny) — already loaded by Orchestrator from `AGENTS.md` off-main refs. Landed first on `Memorithm/itd-simulator` `agent/ecosystem-roadmap`.
2. In-process denylist `merge_policy::auto_merge_denied` — same names, environment flag cannot override.

| Repository | Reason |
|---|---|
| TDI | freeze 11.2 fields, labeled holdouts |
| itd-simulator | V29.18 frozen simulator |
| Replikans | custody / secrets gate before any demo |
| nonlocal-relativity-v2 | SciRust fork snapshot (see FORK.md) |
| SoulSystem | vendor freeze; no merge of sibling monorepo copies |
| scirust-automotive | private empty stub |

## Do not schedule feature work

- `nonlocal-relativity-v2` except documentation of the fork notice
- any path matching SoulSystem `VENDOR_POLICY.md` frozen prefixes
- new repository creation (no 31st repo until `scirust-hub/CATALOG.md` is CI-backed)

## Do not invent a second lab

KV experiments stay in KVLab. Agent adversity stays in GOT. Formal work stays in ProofLab. Attention discovery stays in ADA / FLAT graduation crates.

## Exact-head rule unchanged

No merge unless required CI is green on the exact PR head. No force push. No admin bypass.
