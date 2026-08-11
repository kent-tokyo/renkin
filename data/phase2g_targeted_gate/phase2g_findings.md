# Phase 2G: targeted gate on the 6 named beam-sensitive targets

Source: `renkin --open-state-dominance` (commit `8259cf6`, worktree
`renkin-open-state-dominance`, branch `feat/open-state-dominance-101`).
Conservative ring-context policy, `data/comparison/shared_stock/shared_stock.smi`,
depth=5, `data/templates_extracted_500.smi`. Sequential execution (one target
at a time), matching the Phase 1C sequential-run lesson. Baseline = the
existing `data/phase1c_diagnostics/beam{100,200,300}.jsonl` (candidate flag
off, binary `5d03554`) -- reused rather than re-run, since
`open_state_dominance_default_off_matches_legacy_behaviour_exactly` (unit
test) and a direct real-binary repeat-run check (this session) both confirm
the flag-off path is byte-identical to unmodified master.

## Aggregate result: route_found, beam=100 (the official gate beam)

| target | baseline | candidate | change |
|---|---|---|---|
| L1541 | False | False | no change |
| L984  | False | False | no change |
| L1640 | False | **True** | **newly solved** |
| L4092 | False | **True** | **newly solved** |
| L4259 | True  | True | no change (control) |
| L1167 | True  | True | no change (control) |

**At the official beam=100: +2 newly solved (L1640, L4092), 0 regressions
among these 6.** `dominated_skipped`/`better_replacements` counters are
nonzero for every target including the two always-solved controls (L4259:
855/372, L1167: 305/92), confirming the mechanism engages broadly, not just
on the four beam-sensitive targets -- but only changes the *outcome* for
L1640/L4092 at this beam width.

## beam=200/300 (sensitivity-only per the Phase 2G spec, not gating)

| target | beam | baseline | candidate |
|---|---|---|---|
| L1541 | 200 | True | **False (regression)** |
| L1541 | 300 | False | True |
| L984  | 200/300 | True/True | True/True (no change) |
| L1640 | 200/300 | False/True | True/True (200 also newly solved) |
| L4092 | 200/300 | True/True | True/True (no change) |
| L4259 | all | True | True (no change) |
| L1167 | all | True | True (no change) |

**L1541 shows a genuine regression at beam=200** (baseline solved, candidate
does not) -- this is a real, honest finding, not hidden. Per the Phase 2G
spec beam=200/300 are informational/sensitivity-only and do not gate this
round, but it is reported here in full since it bears on whether the
mechanism might introduce regressions at other beam widths not covered by
the 100-target formal gate (which is beam=100 only, per Phase 1E's
pre-registered protocol).

## L1541 deep dive (candidate-trace, beam=100, `--candidate-trace-limit 30000`)

`open_state_candidates_considered: 16282`, full trace (12545 records, not
truncated by the cap).

- **The depth-5 `suzuki_retro` terminal step, documented in
  `data/phase1c_diagnostics/L1541_candidate_narrative.md` as "never
  generated at all in the beam=100 run" under the pre-Phase-2 binary, is now
  generated 174 times at beam=100** with `open_state_dominance` on --
  confirming the mechanism measurably changes what gets explored, in the
  predicted direction.
- **None of the 174 survive the beam (`survived_beam: false` for all 174),
  and none reach stock (`later_reached_stock: false` for all 174).** L1541
  stays unsolved at beam=100 even though the previously entirely-absent
  terminal step now gets generated -- generation alone was not sufficient at
  this beam width for this target.
- `rules_attempted_total` rose from 99,264 (baseline) to 110,352 (candidate)
  at beam=100 -- consistent with freed beam capacity going toward more
  distinct exploration.

## rules_attempted_total, all 6 targets x 3 beams (baseline vs candidate)

| target | beam=100 | beam=200 | beam=300 |
|---|---|---|---|
| L1541 | 99,264 → 110,352 | 145,728 → 191,664 | 209,088 → 243,936 |
| L984  | 112,464 → 126,720 | 159,456 → 182,688 | 213,840 → 275,088 |
| L1640 | 41,712 → 63,888 | 80,256 → 82,368 | 99,792 → 90,288 |
| L4092 | 70,752 → 80,256 | 106,656 → 133,584 | 143,088 → 187,440 |
| L4259 | 21,648 → 21,648 (unchanged) | unchanged | unchanged |
| L1167 | 12,672 → 12,672 (unchanged) | unchanged | unchanged |

The two always-solved controls (L4259, L1167) show **zero change** in
`rules_attempted_total` at any beam width despite nonzero
dominated/replaced counts -- their search trees are small enough that
duplicate-state crowd-out never becomes a binding constraint, so dominance
has no observable effect on them. This is the expected, reassuring null
result for the control group.

## Conclusion / next step

The official beam=100 result (+2, 0 regressions among these 6) and the
L1541 depth-5 generation finding both support proceeding to Phase 2H (the
100-target formal gate, beam=100, sequential), per the pre-registered
protocol. The beam=200 regression on L1541 is noted for the record but does
not block Phase 2H, since Phase 1E's acceptance gate is defined at beam=100
only.
