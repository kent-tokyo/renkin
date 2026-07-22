# Phase 31 final corrected-baseline USPTO-50k re-measurement (provenance)

Run against `master` **after both 31.11 (halide atom-loss rule removal, PR #31)
and 31.12 (rule-provenance validator fix, PR #33) merged** — per Phase 31's
explicit rule not to re-measure between the two fixes. This supersedes the
provisional `tasks/phase31_corrected_baseline_run.md` (24.01% raw,
commit `35f26cb`, deliberately never published as the corrected baseline
because it predates both fixes).

## Commit / environment

| | |
|---|---|
| renkin commit (binary under test) | `e20dc8ccca6cac10484dd20b3191a2cbfafefb93` (master) — PR #30 (record integrity) + PR #31 (31.11 halide fix) + PR #32 (aggregation script) + PR #33 (31.12 validator fix) merged |
| working tree | clean (`git status --porcelain` empty at build time) |
| rustc | 1.97.0 (2d8144b78 2026-07-07) |
| chematic | 0.4.25 |
| renkin | 0.15.5 |
| OS | macOS 26.5.2 (Darwin 25.5.0, arm64) |
| CPU | Apple M4, 10 cores |
| build | `cargo build --release` (default opt-level=3) |
| handcrafted rule count | 28 (was 31 pre-31.11; 3 atom-loss halide rules removed) |

## Corpus / config

| | |
|---|---|
| depth | 5 |
| beam-width | 100 |
| max-routes | 1 (`run_benchmark_chunks.sh` default) |
| templates | `data/templates_extracted_5000.smi` — sha256 `517f6a084921141b6080c3827c75e6c51ac148455218695dee6e9712e3731517`, 5,000 entries |
| building blocks | `data/building_blocks.smi` — sha256 `6fb4550dbc29480427ef4331dc492f0f66a315776b32bf1a6ab7057c6f1521dd`. **`ChemEnv::bb_count()` after `ChemEnv::load()` = 402** (the actual loaded/searchable count — not the raw line count). Breakdown: 449 non-empty non-comment trimmed lines → 3 fail to parse → 446 parse successfully → 44 are duplicates after canonicalization → 402 unique. Do not cite "475" (a naive `grep -cv '^#'` line count used in an earlier draft of this doc, before this check) or the older "509" figure (a historical documentation value from an earlier measurement era, not independently re-verified against this file) as the building-block count for this run. |
| targets | `data/uspto50k_test.smi` — sha256 `ac246998c6e7b2c68904eaf030e4c6d8c72f0c35334f86e55100d504586fff3d`, 4,907 targets |
| `--plausibility` | on (hardcoded by `run_benchmark_parallel.sh`, atom-balance + rule-provenance-bound forward validation) |
| sharding | 5-way round-robin (`NR % 5`), `RAYON_NUM_THREADS=2` per shard |
| random seed | N/A — search is deterministic (no RNG in `src/search.rs`/`src/chem_env.rs`) |
| aggregation script | `scripts/aggregate_bench_results.py` (added PR #32) at commit `e20dc8c` |

## Command

```
bash scripts/run_benchmark_parallel.sh \
    data/uspto50k_test.smi \
    data/templates_extracted_5000.smi \
    data/bench_chunks_phase31_final_e20dc8c \
    5 100 5 2 \
    data/building_blocks.smi
```

## Status

Started 2026-07-21T23:24:12Z, finished 2026-07-22T02:19:36Z (wall clock
≈2h55m; per-shard `real` time 9,500–10,363s ≈ 2.64–2.88h per
`shard_*.rss.txt`). Pre-run dry-run (50 targets, depth=3 for speed) on
this exact commit completed successfully end-to-end (0 failed chunks,
`aggregate_bench_results.py --expected-total 50` passed) before launching
the full run. 50/50 chunks, **0 failed chunks, 0 repair passes needed**
(no `WARN: renkin-bench failed` in any `shard_*.log`). Peak RSS per shard:
98–164 MB.

## Results (n=4,907, exact — `scripts/aggregate_bench_results.py`)

Reconciliation: `sum(chunk.total) == len(results[]) == 4,907` (script
hard-asserts this; ran with `--expected-total 4907`, exit 0).

**Nested series — all three over the SAME denominator (4,907 total
targets), so they ARE directly comparable to each other** (raw ⊇
atom-balanced ⊇ current-validator-confirmed, each a subset of the previous):

| Public label | Internal metric | Value |
|---|---|---|
| Search-to-stock rate | `raw_solved_rate` | 986 / 4,907 = **20.09%** |
| Atom-balance-filtered rate | `atom_balanced_solved_rate` | 756 / 4,907 = **15.41%** |
| Current-validator-confirmed rate | `provenance_validated_solved_rate` | 43 / 4,907 = **0.88%** |

These three are a nested series over the same 4,907 targets, not
independent measurements, and none of them is an experimentally-verified
synthesis success rate or a human-chemist-reviewed route-accuracy figure.

**What `provenance_validated_solved_rate` (0.88%) actually is, precisely**:
the fraction of targets with a complete stock route where every step (a)
passes the coarse atom-balance check AND (b) is positively confirmed by
its own originating rule's current validator (reverse-SMIRKS match or
graph-structural check, as applicable). **This is not a measured
chemical-accuracy rate, and it is not a proven lower bound on true
correctness** — that would require knowing the validator has no false
positives, which has not been established (see the caveat below: 1 of the
44 `validated` routes fails the atom-balance check, and a diagnostic
sample shows 14/864 steps are `Valid`-but-atom-imbalanced). Read 0.88% as
"the fraction positively confirmed by the current validator under the
stated checks" — not as "RENKIN is chemically correct 0.88% of the time,"
and not as a guaranteed floor on that number either. `atom_balanced_solved_rate`
(15.41%) is likewise necessary-but-not-sufficient for correctness — mass
balance does not imply regiochemical or mechanistic correctness.

Other metrics:

| Metric | Value |
|---|---|
| `depth=0` direct stock hit | 2 / 4,907 = 0.04% |
| `pct_atom_balanced_of_solved` (diagnostic only — of 986 solved, NOT of all 4,907; do not compare directly to the nested series above) | 76.67% (756/986) |
| `route_validation_status` (of 986 solved) | 942 invalid, 44 validated, 0 partially_validated, 0 not_evaluable |
| `validation_coverage` (step-level) | 100.0% uniformly across all 50 chunks |
| `evaluable_validation_pass_rate` | non-uniform across chunks (range 8.0%–38.9%) — **not exactly reconstructable as a single weighted figure** from the current JSON schema (known gap, see `scripts/aggregate_bench_results.py`'s docstring); do not average the range into a point estimate |
| latency, all targets (ms) | p50=7,338 / p95=28,228 / p99=51,165 / max=181,053 / mean=10,175 |
| latency, solved only (ms) | p50=977 / p95=9,362 / p99=15,596 / max=37,889 / mean=2,309 |
| solved-route depth distribution | 0:2, 1:102, 2:324, 3:277, 4:179, 5:102 |

Note: 44 routes are `route_validation_status=validated` but only 43 are
also `atom_balance_ok` — one `validated` route fails the coarse MW-based
atom-balance check despite every step passing its own rule's structural/
SMIRKS-reversal validator. Not investigated further (single-instance edge
case, doesn't change any published figure meaningfully); flagged here
rather than silently dropped.

## Comparison to the provisional pre-fix measurement

| | Provisional (`35f26cb`, pre-31.11/31.12) | This run (`e20dc8c`, post-31.11/31.12) |
|---|---|---|
| `raw_solved_rate` | 24.01% (1,178/4,907) | **20.09%** (986/4,907) |
| `atom_balanced_solved_rate` (of all 4,907 — provisional run only reported `pct_atom_balanced_of_solved`; recomputed here for comparability: 687/4,907) | 14.00% | **15.41%** |
| `pct_atom_balanced_of_solved` (diagnostic, different denominator) | 58.32% | 76.67% |
| `route_validation_status=validated` of solved | 49/1,178 (4.2%) | 44/986 (4.5%) — but now provenance-bound, not cross-rule-contaminated (confounded comparison: both the rule set and the solved set changed between runs, so this is not evidence 31.12 "worked" on its own — the pinned regression test in PR #33 is the actual proof) |
| `evaluable_validation_pass_rate` | 20.25% (single figure — later found to itself be a schema-averaging artifact) | non-uniform per-chunk range 8.0–38.9%, reported as a range rather than a misleading point estimate |

The further raw-rate drop (24.01% → 20.09%) is the expected, predicted
consequence of removing the three atom-loss halide rules (31.11) — the
same pattern as the earlier `aryl_carboxylation_retro` fix (199/200 →
61/200 in that isolated n=200 sample). It is **not** a regression in
search capability; it is fake "solved" routes (chlorobenzene → benzene
with the Cl silently discarded, etc.) no longer being counted. The rise
in `pct_atom_balanced` (58.32% → 76.67%) corroborates this directly.

## Rule-usage / per-rule validation breakdown (n=300 solved-target sample, seed=42)

Sampled 300 of the 986 solved targets (Python `random.seed(42)`, same
methodology as the provisional run) and ran
`examples/inspect_validation` (same search config as the benchmark:
depth=5, beam=100, max-routes=1) — reconstructs each target's best route
deterministically and reports per-step `rule` + `StepValidationStatus` +
atom-balance. 300 routes, 864 steps, 167 distinct rules used.

Route-level (this sample): 290 Invalid, 10 Validated, 0 PartiallyValidated,
0 NotEvaluable (10/300 = 3.3%, in the same ballpark as the full-population
44/986 = 4.5% — sampling variance on a small validated-route count).

**Confirms 31.11 took effect**: zero steps use `aryl_chloride_retro`,
`aryl_iodide_retro`, or `aryl_fluoride_snAr_retro` (all three fully absent
from `default_rules()`, as expected).

**Confirms 31.12 took effect and did not introduce new `NotEvaluable`
noise**: `NotEvaluable` count is 0 for every one of the 167 rules used —
every step's `step.rule` name resolves to an actual rule in the combined
default+extracted rule set, so provenance-binding never degrades to "can't
evaluate" in practice at this rule-pool size.

Step-level atom-balance × validation cross-tab (864 steps): Invalid+balanced
624 (72.2%), Invalid+imbalanced 60 (6.9%), Valid+balanced 166 (19.2%),
Valid+imbalanced 14 (1.6%). The Valid+imbalanced 14 are expected, not a
regression: several graph-based rules (e.g. `boc_deprotection_retro`,
discarding a volatile isobutylene/CO2 byproduct) are intentionally
single-fragment and fail the coarse MW-only `step_balanced()` check by
design — documented in `src/validation/atom_conservation.rs`'s module
doc comment, unrelated to 31.11/31.12.

Top rules by usage (≥10 occurrences), Valid/Invalid/NotEvaluable:

| Rule | Uses | Valid | Invalid | Invalid % |
|---|---|---|---|---|
| `cc_single_cleavage` | 117 | 9 | 108 | 92.3% |
| `aryl_amine_retro` | 52 | 11 | 41 | 78.8% |
| `cn_aliphatic_cleavage` | 41 | 1 | 40 | 97.6% |
| `co_aliphatic_cleavage` | 37 | 8 | 29 | 78.4% |
| `suzuki_retro` | 28 | 28 | 0 | **0.0%** |
| `aryl_carboxylation_retro` | 25 | 7 | 18 | 72.0% |
| `extracted_51` | 22 | 0 | 22 | 100.0% |
| `negishi_retro` | 21 | 14 | 7 | 33.3% |
| `extracted_64` | 21 | 19 | 2 | 9.5% |
| `boc_deprotection_retro` | 20 | 6 | 14 | 70.0% |
| `extracted_30` | 18 | 0 | 18 | 100.0% |
| `extracted_12` | 16 | 1 | 15 | 93.8% |

**Not resolved by this PR or this measurement** (explicitly out of scope,
matches `tasks/todo.md` 31.12's still-open second half): what 72.2%
(624/864 steps in this sample) directly establishes is only that these
steps are `Invalid` under the reverse-SMIRKS/graph-structural check *and*
pass the coarse MW-based atom-balance check — atom balance is closer to a
necessary condition for a correct reaction than a sufficient one, so
passing it does not mean the step is chemically correct. **An unknown
fraction of these `Invalid` verdicts may be validator false negatives; the
remainder may be genuine rule or route errors. The split between the two
has not yet been measured.** One concrete false-negative mechanism is
already documented (PR #33's description): a `chematic` canonicalization
quirk where a freshly-installed bracket atom gets `hydrogen_count =
Some(0)` while a parsed target's bare atom gets `hydrogen_count = None`,
so `canonical_smiles` never matches even for a chemically-correct pair —
confirmed for `aryl_chloride_to_bromide`'s own reversal, not investigated
beyond that one rule, and not shown to generalize to the other 623 steps.
`suzuki_retro` at 0% invalid vs. `cn_aliphatic_cleavage` at 97.6% invalid
is a wide enough spread to be worth investigating further, but does not by
itself establish which of the two explanations (rule/template errors vs.
validator false negatives) dominates, or in what proportion. Recommended
as the next Phase-31-adjacent investigation, not undertaken here.

## Phase 31 status

Phase 31 corrected-baseline publication is complete: the three nested
metrics above are measured, reproducible (`scripts/aggregate_bench_results.py`
against the raw chunk JSON, `data/bench_chunks_phase31_final_e20dc8c`),
and published with explicit framing against misreading
`provenance_validated_solved_rate` as a measured correctness rate.
Validator fidelity analysis — separating real chemistry errors from
validator false negatives in the 72.2% `Invalid`-but-atom-balanced
population — remains an explicit, open follow-up, not undertaken in this
measurement. Cascade (Stage 2) and ChEMBL OOD re-measurement against the
corrected rule set have also not been started.

## Dependency note (2026-07-22, post-publication)

`chematic` was bumped `0.4.25` → `0.4.30` on `master` shortly after this
baseline was published (dependabot PR #24, `chore(deps): bump chematic
from 0.4.25 to 0.4.30`). The measurement above (commit `e20dc8c`) was
built against `chematic 0.4.25`; `master` as of the chematic bump uses
`0.4.30`.

Investigated before merging: the bump broke exactly one existing test
(`suzuki_retro_biphenyl_gives_bromobenzene_and_benzene`), which hardcoded
a canonical-SMILES string ("c1ccccc1" as a substring) that changed format
between versions (0.4.25 wrote "Brc1ccccc1", 0.4.30 writes
"c1ccc(cc1)Br" for the same molecule) — confirmed self-consistent (three
different input SMILES for bromobenzene all canonicalize to the same
0.4.30 string) and chemically identical, not a semantics change. Fixed by
computing the expected canonical form at test time instead of hardcoding
it. Full test suite (112/112) and the Phase-31-specific subset (halide-
rule-removal regression tests, provenance-bound validator tests,
cross-rule corroboration fixture) pass unchanged under `chematic 0.4.30`.

`chematic`'s 0.4.26–0.4.30 changelog includes substantive fixes
(a P0 stereo-metadata bug in `apply_kekule` and related functions, a
SMARTS `[rN]` matching-accuracy fix from 96.9% to 99.93% agreement,
an aromatic bond-direction stashing consolidation) — net expected
direction is more correct, not less, but **the corrected-baseline numbers
above (20.09% / 15.41% / 0.88%) were not re-measured against
`chematic 0.4.30`** and should not be assumed to reproduce byte-for-byte
against current `master`. A future re-measurement should record the
`chematic` version in its manifest (already a required field per the
Phase 31 measurement-reproducibility checklist) and note any drift
explicitly rather than silently carrying these numbers forward.
