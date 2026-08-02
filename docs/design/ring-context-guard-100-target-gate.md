# Ring-Context Safety Guard — 100-Target Gate (Issue #72 / task #242, Phase 5)

Status: **measurement report, PR still draft**. Base commit: `4fc14ad`
(`origin/master`, the commit PR #81 merged into).

This document reports the pre-registered acceptance gate for the
`RingContextPolicy` guard implemented in this branch: the same 100 targets
already published in `data/comparison/results_100/renkin_native.jsonl` (the
Issue #66 comparison sample), run through the same `renkin` binary at four
policy arms, to answer one question before this can go to a 500-target run:
**does the guard change or break anything it shouldn't?**

Raw data: `data/comparison/ring_context_gate_results_100.json` (400 raw
per-target invocations). Report script output:
`data/comparison/ring_context_gate_report_100.txt`. Reproduce with:

```
cargo build --release
python3 scripts/ring_context_gate.py --renkin-binary target/release/renkin \
    --output ring_context_gate_results.json
python3 scripts/ring_context_gate_report.py --input ring_context_gate_results.json
```

## Method

Same 100-target sample, same per-target configuration already checked into
`renkin_native.jsonl` (depth=5, beam-width=100, max-routes=1,
`data/building_blocks.smi`, `data/templates_extracted_500.smi`). Each target
runs at:

- `disabled` — must reproduce current `master` behavior exactly.
- `audit-only` — classifies and diagnoses every match but must return the
  legacy output verbatim (never filters).
- `conservative` — the actual guard.
- `conservative` again (`conservative_repeat`) — same-process determinism
  check.

150s external wall-clock timeout per invocation (the gate harness's own
bound, not a `renkin` CLI limit).

## Headline results

| arm | completed | timeout | routes found |
|---|---|---|---|
| disabled | 100 | 0 | 16/100 |
| audit-only | 99 | 1 | 16/99 completed |
| conservative | 100 | 0 | 15/100 |
| conservative-repeat | 100 | 0 | 15/100 |

### Disabled vs AuditOnly — must be identical by construction

- `route_found` flips: **0**
- `route_signature` flips (same solve state, different route): **0**
- `status` flips: **1** — `uspto50k_test#L1446`: `completed` (122.81s, no
  route) under `disabled`, `timeout` (150.01s) under `audit-only`.

This is the one deviation from a clean zero-diff result, and it is a
**performance artifact, not a correctness regression**: `disabled` and
`conservative` both agree L1446 has no route within budget (122.81s and
130.27s respectively), so nothing was actually found and then lost.
`audit-only` classifies and diagnoses every one of this target's ~6k
ring-checked matches in addition to doing the same work `conservative` does
(apply + element-account every ring-context-accepted match) — on a target
this already close to the timeout, that extra bookkeeping pushed one run
over an arbitrary 150s external cutoff. Every other target's `audit-only`
run reproduced `disabled` exactly. Conclusion: `AuditOnly`'s
diagnose-without-filtering guarantee holds everywhere it got to finish;
`AuditOnly` is not intended for latency-sensitive production use (it is a
diagnostic sweep mode) and this is the expected cost profile for that.

### Disabled vs Conservative

- `status` flips: 0
- `route_found` flips: **1**
- `route_signature` flips (still solved, different route): **3**

**`route_found` flip — `uspto50k_test#L984` (`True → False`):**

This is the real-data counterpart of the `extracted_9` synthetic regression
test (`src/ring_context.rs`'s
`extracted_9_conservative_rejects_isoindolinone_ring_opening`). Confirmed by
direct inspection, not inference:

- L984's target SMILES
  (`Cc1ccc(C(=O)Nc2ccc(-c3ccc4c(c3)CN([C@H](C(=O)O)C(C)C)C4=O)cc2)cc1`)
  contains an isoindolinone (benzene-fused γ-lactam) scaffold.
- The currently-published `renkin_native.jsonl` entry for this target
  already carries `"target_element_accounting_status":
  "unaccounted_target_element"` and
  `"common_validation_warnings": ["unaccounted_target_element", ...]` — i.e.
  the route this branch's `Conservative` policy now rejects was **already
  flagged as chemically wrong** in currently-published data, not a route
  this change newly breaks.
- The template driving it is line 14 of `data/templates_extracted_500.smi`
  (`[C:4]-[N:5](-[C:1](=[O:2])-[c:3])-[C:6]>>O-[C:1](=[O:2])-[c:3].[C:4]-[NH:5]-[C:6]`,
  weight 231) — byte-identical SMIRKS and weight to the `extracted_9` unit
  test fixture. This template's training data was 235/235 non-ring
  occurrences (0 ring observations), yet it pattern-matches the
  isoindolinone's ring N–C(=O) bond here; disconnecting it opens the lactam
  ring, exactly the failure class #72 describes.
- Under `audit-only`, this target still returns the same route as
  `disabled` (architectural guarantee holds), but its diagnostics already
  show `ring_rejects_nonring_intent_on_ring_bond: 180` for this target —
  i.e. it flags the match `conservative` will reject, without rejecting it.
- Under `conservative`, the unsafe match is rejected and no alternate valid
  route is found within depth 5 / beam 100 / max-routes 1 — `route_found`
  goes to `False`. This is the conservative trade-off working as designed:
  a known-wrong route is removed, at the cost of not finding a replacement
  within this search budget. No other route existed to fall back to; this
  is not a search regression, it's the guard doing its job on a target
  where the *only* route the legacy search had found was the unsafe one.

**`route_signature` flips (still solved, route changed) — three targets:**

| target | disabled sig | conservative sig | conservative `ring_rejects_nonring_intent_on_ring_bond` |
|---|---|---|---|
| L3857 | `f3c9056bab85ffa3` | `f9a470c5ce63e4bc` | 14 |
| L4464 | `f9c58de1aa75ef83` | `318a39da3672f52e` | 198 |
| L4575 | `69d0a25a4ab99a73` | `2a3cf3a37287f75d` | 292 |

All three remain solved under `conservative`; each has a nonzero ring-reject
count, consistent with: the legacy route depended on at least one
ring-context-unsafe match, that match got rejected, and the search found a
different, still-valid route within the same budget. `audit-only`
reproduces the `disabled` signature exactly for all three (architectural
guarantee holds), confirming these are genuine `conservative`-only route
changes, not search nondeterminism.

### Conservative vs Conservative-repeat — determinism

- `status` flips: 0, `route_found` flips: 0, `route_signature` flips: 0.

Zero diffs across all 100 targets, run in two separate process invocations.
The guard is deterministic.

### Aggregate `ring_context_diagnostics`

| counter | audit-only (99 completed) | conservative (100 completed) |
|---|---|---|
| matches_enumerated | 444,381 | 464,452 |
| matches_ring_checked | 235,836 | 248,311 |
| matches_applied (ring-context-accepted) | 422,065 | 440,666 |
| outcomes_accepted | 421,821 | 440,076 |
| outcomes_element_rejected | 244 | 590 |
| ring_rejects_nonring_intent_on_ring_bond | 22,316 | 23,786 |
| ring_rejects_ring_intent_on_nonring_bond | 0 | 0 |
| ring_rejects_unknown_intent_on_ring_bond | 0 | 0 |
| invalid_mapped_bond | 0 | 0 |
| templates_missing_metadata | 0 | 0 |

`audit-only`'s totals are lower purely because L1446 (the one timeout)
contributes zero diagnostics to that column; every other target's counters
are identical between the two arms by construction (both classify every
match the same way — `audit-only` just never acts on the verdict).

No `ring_rejects_ring_intent_on_nonring_bond` or
`ring_rejects_unknown_intent_on_ring_bond` fired on this sample — this
100-target sample doesn't happen to exercise those two reject reasons; the
dedicated unit tests (`src/ring_context.rs`) are the coverage for them, not
this measurement.

### Latency

| arm | n | total | p50 | p95 | max |
|---|---|---|---|---|---|
| disabled | 100 | 1458.3s | 6.90s | 64.95s | 122.81s |
| audit-only | 100 | 1837.1s | 10.17s | 69.42s | 150.01s (timeout) |
| conservative | 100 | 1431.3s | 6.91s | 60.00s | 130.27s |

`conservative` is not slower than `disabled` in aggregate (rejecting unsafe
matches early skips otherwise-wasted `apply_reaction_match` work on some
targets, offsetting the added classification cost). `audit-only` carries a
real ~26% total-time overhead from unconditional match classification and
diagnostics bookkeeping on every match — expected for a diagnostics-only
mode, and the direct cause of the one L1446 timeout above.

## Conclusion

- `Disabled` ≡ current `master` (unaffected; this policy is the default).
- `AuditOnly` returns byte-identical output to `Disabled` on every target
  that completes within the harness's external timeout; the guard's
  diagnose-without-filter invariant holds with no exceptions observed.
- `Conservative` changes exactly 4/100 targets, every one of them explained:
  one known-wrong route removed (the real-world `extracted_9`/L984 case,
  already flagged as chemically wrong in currently-published data) with no
  budget left over to find a replacement, and three routes swapped for a
  different still-valid route after an unsafe match was rejected.
  `Conservative` is deterministic across repeated runs.
- No unexplained route loss. Per the Phase 5 acceptance criterion, this
  clears the bar to report and stop here; the 500-target run is explicitly
  out of scope for this PR.
