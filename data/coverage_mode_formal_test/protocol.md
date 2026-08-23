# v0.24 Coverage Mode — Formal-TEST Confirmation Protocol

**Status: PRE-REGISTERED, FROZEN. Not yet executed.** User-authored,
verbatim, 2026-08-15 (in response to a question about whether a
formal-TEST protocol for coverage mode already existed as a committed
artifact — it did not; Phase B.1's 10-step pre-registration only went
through Step 9, reranker-ON compatibility, on VAL, and Phase B.2
branched off with its own separate GO after Phase B.1 closed as a
negative result. Neither ever defined a formal-TEST-scale protocol for
the coverage-mode arms).

This document is the "formal protocol" artifact referenced by the
post-TEST immutability policy in the v0.24.0 release-candidate PR: once
`RELEASE_CANDIDATE_SHA` is frozen, this file may not change (only the
run's own results/report may be added elsewhere). Committing this
document does **not** authorize running the formal TEST — that
requires a separate, later, explicit GO once implementation, CI, and
independent review are all complete and the candidate SHA is frozen.

## 0. Why this protocol exists, and why it is not just a copy of the VAL gate

The VAL reranker-compatibility gate (`data/phase_b1_frontier/findings.md`,
"RERANKER COMPATIBILITY GATE") used Arm A (500 templates, reranker ON)
vs. Arm C (500→2,000 coverage mode, reranker ON) over a 100-target VAL
sample, with a `determinism exact` criterion verified by running each
arm twice and diffing a semantic projection. That gate's Arm
configuration is reused here unchanged. Its threshold set is **not**
copied unchanged — two things differ enough at formal-TEST scale to
require re-registering the pass/fail rule rather than reusing it
verbatim:

- **Scale**: 4,903 possible targets vs. 100. Running Stage 2 (opt-in,
  `p95 5.72x` cost) over the full corpus at an 83.5%-observed
  invocation rate is not proportionate for a single confirmation run —
  hence the 500-target cohort below, not the full corpus.
- **`determinism exact` does not extend to formal TEST as a hard gate.**
  The VAL gate's own extended-replay result
  (`uspto50k_val#L2330`, `data/phase_b1_frontier/findings.md`,
  "RERANKER COMPATIBILITY GATE") already established that Stage-2
  wall-clock timeout *classification* near the deadline is
  load-sensitive by construction — a real, accepted product property
  (`docs/design/coverage-mode-v0.md` states this as an explicit
  two-layer contract: algorithmic semantic determinism vs. operational
  timeout-classification variability), not a bug. Running formal TEST
  twice specifically to demand exact timeout-classification agreement
  would re-litigate a question this program has already answered, and
  would do so by spending the one-shot formal-TEST resource on a
  question VAL + the L2330 uncensored diagnostic (Phase B.2d,
  3/3 exact) already settled. Algorithmic determinism is retained as a
  gate criterion below, in a form this run *can* actually check without
  a second full pass: Arm A's solved targets must reproduce identically
  in Arm C's Stage 1 (same rules, same reranker, same config — the only
  new variable Stage 2 introduces is escalation for Arm-A-unsolved
  targets, which by construction never touches an Arm-A-solved target's
  result).

## 1. Cohort

- **Source corpus**: `data/reranker_groups_uspto50k_test.jsonl` (4,903
  groups — the same denominator Task 35's formal candidate-ranking gate
  used, `data/phase3e_reranker_training/findings.md`), not the raw
  4,907-row `data/uspto50k_test.smi` directly.
- **Selection rule**: deterministic hash order.
  `sample_key = SHA256("renkin-v024-coverage-formal-test-cohort-v1|" +
  canonical_smiles)`, RDKit canonicalization, ascending sort by
  `(sample_key, canonical_smiles)`, first 500 taken as a prefix.
  Implemented in `scripts/select_coverage_mode_formal_test_cohort.py`
  (tests: `scripts/tests/test_select_coverage_mode_formal_test_cohort.py`).
  A distinct protocol-version string from `scripts/compare_sampling.py`'s
  Issue #66 sampling — same technique, unrelated hash namespace, despite
  both drawing on the same underlying TEST corpus.
- **Committed manifest**: `data/coverage_mode_formal_test/cohort_manifest.json`
  — 500 target IDs, canonical SMILES, sample keys, source-corpus SHA-256,
  committed in the same commit as this protocol document, **before any
  search has been run against any of these targets** (selection is a
  pure function of corpus content, not of any observed route-search
  outcome).
- **`cohort_targets_sha256`**: `bf68169e6a1eccfc...` (see the manifest
  file for the full value) — pin for this exact 500-target set.
- The remaining 4,403 groups are **not used** by this confirmation run.

## 2. Arms

- **Arm A (baseline, standard mode)**: 500 templates
  (`data/phase_a5_template_scaling/templates/templates_500.smi`),
  reranker ON.
- **Arm C (coverage mode)**:
  - Stage 1: identical 500 templates, identical reranker — same rule
    set and config as Arm A.
  - Stage 2 (only for Arm-A-unsolved targets, per coverage mode's own
    structural guarantee that Stage 1's result is never overwritten):
    full 2,000 templates
    (`data/phase_a5_template_scaling/templates/templates_2000.smi`),
    same reranker, `--coverage-timeout-secs 600`.

## 3. Common configuration

- Same compiled `renkin` binary for both arms (the frozen
  `RELEASE_CANDIDATE_SHA` release build) — invoked via **the actual
  shipped `--search-mode coverage` / `--coverage-templates` /
  `--coverage-timeout-secs` CLI surface (Phase 41.18B, PR #120)**, not
  the earlier VAL-gate's two-phase `scripts/compare_run.py`
  Stage-1-then-filter-then-Stage-2 orchestration script. **This is an
  implementation-level decision made without a separate user
  confirmation round** (the pre-registered scientific parameters —
  cohort, arms, thresholds — came from the user verbatim; the choice of
  *which tool invokes them* did not). Rationale: the VAL gate predates
  Phase 41.18B, so it necessarily reimplemented the staged Stage-1/
  Stage-2 escalation at the benchmark-script level. That product code
  now exists and shipped in PR #120 specifically so CLI/Python callers
  don't have to reimplement it — running the formal confirmation
  through the actual product surface tests the thing v0.24.0 ships,
  not a parallel research-harness stand-in for it. If this call should
  be reverted to reusing `compare_run.py`'s original two-phase
  orchestration instead, say so before the run starts.
- `--depth 5 --max-routes 1 --beam-width 100` (matches the VAL gate).
- Stock: `data/comparison/shared_stock/shared_stock.smi` (393 compounds,
  `shared_stock` comparison mode — matches the VAL gate).
- Reranker: `data/phase3e_reranker_training/model.txt` +
  `frequency_table.json` (the same frozen Task 35 artifacts already used
  throughout this program).
- One run per arm per target. No retry. No failed-row rerun. No
  threshold adjustment after any result is observed.
- Per-target and aggregate results captured via a formal-TEST-scale
  extension of `scripts/compare_run.py`'s existing structured-output/
  `--resume`/environment-capture harness, adapted to invoke the native
  `--search-mode coverage` CLI for Arm C rather than orchestrating two
  separate calls — this harness extension is written and reviewed
  before the run, as part of the same release-candidate PR, and is
  itself subject to the post-TEST immutability policy (§ "search
  configuration" / "benchmark runner").

## 4. Pass/fail criteria

**Primary efficacy**:
- Route coverage delta (Arm C solved − Arm A solved) ≥ **+3.0pp**
  (n=500 ⇒ net gain of at least **+15** solved targets).

**Structural safety**:
- Regressions (Arm-A-solved, Arm-C-unsolved) = **0**.
- Invalid/unparseable results = **0**.
- `reranker_failures` = **0** in either arm.
- Every Arm-A-solved target: Arm C's `selected_stage` is `stage1` and
  the semantic route result (route found / shape / validator outcome)
  is exactly identical to Arm A's — algorithmic-determinism check, not
  a second full run (see §0).
- Every target where Arm C's `selected_stage` is `stage1`: `stage2_invoked
  = false` (Stage 2 never runs when Stage 1 already solved — same
  structural guarantee `coverage_mode::tests::stage1_solved_never_
  invokes_stage2` proves at the unit level, now checked at corpus scale).

**Operational reporting (informational, not a hard gate)**:
- Stage 2 invocation rate.
- Stage 2 timeout count/rate.
- p50/p95 elapsed (per arm).
- Total wall-clock, total additional compute per newly-solved target.

**Operational release blocker (hard gate, pre-fixed to avoid a
post-hoc definition of "extreme")**:
- Stage 2 timeout rate ≤ **5%**. Above that, even a coverage-delta PASS
  does not make the release candidate Ready — timeouts are reported
  either way, but a >5% rate means the operational cost characterized
  by this run doesn't match what was measured at VAL scale, and that
  gap needs investigation before shipping, not overriding.

**Explicitly not a gate for this run**: bitwise/timing determinism
across repeated full arm runs (see §0's rationale).

## 5. PASS / FAIL handling

- **PASS**: commit raw per-target results, aggregate, this cohort
  manifest, all relevant SHA-256 hashes, the exact command line used,
  environment capture, and the PASS verdict. Release candidate may
  proceed toward Ready.
- **FAIL** (any hard-gate criterion in §4 not met): record as FAIL,
  as-is. Release candidate does **not** become Ready. No change to
  algorithm, config, or threshold in response to a FAIL. No rerun.
  Report and STOP — return to the user for the next decision.
- **Execution anomaly** (crash, environment kill, infrastructure
  failure — distinct from a completed run with a failing verdict):
  STOP immediately, do not auto-resume even if per-chunk checkpointing
  makes resuming technically possible, save and report exactly how far
  execution got, and wait for explicit authorization before continuing.
  An anomaly is not a result and must never be reported or treated as
  one.

## 6. Report contents (either outcome)

Gains/regressions detail, coverage delta, Stage 2 invocation/timeout
rate, latency p50/p95, total compute, additional compute per new solve
— per §4's operational-reporting list, plus the PASS/FAIL verdict
itself and its supporting raw data locations.
