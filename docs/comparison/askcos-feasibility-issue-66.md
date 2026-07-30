# ASKCOS reproducibility feasibility (Issue #66)

This is a **feasibility classification only** — no ASKCOS adapter exists in
this comparison, and no ASKCOS run was attempted. Per the frozen protocol
(see [`open-source-retrosynthesis-comparison.md`](../guides/open-source-retrosynthesis-comparison.md)),
a tool is only executed in the 100-target feasibility round if it classifies
as `reproducible_now`. ASKCOS does not.

## Classification: `reproducible_with_manual_setup`

ASKCOSv2's source code, deployment tooling, and five of six one-step
retrosynthesis model checkpoints are publicly available with no login wall.
But the model most commonly cited as ASKCOS's representative one-step model
(a Reaxys-trained template-relevance checkpoint) is licensed CC BY-NC 4.0
(non-commercial), and its Monte Carlo tree search is a wall-clock-bounded
stochastic process with no documented seed control. Running it requires a
human to (a) knowingly accept a non-commercial-use license for at least one
model variant used in any "typical settings" run, and (b) decide how to
account for non-reproducible-by-construction search behavior. Neither of
those is something this harness can resolve or authorize on its own — which
is exactly what keeps this out of `reproducible_now`. It stays out of
`not_reproducible_from_public_artifacts` because the code, the deployment
procedure, and most of the models genuinely are public without a commercial
license or membership.

## What is public

- **Current canonical repository**: `gitlab.com/mlpds_mit/askcosv2`, entry
  point `askcosv2/askcos2_core`. Confirmed fetchable unauthenticated (raw
  file contents retrieved without a token). The legacy `github.com/ASKCOS/ASKCOS`
  repository is explicitly archived ("will no longer be updated"; final
  release v0.4.1) — not the basis for any feasibility judgment here.
- **Code license**: v2 is MIT. (v1's code was MPL 2.0; v1's data/models were
  CC BY-NC-SA 4.0 — superseded by v2's per-repo model distribution below.)
- **One-step model checkpoints**: five of six template-relevance checkpoints
  (`cas`, `pistachio`, `pistachio_ringbreaker`, `bkms_metabolic`,
  `reaxys_biocatalysis`) are documented MIT-licensed and downloadable via a
  script in the `template_relevance` repository with no login/account/token
  requirement mentioned. The sixth, `reaxys`, is explicitly CC BY-NC 4.0.
- **Batch API**: a documented REST API (FastAPI gateway) including a
  tree-search endpoint, interactive API docs, and example batch scripts
  (including a documented "run on 100 test molecules" example) — this is a
  genuine strength; ASKCOS's programmatic interface is better-documented
  than its licensing story.
- **CPU feasibility**: documented as supported for inference (GPU is stated
  as required only for model retraining); separate CPU-only images/services
  exist with an identical API surface.

## What blocks `reproducible_now`

1. **License ambiguity on the most-cited model.** The Reaxys-trained
   checkpoint — the one most papers treat as "the" ASKCOS one-step model —
   is CC BY-NC 4.0. Per this project's own rule (no models with unclear or
   restrictive license terms used in the executable comparison), using it
   would require a human to explicitly accept non-commercial terms; this
   harness will not make that acceptance on anyone's behalf, and will not
   substitute a different "representative" model to route around the
   restriction, since that changes what's actually being measured.
2. **An unresolved contradiction on the `cas` checkpoint specifically.** The
   official ASKCOS paper (arXiv:2501.01835) states models trained on the CAS
   Content Collection are "only accessible to MLPDS members," while the
   `template_relevance` repository's own README labels the `cas` checkpoint
   MIT with no stated access gate. These two public sources disagree, and
   this audit did not attempt to resolve the discrepancy by downloading the
   checkpoint (out of scope for a read-only feasibility pass) — it is
   reported as an open question, not silently resolved either way.
2. **Deployment is a full multi-service stack, not a CLI tool.** ASKCOSv2 is
   a FastAPI gateway plus separate prediction-service containers, a
   database, and (per v1-era docs, presumed to still apply architecturally)
   task-queue infrastructure — deployed via `make deploy`. This is
   documented and scriptable, so it is not by itself disqualifying, but it
   is real operational weight compared to RENKIN's single binary or
   AiZynthFinder's single-process CLI.
3. **x86-only.** ASKCOSv2's documentation states Apple Silicon/ARM is not
   currently supported. This machine is arm64 — running ASKCOS here would
   require an x86_64 Docker host or emulation, which was not attempted.
4. **No documented seed/determinism control.** The MCTS tree search is
   bounded by wall-clock time (the docs' own example cites "10 seconds, the
   time limit set for this sample query"), not an iteration/expansion count.
   A time-bounded stochastic search is not reproducible run-to-run by
   construction, and result variance would depend on machine load in a way
   this project's fixed-timeout protocol cannot fully control for. Whether
   an expansion-count-based (machine-independent) alternative exists was not
   resolved by this audit.

## Why this is not `not_reproducible_from_public_artifacts`

The code is public and MIT-licensed. The deployment procedure is public,
scripted, and (for the current v2 path) does not require an authentication
token to begin — a materially different situation from an unmaintained or
fully access-gated project. Most of the model checkpoints are public and
MIT-licensed with no account required. What's missing is not public
availability in general — it's an unambiguous, simultaneously-satisfied
{model, data, license, procedure} story across the *entire* stack, plus a
resolution to the CAS-checkpoint license contradiction above.

## What would need to happen before a future round could run ASKCOS

- A human explicitly chooses and documents which one-step model checkpoint
  to use, accepting that checkpoint's specific license (e.g., using only
  the five MIT-licensed checkpoints and excluding `reaxys` entirely, or
  explicitly accepting CC BY-NC 4.0 terms and disclosing that in any
  published comparison).
- The `cas` checkpoint's licensing contradiction (MIT per repo README vs.
  "MLPDS members only" per the published paper) is resolved directly with
  the maintainers before that specific checkpoint is used for anything
  beyond code inspection.
- An x86_64 host (or accepted emulation, with its own performance caveats)
  is provisioned, since this comparison's other measurements ran on native
  Apple Silicon arm64.
- A methodology decision is made for ASKCOS's wall-clock-bounded, seedless
  MCTS search — e.g., running each target multiple times and reporting
  variance, rather than treating a single run as representative the way
  RENKIN's deterministic search allows.

None of this is attempted in this round. This document is the complete
ASKCOS deliverable for Issue #66's 100-target feasibility pass.
