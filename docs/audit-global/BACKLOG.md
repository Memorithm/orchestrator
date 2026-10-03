# Backlog de remédiation — 30 septembre 2026

101 IDs ; 12 positifs conservés. Dernier état inscrit dans `registry.json` : lot 1 à 37/50 ; cumul fusionné : 37 ; clos : 31 ; restants : 70.

État réconcilié du 3 octobre 2026 à 06:28 Europe/Paris : **37/50 PR techniques fusionnées**, **31/101 constats clos**, **70 restants**. `FORGE-01` et `FORGE-02` sont clos sur `main` au SHA `82df6c5bdeb60911be88cf3efd8b740133e9f9cc`. La PR [Forge #50](https://github.com/Memorithm/Forge/pull/50) impose l’admission stricte avant toute compilation/exécution native et refuse avant spawn tant qu’aucun backend conteneur ou plus fort ne satisfait réellement le contrat. Son arbre candidat vert est exactement l’arbre intégré. Voir la [preuve](proofs/Forge-50-FORGE-02.json), la [CI exacte](https://github.com/Memorithm/Forge/actions/runs/37096187524) et le [journal](journal/2026-10-03.md). Aucune qualification GPU matérielle n’est revendiquée.

| ID | Dépôt | Priorité | Statut | Dépendances proposées |
|---|---|---|---|---|
| RT-SCI-01 | Memorithm/scirust | 9 | à faire |  |
| RT-SCI-02 | Memorithm/scirust | 1 | résolu |  |
| RT-SCI-03 | Memorithm/scirust | 9 | à faire |  |
| RT-TQ-01 | Memorithm/TurboQuant | 1 | résolu |  |
| RT-TQ-02 | Memorithm/TurboQuant | 9 | à faire | RT-TQ-01 |
| RT-TQ-03 | Memorithm/TurboQuant | 9 | à faire |  |
| RT-TQ-04 | Memorithm/TurboQuant | 9 | à faire |  |
| RT-TQ-05 | Memorithm/TurboQuant | 8 | résolu |  |
| RT-TQ-06 | Memorithm/TurboQuant | 5 | à faire |  |
| RT-TQ-07 | Memorithm/TurboQuant | 8 | résolu |  |
| RT-SLHA-01 | Memorithm/SLHAv2 | 9 | à faire |  |
| RT-SLHA-02 | Memorithm/SLHAv2 | 9 | à faire |  |
| RT-SLHA-05 | Memorithm/SLHAv2 | 9 | à faire |  |
| RT-FLAT-01 | Memorithm/FLAT-ATTENTION | 8 | à faire |  |
| RT-ELASTIC-01 | Memorithm/ElasticXxx | 8 | à faire |  |
| RT-NNIS-01 | Memorithm/NNIS | 9 | à faire |  |
| RT-NNIS-02 | Memorithm/NNIS | 2 | résolu | [preuve](proofs/NNIS-253-RT-NNIS-02.json), [PR #253](https://github.com/Memorithm/NNIS/pull/253) |
| RT-NNIS-03 | Memorithm/NNIS | 9 | à faire |  |
| RT-NNIS-05 | Memorithm/NNIS | 9 | à faire |  |
| RT-NNIS-06 | Memorithm/NNIS | 1 | résolu |  |
| MEM-01 | Memorithm/CCOS-Core | 9 | à faire |  |
| MEM-02 | Memorithm/CCOS-Core | 6 | à faire |  |
| MEM-03 | Memorithm/CCOS-Core | 1 | résolu | [preuve](proofs/CCOS-Core-MEM-03-bounded-persistence.json), [PR #31](https://github.com/Memorithm/CCOS-Core/pull/31) |
| MEM-04 | Memorithm/CCOS-Core | 2 | résolu |  |
| MEM-05 | Memorithm/CCOS-Core | 8 | à faire |  |
| MEM-06 | Memorithm/CCOS-Core | 9 | à faire |  |
| MEM-07 | Memorithm/CCOS-Enterprise | 9 | à faire |  |
| MEM-08 | Memorithm/CCOS-Enterprise | 9 | à faire | MEM-02, MEM-03, MEM-04 |
| MEM-09 | Memorithm/CCOS-Enterprise | 9 | à faire |  |
| MEM-10 | Memorithm/SciCapsule | 3 | résolu |  |
| MEM-11 | Memorithm/SciCapsule | 8 | à faire |  |
| MEM-12 | Memorithm/SciCapsule | 9 | à faire |  |
| MEM-13 | Memorithm/scirust-hub | 3 | résolu |  |
| MEM-14 | Memorithm/scirust-hub | 3 | résolu |  |
| MEM-15 | Memorithm/scirust-hub | 9 | à faire |  |
| MEM-16 | Memorithm/SciRust-Verify | 3 | résolu |  |
| MEM-17 | Memorithm/SciRust-Verify | 2 | résolu |  |
| MEM-18 | Memorithm/SciRust-Verify | 8 | résolu |  |
| MEM-19 | Memorithm/SciRust-Verify | 9 | à faire |  |
| COG-ADA-001 | Memorithm/ADA | 9 | à faire |  |
| COG-ADA-002 | Memorithm/ADA | 9 | à faire |  |
| COG-COGNO-001 | Memorithm/COGNO-1 | 4 | résolu | [preuve](proofs/COGNO-1-261-262-COG-COGNO-001-002-004.json), [PR #261](https://github.com/Memorithm/COGNO-1/pull/261), [PR #262](https://github.com/Memorithm/COGNO-1/pull/262) |
| COG-COGNO-002 | Memorithm/COGNO-1 | 4 | résolu | [preuve](proofs/COGNO-1-261-262-COG-COGNO-001-002-004.json), [PR #261](https://github.com/Memorithm/COGNO-1/pull/261) |
| COG-COGNO-003 | Memorithm/COGNO-1 | 9 | à faire |  |
| COG-COGNO-004 | Memorithm/COGNO-1 | 4 | résolu | [preuve](proofs/COGNO-1-261-262-COG-COGNO-001-002-004.json), [PR #261](https://github.com/Memorithm/COGNO-1/pull/261) |
| COG-RSI-001 | Memorithm/RSI | 4 | résolu | [preuve](proofs/RSI-58-COG-RSI-001-002.json), [PR #58](https://github.com/Memorithm/RSI/pull/58) |
| COG-RSI-002 | Memorithm/RSI | 4 | résolu | [preuve](proofs/RSI-58-COG-RSI-001-002.json), [PR #58](https://github.com/Memorithm/RSI/pull/58) |
| COG-RSI-003 | Memorithm/RSI | 7 | résolu | [RSI #59](https://github.com/Memorithm/RSI/pull/59), [RSI #61](https://github.com/Memorithm/RSI/pull/61), [preuve](proofs/RSI-59-61-COG-RSI-003.json) |
| COG-OCTA-001 | Memorithm/octasoma | 6 | à faire |  |
| COG-OCTA-002 | Memorithm/octasoma | 1 | résolu | [preuve](proofs/octasoma-68-COG-OCTA-002.json), [PR #68](https://github.com/Memorithm/octasoma/pull/68) |
| COG-OCTA-003 | Memorithm/octasoma | 9 | à faire |  |
| COG-SML-001 | Périmètre privé — source interne | 9 | à faire |  |
| COG-SML-002 | Périmètre privé — source interne | 8 | à faire |  |
| COG-SML-003 | Périmètre privé — source interne | 9 | à faire |  |
| FORGE-01 | Memorithm/Forge | 5 | résolu | [Forge #49](https://github.com/Memorithm/Forge/pull/49), [preuve](proofs/Forge-49-FORGE-01.json), revalidé dans l’arbre exact vert de [Forge #50](https://github.com/Memorithm/Forge/pull/50) |
| FORGE-02 | Memorithm/Forge | 7 | résolu | [Forge #50](https://github.com/Memorithm/Forge/pull/50), [preuve](proofs/Forge-50-FORGE-02.json) |
| FORGE-03 | Memorithm/Forge | 9 | à faire |  |
| FORGE-04 | Memorithm/Forge | 3 | résolu | [preuve](proofs/Forge-48-FORGE-04.json), [PR #48](https://github.com/Memorithm/Forge/pull/48) |
| ORCH-01 | Memorithm/orchestrator | 7 | à faire |  |
| PAPERS-01 | Memorithm/PAPERS-AGENT | 7 | à faire |  |
| PAPERS-02 | Memorithm/PAPERS-AGENT | 7 | à faire |  |
| PAPERS-03 | Memorithm/PAPERS-AGENT | 9 | à faire |  |
| PAPERS-04 | Memorithm/PAPERS-AGENT | 9 | à faire |  |
| GOT-01 | Memorithm/GOT | 5 | à faire |  |
| GOT-02 | Memorithm/GOT | 5 | à faire |  |
| GOT-03 | Memorithm/GOT | 5 | à faire | GOT-01, GOT-02 |
| B-TDI-01 | Memorithm/TDI | 9 | à faire |  |
| B-TDI-02 | Memorithm/TDI | 9 | à faire |  |
| B-ITD-01 | Memorithm/itd-simulator | 9 | à faire |  |
| B-ITD-02 | Memorithm/itd-simulator | 9 | à faire |  |
| B-ITD-03 | Memorithm/itd-simulator | 9 | à faire | B-NO-03 |
| B-NR-01 | Périmètre privé — source interne | 8 | à faire |  |
| B-NR-02 | Périmètre privé — source interne | 9 | à faire |  |
| B-RH-01 | Memorithm/riemann_ndim_bench | 8 | à faire |  |
| B-RH-02 | Memorithm/riemann_ndim_bench | 6 | à faire |  |
| B-PL-01 | Memorithm/ProofLab | 6 | résolu | [preuve](proofs/ProofLab-83-B-PL-01.json), [PR #83](https://github.com/Memorithm/ProofLab/pull/83), [CI cible](https://github.com/Memorithm/ProofLab/actions/runs/37082869060) |
| B-PL-02 | Memorithm/ProofLab | 3 | résolu | [preuves #77–#80](proofs/ProofLab-80-B-PL-02.json), [politique agrégée cgroup #81/#82](proofs/ProofLab-81-82-B-PL-02.json), [PR #81](https://github.com/Memorithm/ProofLab/pull/81), [PR #82](https://github.com/Memorithm/ProofLab/pull/82), [CI cible](https://github.com/Memorithm/ProofLab/actions/runs/37079650851) |
| B-PL-03 | Memorithm/ProofLab | 9 | à faire | B-PL-01, B-PL-02 |
| B-FL-01 | Memorithm/FieldLab | 5 | à faire |  |
| B-FL-02 | Memorithm/FieldLab | 9 | à faire |  |
| B-FL-03 | Memorithm/FieldLab | 8 | à faire |  |
| B-NO-01 | Memorithm/NeuralOperator | 9 | à faire |  |
| B-NO-02 | Memorithm/NeuralOperator | 8 | à faire |  |
| B-NO-03 | Memorithm/NeuralOperator | 9 | à faire |  |
| B-NO-04 | Memorithm/NeuralOperator | 9 | à faire |  |
| B-NL-01 | Memorithm/NoiseLab | 5 | résolu | [PR 74](https://github.com/Memorithm/NoiseLab/pull/74), [preuve](proofs/NoiseLab-74-B-NL-01.json), [CI cible](https://github.com/Memorithm/NoiseLab/actions/runs/36902249415), [smoke cible](https://github.com/Memorithm/NoiseLab/actions/runs/36902249479) |
| B-NL-02 | Memorithm/NoiseLab | 1 | résolu | [PR 73](https://github.com/Memorithm/NoiseLab/pull/73), [preuve](proofs/NoiseLab-73-B-NL-02.json) |
| B-NL-03 | Memorithm/NoiseLab | 8 | à faire |  |
| BF-EE-01 | Memorithm/ExtremEngine | 1 | résolu |  |
| BF-EE-02 | Memorithm/ExtremEngine | 1 | résolu |  |
| BF-EE-03 | Memorithm/ExtremEngine | 1 | résolu |  |
| BF-KV-01 | Memorithm/KVLab | 9 | à faire |  |
| BF-KV-02 | Memorithm/KVLab | 9 | à faire |  |
| BF-KV-03 | Memorithm/KVLab | 9 | à faire |  |
| BF-KV-04 | Memorithm/KVLab | 8 | à faire |  |
| BF-KV-05 | Memorithm/KVLab | 9 | à faire |  |
| BF-BL-01 | Memorithm/BooleanLab | 8 | à faire |  |
| BF-BL-02 | Memorithm/BooleanLab | 8 | à faire |  |
| BF-EE-04 | Memorithm/ExtremEngine | 9 | à faire |  |
| BF-EE-05 | Memorithm/ExtremEngine | 9 | à faire |  |
| BF-EE-06 | Memorithm/ExtremEngine | 9 | à faire |  |
