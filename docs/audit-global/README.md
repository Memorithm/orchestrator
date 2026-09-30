# Audit global Memorithm — remédiation

Suivi central : https://github.com/Memorithm/orchestrator/issues/81

`registry.json` contient les 101 IDs, priorités, dépendances proposées, critères d’acceptation, observations HEAD et preuves. Les détails privés restent dans les sources persistantes internes et leurs dépôts privés. Ne jamais publier le rapport ou JSON complet dans ce dépôt public.

La branche `audit/global-remediation-2026-09-30` est le registre opérationnel avant publication des snapshots par PR ; ne pas fusionner ses commits de contrôle. Chaque reprise lit cette branche et l’issue, réconcilie les PR/checks/HEAD avec GitHub, puis acquiert le lease avant une mutation. La présence d’un fichier en scratch n’est pas une preuve de publication.

## Verrou récupérable

Le lease est `docs/audit-global/lease.json` sur la branche de contrôle. L’acquisition initiale utilise create-file exclusif. Toute acquisition suivante, renouvellement ou libération utilise update-file avec le blob SHA observé. Un conflit annule la mutation. Relire le fichier et vérifier propriétaire, état et expiration avant chaque publication. Un lease actif appartenant à un autre passage interdit toute mutation de cette campagne. Ne reprendre qu’un lease explicitement libéré ou expiré ; un fichier mal formé bloque. La lecture reste possible.

Une branche PR appartient à une seule tâche/ID et un seul rédacteur. Avant une mise à jour, comparer son HEAD au dernier HEAD enregistré, ne jamais forcer et arrêter en cas de commit étranger. Ne pas toucher les branches de PR existantes non possédées par la campagne.

## Compteurs et fusion

Une campagne contient 50 PR correctives réellement fusionnées, le dernier lot peut être réduit. Les snapshots documentaires de suivi ne comptent pas comme correction technique. Identité unique d’une PR = dépôt + numéro ; revérifier merged, merge SHA et intégration avant comptage. Aucune double comptabilisation. Un constat déjà corrigé ne compte pas comme nouvelle PR ; sa clôture exige des preuves au HEAD intégré. Les blocages restent ouverts.

Chaque PR lie IDs, diff, risques, commandes, résultats, base/HEAD/policy et checks réellement exécutés. Ne fusionner qu’avec HEAD et base revalidés, tous les checks obligatoires réussis, état mergeable et protections respectées, expected_head_sha et méthode autorisée. Un ancien vert, un check sauté, un lint ou un mock ne constituent pas qualification fonctionnelle/hardware. Vérifier la branche cible et ses validations après merge. Aucun contournement ni réduction de gate.

## Périmètre et reprise

Exclus : CHECKUPAUTO.FR, RemoteOps, SoulSystem, scirust-automotive. RemoteOps peut servir de moyen existant sans modification. Les branches par défaut viennent des métadonnées GitHub, notamment ExtremEngine `agent/initial-engine`, SciRust-Verify `feat/scirust-verify-foundation`, et les trunks `master` lorsqu’observés. Lire AGENTS, roadmaps référencées, licences et instructions imbriquées avant tout edit. L’autorisation explicite de cette campagne couvre PR et merges ordinaires ; elle ne déverrouille pas les holdouts, les données confirmatoires ou les exigences matérielles.

Le journal ne couvre que les heures réellement observées et inclut état/ID/dépôt/actions/résultats/PR/SHA/tests/attente/prochaine action. Bilan quotidien cumulatif dans `journal/`. Garder les 12 positifs et les refus fail-closed corrects. Préférer les capacités chez leurs propriétaires existants, sans nouveau scheduler, schéma concurrent ni nouveau dépôt non justifié.
