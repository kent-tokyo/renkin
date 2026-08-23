# SpectatorBondLoss smoke measurement — 2026-08-24

**Status: lightweight smoke evidence only, not a formal corpus-wide
measurement.** This run demonstrates `SearchConfig::spectator_bond_diagnostics`
(PR #186; renamed to the `spectator_bond_policy: SpectatorBondPolicy`
enum's `DiagnosticsOnly` variant in PR #188's fail-closed gating work,
which landed after this run -- same behavior, current field name) against
15 real targets and produces a real, reproducible data point, but the
sample is small, right-censored by a 90-second per-target timeout, and
must not be generalized to "N% of the 5,000-template corpus is defective"
or any broader population claim.

## Purpose

v0.35.0 "Template Integrity & Spectator Bond Loss" rollout plan's smoke-test
stage: after Case A + Case B detectors (PR #186) and diagnostic-only wiring
were merged, measure their real-world behavior on a small target sample
before considering fail-closed gating — record excluded-candidate counts and
whether any *found* route would actually be affected, without running a
heavy 300/4,907-target remeasurement.

## Environment / provenance

| | |
|---|---|
| renkin commit (binary under test) | `a2670e8` (branch `feat/spectator-bond-target-smiles`, PR #187) |
| build | `cargo build --release --example spectator_bond_smoke` |
| OS / CPU | macOS (Darwin 25.5.0, arm64) |
| depth | 5 |
| beam-width | 100 |
| max-routes | 1 |
| per-target timeout | 90 seconds (`SearchControl::with_timeout`, fresh per target) |
| rules | `default_rules()` + `data/templates_extracted_5000.smi` |
| building blocks | `data/building_blocks.smi` |
| target sample | **first 15 lines** of `data/finding4_pilot_2026-08-23/target_sample_n300_seed42.smi` (Finding #4 pilot's own n=300, `random.seed(42)` from `data/uspto50k_test.smi`'s 4,907 targets) — reused rather than drawing a fresh sample, since that sample's own ordering is already randomized |
| script | `examples/spectator_bond_smoke.rs` (committed) |

## Command

```
./target/release/examples/spectator_bond_smoke > run.out 2> run.log
```

## Results (n=15, exact)

| Outcome | Count |
|---|---:|
| ROUTE found | 5 |
| TIMEOUT (no route before 90s) | 7 |
| UNSOLVED (search exhausted, no route) | 3 |

| | Count |
|---|---:|
| Targets with ≥1 SpectatorBondLoss finding | 11 / 15 |
| Total findings (across the whole search, all intermediates) | 19,606 |
| — Case A (`MatchedPairUndeclared`) | 893 |
| — Case B (`CrossProductTerritory`) | 18,713 |
| Distinct rules that produced ≥1 finding | 276 |
| Targets whose *returned* route used a flagged `(rule, target)` step | 1 / 5 solved |

Raw output: `data/spectator_bond_smoke_2026-08-24/run.out` (gitignored, local
only, 7.5MB) — sha256 `505dcff1bfe1a7d0cedbe3633c9a3fc166297be2a3f2bbc0024ede4db4b06e0d`.

## The one route-impacting case, hand-verified

Target `[CH3][CH2][CH2][CH2][CH2][CH2][CH2]OC(=O)[NH][C@H]1C(=O)O[C@H]1[CH3]`
(a heptyl-carbamate-protected methyl-β-lactone) found a depth-4 route whose
first step used `extracted_288`:

```
extracted_288: [C@H]1(OC(=O)[C@@H]1NC(=O)OCCCCCCC)C
  -> C(C(O)=O)NC(OCCCCCCC)=O  +  C(C)O
```

Flagged Case A (1 undeclared bond between matched atoms). Manually traced:
the target is a 4-membered β-lactone ring bearing a methyl substituent on
one ring carbon and an N-heptylcarbamate on the adjacent one. The rule's own
declared precursors are (1) a *generic, unsubstituted* glycine-heptylcarbamate
fragment and (2) plain ethanol — the methyl substituent on the ring's
alcohol-forming carbon vanishes entirely, and the two "separate" precursor
fragments are actually the two halves of one real ring, mirroring the exact
same ring-tether-blindness pattern already confirmed on `extracted_112`/
`extracted_824`/`extracted_109`/`extracted_4255` (Finding #4). This is a
**genuine, previously-unknown template defect**, found on a real search
trace, not a fixture constructed to exercise the detector — first evidence
the detectors generalize beyond the 4 known instances.

## Spot-checks on the high-volume findings

The bulk of findings concentrate in 4 of the 15 targets (each with
hundreds-to-thousands of findings). Sampled several from the two
highest-volume targets:

- Many *different* rules (`extracted_11`, `extracted_20`, `extracted_22`,
  `extracted_36`, ... — dozens more) all flag the **same** complex fused-ring
  target with near-identical "matched atoms 2/4/5" signatures. Consistent
  with many templates in the corpus sharing a similarly narrow, ring-context-
  blind LHS shape, all tripping over the same real fused-ring structure —
  not an obvious detector bug, but not exhaustively verified per-instance
  either (see caveats below).
- Case A findings span 152 distinct targets (not concentrated on one
  molecule), including another β-lactone/epoxide-shaped target hit by
  multiple different rules (`extracted_4025`, and others) in the same shape
  as the hand-verified case above.

Full per-instance verification of all 19,606 findings was not attempted —
out of scope for a lightweight smoke test.

## Caveats — do not overgeneralize

- **n=15, not n=300 or n=4,907.** No claim about the fraction of the 5,000-
  template corpus that is defective can be made from this sample.
- **Right-censored by the 90s timeout**, same caveat as the Finding #4 pilot
  (`docs/validation/finding4-validator-pilot-2026-08-23.md`) — the search
  trees explored for the 5 solved + 3 unsolved targets are not representative
  of what a full/unbounded search would explore for the 7 timed-out targets.
- **Diagnostic-only**: no candidate was actually excluded by this run: the 1
  route-impacting finding describes what a *future* fail-closed gate would
  touch, not something that already changed.
- The 276-distinct-rules and 19,606-findings figures are almost certainly
  dominated by a handful of structurally "magnet" targets (complex fused-ring
  systems) that happen to trip many similarly-shaped templates at once, not
  evidence that 276 templates are independently, individually broken in
  276 different ways.

## Next steps (tracked outside this document)

1. Whether to expand this into a real corpus-wide scan of the 5,000
   extracted templates (the release plan's own table allows deferring this
   until after v0.35.0 ships) — explicit user decision, not made here.
2. Fail-closed gating design (excluding only "confident" findings, `not_
   evaluable`/warning for ambiguous ones) — this smoke test's one hand-
   verified positive case is a real candidate for that design's own
   acceptance fixture set, alongside the 4 Finding #4 instances.
3. No further heavy remeasurement planned at this stage.
