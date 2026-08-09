# Ring-Context Safety Guard — 100-Target Gate (Issue #72 / task #242, Phase 5)

Status: **measurement report, PR still draft**. Base commit: `4fc14ad`
(`origin/master`, the commit PR #81 merged into).

This document reports the pre-registered acceptance gate for the
`RingContextConfig` guard implemented in this branch: the same 100 targets
already published in `data/comparison/results_100/renkin_native.jsonl` (the
Issue #66 comparison sample), run through the same `renkin` binary at six
arms, to answer one question before this can go to a 500-target run: **does
the guard change or break anything it shouldn't?**

Raw data: `data/comparison/ring_context_gate_results_100.json` (600 raw
per-target invocations, native stock). Report script output:
`data/comparison/ring_context_gate_report_100.txt`. A shared-stock
confirmatory check (§ Shared-stock confirmation) adds
`data/comparison/ring_context_gate_results_shared_stock_100.json` (300 more
invocations) and `..._report_shared_stock_100.txt`. Reproduce with:

```
python3 scripts/generate_ring_context_metadata.py \
    --dataset-revision 08a575f0546b2be57242997fd45f684d6814d5a9   # regenerate the sidecar
cargo build --release
python3 scripts/ring_context_gate.py --renkin-binary target/release/renkin \
    --output ring_context_gate_results.json
python3 scripts/ring_context_gate_report.py --input ring_context_gate_results.json
```

`--dataset-revision` is shown explicitly above for clarity, but this exact
SHA is also `generate_ring_context_metadata.py`'s `PINNED_DATASET_REVISION`
default — omitting the flag reproduces the same corpus. `--resolve-latest`
opts into a dynamic HEAD resolution instead, for a deliberate re-baseline
against upstream drift (never used for reproducing this checked-in
artifact).

## Methodology correction (this revision)

An earlier version of this gate ran against a sidecar whose generator
(`scripts/generate_ring_context_metadata.py`) had a real attribution bug:
it counted **every** raw substructure match of a template's LHS pattern
against a historical product as an "observation," including matches at
sites the reaction never actually touched (the template's pattern can
recur elsewhere in the same molecule by coincidence). This measurably
inflated `Either` classifications — a template genuinely non-ring at every
real reaction center could still pick up spurious ring observations from
incidental matches elsewhere, silently making `Conservative` more
permissive than the true chemistry warrants.

**Fix**: the generator now independently re-derives each historical
reaction's actual formed/deleted bond set directly from the dataset's
atom-mapped reactant/product SMILES (`product_bonds - reactant_bonds`,
keyed on the dataset's own atom-map numbers — not rdchiral's per-template
renumbering), and only counts a raw match as an observation if it lands on
that real bond. Incidental matches are excluded and counted separately
(`incidental_matches_excluded`) for transparency. See
`scripts/generate_ring_context_metadata.py`'s module docstring and
`attribute_bucket` for the exact algorithm, and `scripts/tests/
test_generate_ring_context_metadata.py` for unit coverage of the
genuine-vs-incidental distinction on synthetic data.

**Regeneration result** (full 40,008-reaction USPTO-50k pass,
`bisectgroup/USPTO_50K@08a575f0546b2be57242997fd45f684d6814d5a9`, pinned):

| | old (buggy) sidecar | new (fixed) sidecar |
|---|---|---|
| `either` | 62 | **18** |
| `non_ring` | 335 | **379** |
| `ring` | 8 | 8 |
| `unknown` | 0 | 0 |

All 44 changed bonds that flipped moved **`either` → `non_ring`** — zero
flipped to `ring` or `unknown`. This is exactly the expected direction: the
fix removes false permissiveness (bogus `either`), it doesn't manufacture
new uncertainty. The generator's own invariant assertion (every changed
bond's `ring+non_ring+ambiguous+unknown` observations exactly equal to that
template's `source_occurrences_matched`) passed with zero violations,
independently re-verified outside the generator too. Of 16,855 raw matches
processed, 2,357 (14%) were incidental and correctly excluded; 14,498 were
genuine.

**Anchor check**: `extracted_9` (line 14 of `data/templates_extracted_500.smi`,
the template at the center of Issue #72's original L984 failure) stays
`non_ring`, with observations shrinking from 235 → 231 — now numerically
identical to its checked-in corpus weight (231), i.e. every one of its
historical occurrences had exactly one genuine match and it was always
non-ring. The pipeline is tightening the data, not breaking it.

## Method

Same 100-target sample, same per-target configuration already checked into
`renkin_native.jsonl` (depth=5, beam-width=100, max-routes=1,
`data/building_blocks.smi`, `data/templates_extracted_500.smi`). Each target
runs at six arms:

- `disabled` — must reproduce current `master` behavior exactly.
- `audit-only` — classifies and diagnoses every match (both axes) but must
  return the legacy output verbatim (never filters).
- `conservative` — both axes (ring-context, element-accounting) enforced.
- `conservative` again (`conservative_repeat`) — same-process determinism
  check.
- `ring-only` — ring-context enforced, element-accounting audit-only.
  Isolates the ring-context gate's individual contribution.
- `element-only` — element-accounting enforced, ring-context audit-only.
  Isolates the element-accounting gate's individual contribution.

150s external wall-clock timeout per invocation (the gate harness's own
bound, not a `renkin` CLI limit).

## Headline results

| arm | completed | timeout | routes found |
|---|---|---|---|
| disabled | 100 | 0 | 16/100 |
| audit-only | 100 | 0 | 16/100 |
| conservative | 100 | 0 | 14/100 |
| conservative-repeat | 100 | 0 | 14/100 |
| ring-only | 100 | 0 | 14/100 |
| element-only | 100 | 0 | 15/100 |

### Disabled vs AuditOnly — must be identical by construction

- `status` flips: **0**, `route_found` flips: **0**, `route_signature`
  flips: **0**.

Clean zero-diff result on all 100 targets with the corrected sidecar (an
earlier run against the old sidecar saw one target time out under
`audit-only`'s added diagnostic overhead — a performance artifact, not
reproduced here). `AuditOnly`'s diagnose-without-filter guarantee holds
with no exceptions.

### Disabled vs Conservative

- `status` flips: 0
- `route_found` flips: **2** — `uspto50k_test#L1167`, `uspto50k_test#L984`
  (both `True → False`)
- `route_signature` flips (still solved, different route): **2** —
  `uspto50k_test#L4464`, `uspto50k_test#L4575`

**`L984`** (unchanged from the prior revision's finding): real-data
counterpart of the `extracted_9`/isoindolinone unit-test regression
(`src/ring_context.rs::extracted_9_conservative_rejects_isoindolinone_ring_opening`).
Target SMILES
`Cc1ccc(C(=O)Nc2ccc(-c3ccc4c(c3)CN([C@H](C(=O)O)C(C)C)C4=O)cc2)cc1` contains
an isoindolinone (benzene-fused γ-lactam). This route is independently
invalid on two grounds, not just this guard's own opinion: the
currently-published `renkin_native.jsonl` entry for it already carries
`"target_element_accounting_status": "unaccounted_target_element"` from
the pre-existing comparison-harness validator, and the RingOnly/ElementOnly
ablation below (§ RingOnly/ElementOnly ablation) shows this specific match
independently fails this guard's own element-accounting axis too, with
ring-context off. `extracted_9`'s corrected observations (231/231
non-ring, 0 ring) still classify it `non_ring`, so `Conservative` fails
closed here; no alternate route is found within budget.

**`L1167`** (new in this revision — did not flip under the old,
falsely-permissive sidecar): target SMILES `CC(N)C(=O)N1CC(=O)NC1(C)C`, a
methylated hydantoin/imidazolidinedione ring. Directly traced (not
inferred): re-running this target at `disabled` gives `routes_found: 1`;
at `conservative`, `ring_context_diagnostics` shows
`"ring_rejects_nonring_intent_on_ring_bond":2042` and
`"outcomes_element_rejected":0` for this specific target — i.e. this is a
**pure ring-context rejection**, unrelated to element-accounting, and
`routes_found: 0` follows. This is a **provenance-inconsistent
application, conservatively rejected** — the template's historical
occurrences give no support for applying this disconnection to a ring
bond, so the guard fails closed by design. That is a safety
classification, not independent proof that this specific disconnection is
chemically impossible at this site (unlike `L984`, no independent
element-accounting or pre-existing-validator confirmation was checked for
this target). Under the old, inflated-`either` sidecar this same
application was classified permissively and passed through unflagged; the
corrected sidecar now classifies it strictly by the same mechanism as
`L984` (an extracted template trained overwhelmingly non-ring
pattern-matching a real target's ring bond) — the underlying mechanism is
shared, the *chemical* wrongness is only independently confirmed for
`L984`.

**`L4464`/`L4575`** (route changed, still solved): same targets flagged in
the prior revision; the search finds a different alternate route — one
that passes this guard's implemented structural gates — after an unsafe
match is rejected. This is not an external chemical-correctness claim
about that alternate, only that it clears `Conservative`'s own checks.
`L3857` (flagged in the prior revision) no longer flips at all with the
corrected sidecar, i.e. `disabled` and `conservative` now agree on its
route.

### Conservative vs Conservative-repeat — determinism

- `status` flips: 0, `route_found` flips: 0, `route_signature` flips: 0.

Zero diffs across all 100 targets, run in two separate process
invocations. The guard is deterministic.

### RingOnly / ElementOnly ablation — attributing the 4 route changes

| comparison | route_found flips | route_signature flips |
|---|---|---|
| Disabled vs RingOnly | L1167, L984 | L4464, L4575 |
| Disabled vs ElementOnly | L984 | L4575 (different alternate route than Conservative's) |
| Disabled vs Conservative | L1167, L984 | L4464, L4575 |

**RingOnly alone reproduces Conservative's full route-level effect on this
100-target sample** — every target Conservative changes, RingOnly changes
identically. Element-accounting's independent marginal contribution here is
real but doesn't change any *final* route on this sample: aggregate
`outcomes_element_rejected` is 143 under RingOnly vs. 333 under
Conservative (Conservative rejects ~2.3× more outcomes on element grounds),
but none of those extra rejections happen to sit on this sample's
best-route path.

**`L984` fails under both axes independently** — RingOnly alone loses the
route (ring-context rejects), and ElementOnly alone *also* loses it
(confirmed via `src/ring_context.rs::element_only_ablation_still_attempts_ring_unsafe_match`,
which found this exact isoindolinone case fails element-accounting too: a
fused-ring template misapplication can't cleanly split into the two
product fragments the template's SMIRKS declares, since removing a ring
bond never disconnects a graph into two components). This is what lets
`L984` be described as independently invalid above (§ Disabled vs
Conservative) rather than only conservatively rejected: two of this
guard's own independent checks agree, not just one.

**`L4575` takes a different alternate route under ElementOnly than under
RingOnly/Conservative** (distinct `route_signature`): the two gates are
not simply superadditive — with ring-context left `AuditOnly`, a
ring-context-unsafe match stays available to the search and it settles on
yet another distinct route through it. This `ElementOnly` alternate is
**not** described as "valid" here: by construction it can still contain
the same kind of ring-context-unsafe disconnection this guard exists to
catch, since `ElementOnly` never enforces that axis. It only demonstrates
that the two axes interact rather than one subsuming the other.

### Aggregate `ring_context_diagnostics`

| counter | audit-only | conservative | ring-only | element-only |
|---|---|---|---|---|
| matches_enumerated | 454,427 | 471,729 | 472,421 | 462,044 |
| matches_ring_checked | 241,828 | 255,725 | 255,998 | 246,213 |
| matches_applied | 362,750 | 368,798 | 369,743 | 462,044 |
| outcomes_accepted | 362,606 | 368,465 | 369,600 | 461,229 |
| outcomes_element_rejected | 144 | 333 | 143 | 815 |
| ring_rejects_nonring_intent_on_ring_bond | 91,677 | 102,931 | 102,678 | 93,762 |
| ring_rejects_ring_intent_on_nonring_bond | 0 | 0 | 0 | 0 |
| ring_rejects_unknown_intent_on_ring_bond | 0 | 0 | 0 | 0 |
| invalid_mapped_bond | 0 | 0 | 0 | 0 |
| templates_missing_metadata | 0 | 0 | 0 | 0 |

`element-only`'s `matches_applied == matches_enumerated` by construction
(ring-context axis is `AuditOnly` there, so nothing is skipped before
`apply_reaction_match`). No `ring_rejects_ring_intent_on_nonring_bond` or
`ring_rejects_unknown_intent_on_ring_bond` fired on this sample — covered
by the dedicated unit tests in `src/ring_context.rs`, not this measurement.

### Latency

| arm | n | total | p50 | p95 | max |
|---|---|---|---|---|---|
| disabled | 100 | 1429.2s | 7.63s | 61.56s | 75.28s |
| audit-only | 100 | 1744.4s | 9.41s | 73.46s | 88.51s |
| conservative | 100 | 1309.5s | 7.85s | 50.36s | 66.26s |
| ring-only | 100 | 1297.2s | 7.58s | 50.48s | 69.24s |
| element-only | 100 | 1437.1s | 7.70s | 59.71s | 82.20s |

`conservative`/`ring-only` are not slower than `disabled` in aggregate
(rejecting unsafe matches early skips otherwise-wasted
`apply_reaction_match` work). `audit-only`/`element-only` (both leave the
ring-context axis un-enforced, so nothing is skipped pre-apply) carry the
expected overhead from unconditional classification/diagnostics.

## Shared-stock confirmation

Issue #66's formal comparison uses two stock arms: native (this branch's
own 402-compound `data/building_blocks.smi`) and shared (the 393-compound
`data/comparison/shared_stock/shared_stock.smi`, constructed for a
guaranteed zero-diff identity between RENKIN's and AiZynthFinder's stocks).
Everything above uses native stock; before a 500-target run touches shared
stock too, this reruns the same 100 targets against shared stock at a
cheaper 3-arm subset (`disabled`/`conservative`/`conservative-repeat` —
`--arms` on `scripts/ring_context_gate.py`) as a confirmatory check, not a
repeat of the full ablation. Raw data:
`data/comparison/ring_context_gate_results_shared_stock_100.json`; report:
`data/comparison/ring_context_gate_report_shared_stock_100.txt`.

| arm | completed | routes found |
|---|---|---|
| disabled | 100 | 16/100 |
| conservative | 100 | 14/100 |
| conservative-repeat | 100 | 14/100 |

- **Disabled vs Conservative**: identical target-level result to native
  stock — `route_found` flips on `L1167`/`L984` only, `route_signature`
  flips on `L4464`/`L4575` only, 0 status flips. Aggregate
  `ring_context_diagnostics` for `conservative` are numerically identical
  to native stock's (e.g. `matches_applied: 368,798`,
  `ring_rejects_nonring_intent_on_ring_bond: 102,931`) — for this 100-target
  sample, stock choice doesn't change which matches get enumerated or
  ring-checked, only (separately, not measured by this gate) which leaves
  ultimately count as solved.
- **Conservative vs Conservative-repeat**: 0 diffs (deterministic, same as
  native stock).
- No crashes, no invalid output, no unexplained target-level difference
  between native and shared stock.

This is a confirmatory check, not a full ablation: `audit-only`/
`ring-only`/`element-only` were not re-run against shared stock (their
native-stock results already establish the mechanism; re-running here
would only be useful if this 3-arm check had surfaced a stock-dependent
discrepancy, which it didn't).

## Conclusion

- `Disabled` ≡ current `master` (unaffected; this policy is the default).
- `AuditOnly` returns byte-identical output to `Disabled` on all 100
  targets. The guard's diagnose-without-filter invariant holds with no
  exceptions.
- `Conservative` changes exactly 4/100 targets, every one explained: two
  routes removed with no budget left to find a replacement (`L984` —
  independently invalid, confirmed by both this guard's element-accounting
  axis and the pre-existing comparison-harness validator; `L1167` — a
  provenance-inconsistent application conservatively rejected because the
  template's historical occurrences give no support for this disconnection
  on a ring bond, first visible only after the generator's attribution fix
  removed false `Either` permissiveness), and two routes swapped for a
  different alternate that passes this guard's implemented structural
  gates after an unsafe match was rejected — not an external
  chemical-correctness claim about those alternates. `Conservative` is
  deterministic across repeated runs.
- The `RingOnly`/`ElementOnly` ablation shows ring-context is doing the
  observable route-level work on this sample; element-accounting's
  contribution is real (fewer accepted outcomes) but not yet
  route-changing here, except in combination on `L984` (both axes reject it
  independently) and in producing a *different* alternate for `L4575` when
  isolated (that alternate is not itself asserted valid — `ElementOnly`
  never enforces the ring-context axis).
- The shared-stock confirmatory check (§ Shared-stock confirmation)
  reproduces the exact same target-level result as native stock, with
  identical aggregate diagnostics and clean determinism — no
  stock-dependent surprise before a formal shared-stock run.
- No unexplained route loss. This clears the bar to report and stop here;
  the 500-target run is explicitly out of scope for this PR.
