# Thor continuous-research deployment

Thor is the unattended host profile for Memorithm Orchestrator. The profile keeps the existing fail-closed repository, policy, CI, evidence and exact-head merge gates; it only enables continuous scheduling and permits Orchestrator to complete a merge when those gates already authorize it.

## Preconditions

Run as `root` on the ARM64 Thor host. The following commands must already be available and authenticated/configured where applicable:

- `git`
- `gh`
- `ollama`
- `opencode`
- `cargo` and `rustc`
- `bwrap`
- `systemctl`

The autonomous Ollama model configured by the installer is `qwen3.8:latest`. `gh auth status` must succeed for the authorized Memorithm identity before installation.

## Install

From a clean checkout of `Memorithm/orchestrator` on its validated default branch:

```bash
bash scripts/install-thor.sh
```

The Thor profile defaults to:

```text
ORCHESTRATOR_INTERVAL_SECS=60
ORCHESTRATOR_AUTO_MERGE=1
ORCHESTRATOR_AUTO_MERGE_SCOPE=orchestrator-validated
ORCHESTRATOR_FULL_VALIDATION=1
```

`ORCHESTRATOR_AUTO_MERGE=1` does not bypass any merge protection. A merge still requires the repository-specific policy, exact PR head, required CI, evidence class, canonical authorship and validated base-tip conditions enforced by Orchestrator.

To change the scheduling interval without editing repository files:

```bash
ORCHESTRATOR_INSTALL_INTERVAL_SECS=180 bash scripts/install-thor.sh
```

To deploy Thor temporarily without automatic merge:

```bash
ORCHESTRATOR_INSTALL_AUTO_MERGE=0 bash scripts/install-thor.sh
```

## Verify

The installer fails if the generated systemd unit is invalid or the service does not become active. After installation, verify the live daemon and follow its decisions:

```bash
systemctl status memorithm-orchestrator --no-pager --full
journalctl -u memorithm-orchestrator -n 200 --no-pager
journalctl -u memorithm-orchestrator -f
```

The service runs `scripts/start.sh Memorithm`, and systemd restarts it after process failure. The service retains GPU/device access while restricting autonomous filesystem writes to Orchestrator-managed data, build output and its private temporary directory.

## Continuous research contract

Research autonomy remains explicit and repository-scoped. Broad programme issues must opt in using the machine-readable directive already implemented by Orchestrator:

```html
<!-- orchestrator-research-mode: autonomous-v1 -->
<!-- orchestrator-research-programme: PROGRAMME-ID -->
```

Without that directive, ordinary issue semantics remain unchanged. With it, the research agent may choose the next bounded evidence-producing slice, revise hypotheses and preserve negative/null results, but it cannot override human-only, forbidden, financial, credential, holdout, hardware-evidence, validation, publication or merge gates.

The intended steady state on Thor is therefore:

1. scan and triage Memorithm repositories;
2. prioritize failing trusted PR CI;
3. merge eligible exact-head green PRs when repository policy allows;
4. select permitted issue/research work;
5. delegate coding/research inside the sandbox;
6. validate and publish one reviewable PR transaction;
7. observe CI/evidence and continue the loop indefinitely.

`ORCHESTRATOR_MAX_CYCLES=0` remains the unlimited-cycle default.
