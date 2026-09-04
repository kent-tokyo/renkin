# Stock-tier pilot — 10k smoke (Phase B), 2026-08-29

Harness validation run (20 fixed targets, first 20 by `sample_rank` from
`data/comparison/sample_full_sorted.jsonl`), per
`docs/design/...`/`internal_docs/ROADMAP.md`'s v0.36.0 stock-pilot plan.
Purpose: confirm the harness and parameters work before committing to
Phase C's bigger (100-target) run -- **not** a conclusion-drawing
measurement on its own. But it surfaced a real methodological finding
that changes how Phase C's numbers must be interpreted, so it's recorded
here in full rather than only as "harness OK."

## Setup

- Baseline: default `data/building_blocks.smi` (402 compounds).
- Candidate: `data/stock_tiers/tier_10000.imported.smi` (10,000 compounds,
  deterministic hash-rank tier of `data/building_blocks_emolecules_canonical.smi`,
  see `scripts/build_stock_tiers.py`).
- `--spectator-bond-policy gated` fixed identically in both arms (isolates
  the stock variable from the gate variable, per this project's own
  orthogonal-policy-axis discipline).
- `--depth 5 --beam-width 100 --templates data/templates_extracted_500.smi`,
  same for both arms.
- Each arm run via `scripts/compare_run.py --tool renkin --comparison-mode native`
  (existing, unmodified harness -- only `--building-blocks` differs between
  the two invocations). Joined via the new `scripts/stock_tier_paired_report.py`.

## Raw summary

```json
{
  "n_targets": 20,
  "baseline_route_found_rate": {"n_numerator": 3, "value": 0.15},
  "candidate_route_found_rate": {"n_numerator": 0, "value": 0.0},
  "baseline_validator_confirmed_rate": {"n_numerator": 3, "value": 0.15},
  "candidate_validator_confirmed_rate": {"n_numerator": 0, "value": 0.0},
  "baseline_timeout_count": 5,
  "candidate_timeout_count": 2,
  "regression_count": 3,
  "regressions": ["uspto50k_test#L1135", "uspto50k_test#L370", "uspto50k_test#L3773"],
  "new_solve_count": 0,
  "baseline_peak_rss_bytes_max": 40337408,
  "candidate_peak_rss_bytes_max": 66437120,
  "baseline_gated_out_candidate_count_total": 6760,
  "candidate_gated_out_candidate_count_total": 13145
}
```

Full summary at `data/stock_tiers/gate_10k_smoke/summary.json`; raw rows at
`baseline.jsonl`/`candidate.jsonl`.

## Harness validation: PASS

The harness itself worked correctly end-to-end: `compare_run.py` ran both
arms cleanly (0 crashes, 0 invalid-output rows), `stock_tier_paired_report.py`
joined and computed all 4 axes without error, peak RSS scaled sensibly with
stock size (candidate's larger stock costs more memory: 40MB -> 66MB max),
and `gated_out_candidate_count` scaled with stock size too (more compounds
in scope -> more candidates for `SpectatorBondPolicy::Gated` to evaluate
and potentially exclude), all as expected. **Phase C can reuse this exact
harness with the same confidence Phase B was meant to establish.**

## Real finding: the comparison conflates stock SIZE with stock SOURCE

3/3 baseline-solved targets became unsolved under the candidate arm
(`route_found` 3/20 -> 0/20). At face value this looks like "a bigger
stock hurt solve rate" -- but root-causing all 3 shows this is **not** a
stock-size effect at all:

- **`uspto50k_test#L1135`**: baseline's route terminates in phenol
  (`c1(O)ccccc1` / chematic-canonical `c1(O)ccccc1`) among its leaves.
  Confirmed via `renkin stock import` on a throwaway fixture (uses
  RENKIN's own chematic canonicalization, not RDKit -- a naive RDKit-vs-
  chematic string comparison would have given a false negative here)
  that **phenol's canonical form does not appear anywhere in the full
  9,481,986-compound eMolecules corpus**, not just the 10k tier.
- **`uspto50k_test#L370`**: baseline's route terminates in propane
  (`CCC`) and butane (`CCCC`) via a generic aliphatic C-C cleavage.
  **Neither exists anywhere in the eMolecules corpus.**
- **`uspto50k_test#L3773`**: baseline's route terminates in methanesulfonyl
  chloride (`CS(=O)(=O)Cl`) among its leaves. **Absent from the entire
  eMolecules corpus.**

**Since these compounds are missing from the full 9.48M-compound source,
not merely from the 10k-compound slice, no tier size (10k, 100k, 1M, or
even the full corpus) could ever produce these exact routes.** This is a
stock-*identity* gap (RENKIN's small hand-curated stock happens to include
some common simple reagents -- phenol, propane, butane, methanesulfonyl
chloride -- that this particular eMolecules free-tier extract doesn't
carry at all), not evidence that adding compounds reduces solve rate.

Cross-check, for completeness: of the other leaves in these same 3 routes
(glycolic acid, 4-aminobenzonitrile, 4-bromophenol, thiophene-2-boronic
acid, formic acid), all 5 **are** present somewhere in the full corpus --
confirming the corpus isn't uniformly missing "simple" reagents, just
these specific ones. Whether any of those 5 land inside the 10k-compound
tier specifically wasn't checked further (moot given each route's own
independent, corpus-wide-absent blocker).

## Consequence for Phase C

**Any regression Phase C's 100-target run surfaces needs this same
root-cause check before being read as a stock-size finding** -- a
regression could be (a) a genuine "candidate's larger-but-still-bounded
tier doesn't yet include a needed compound that a bigger tier would"
finding (the kind Phase C is actually meant to measure), or (b) another
instance of this same permanent source-identity gap. Conflating the two
would misattribute a source-swap artifact to a size effect.

This was not investigated further here (out of Phase B's own smoke-test
scope) -- flagged for a decision before Phase C, not resolved unilaterally:
whether to (1) run Phase C exactly as specified (402 baseline vs. 100k
tier) with this caveat documented and every regression re-root-caused
individually, or (2) change the candidate arm's definition to `union(402
default stock, tier)` so the candidate is a guaranteed superset of the
baseline (cleanly isolating "does adding compounds ever hurt" -- answer
would then be structurally guaranteed no -- from "does the added scale
help", which is what Item 4's own beam-diversity-style stock question
actually wants to measure).
