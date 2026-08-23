# Finding #4 validator-fidelity pilot — 2026-08-23

**Status: pilot evidence only, not a formal validator-fidelity measurement.**
This run demonstrates the harness and produces a real, reproducible data
point, but the result is strongly right-censored (see "Why this is not
comparable to Phase 31" below) and must not be cited as a measurement of
RENKIN's current Invalid/Valid rate.

## Purpose

Follow-up to `docs/design/retro-rule-precision-gaps-v0.md` Finding #4: a
rule-stratified sample of "Invalid + atom-balanced" route steps, to
eventually classify each as a genuine rule/template error vs. a validator
false negative. Explicit scope constraint from this round: no full
4,907-target remeasurement; ~100 Invalid+balanced steps from a smaller,
targeted sample was the original goal.

Designed after `docs/design/retro-rule-precision-gaps-v0.md`'s Issue #128
investigation (root-caused and closed, moved upstream to
[chematic#372](https://github.com/kent-tokyo/chematic/issues/372)) made clear
that per-target search latency in this configuration is highly variable and
not predictable in advance — a per-target timeout was added to
`examples/inspect_validation` (`INSPECT_VALIDATION_TIMEOUT_SECS`, PR #177)
specifically to bound this pilot's wall-clock.

## Environment / provenance

| | |
|---|---|
| renkin commit (binary under test) | `d0f6421` (branch `feat/inspect-validation-timeout`) — first-timeout-fix commit; the later review-feedback commit `601821a` (fail-fast env parsing, `termination=` on route lines, tests) postdates this run and does not affect its output shape or results |
| build | `cargo build --release --example inspect_validation` (default opt-level=3) |
| OS / CPU | macOS (Darwin 25.5.0, arm64), 10 cores (`sysctl hw.ncpu`) |
| depth | 5 |
| beam-width | 100 |
| max-routes | 1 |
| per-target timeout | 90 seconds (`INSPECT_VALIDATION_TIMEOUT_SECS=90`) |
| templates | `data/templates_extracted_5000.smi` — sha256 `517f6a084921141b6080c3827c75e6c51ac148455218695dee6e9712e3731517` (byte-identical to the Phase 31 corpus, `tasks/phase31_final_remeasurement_run.md`) |
| building blocks | `data/building_blocks.smi` — sha256 `6fb4550dbc29480427ef4331dc492f0f66a315776b32bf1a6ab7057c6f1521dd` (byte-identical to Phase 31's, 402 loaded compounds) |
| sharding | 5-way round-robin (`NR % 5`, `awk`), `RAYON_NUM_THREADS=2` per shard |
| target sample source | `data/uspto50k_test.smi` (4,907 targets after stripping comments) |
| sample method | Python `random.seed(42); random.sample(all_targets, 300)` — **not** pre-filtered by solved status (unlike Phase 31's own n=300 diagnostic sample, which drew from the already-known-solved 986; this pilot's 300 are drawn from the full 4,907, since no fresh full-corpus solve list exists for the current commit and generating one was explicitly out of scope) |
| target sample file | `data/finding4_pilot_2026-08-23/target_sample_n300_seed42.smi` (gitignored, local only) — sha256 `25fbfc3e0df199a56a6d888b25396ba144b3faf6502a63c70b9dd97caef2c7ef` |

## Command

```
INSPECT_VALIDATION_TIMEOUT_SECS=90 RAYON_NUM_THREADS=2 \
    ./target/release/examples/inspect_validation < shard_N.smi > shard_N.out
```
— run 5-way in parallel (`data/finding4_pilot_2026-08-23/run_shards.sh`, gitignored local copy).

## Timing

Started `2026-08-23T19:01:13+09:00`, finished `2026-08-23T20:31:58+09:00` —
**90 minutes 45 seconds** total wall-clock (5 shards of 60 targets each,
running concurrently).

## Results (n=300, exact — from the 5 shard `.out` files)

| Outcome | Count | % of 300 |
|---|---:|---:|
| TIMEOUT (`SearchTermination::DeadlineExceeded`, no route found before 90s) | 274 | 91.3% |
| ROUTE found, `Validated` | 16 | 5.3% |
| ROUTE found, `Invalid` (≥1 step confirmed wrong) | 5 | 1.7% |
| UNSOLVED (search genuinely exhausted within 90s, no route) | 5 | 1.7% |

21 routes were returned in total (16 + 5), yielding **46 validated steps**.

### Step-level validation × atom-balance (n=46 steps, from the 21 returned routes)

| Status × balance | Count | % of 46 |
|---|---:|---:|
| Valid + balanced | 39 | 84.8% |
| **Invalid + balanced** | **6** | **13.0%** |
| Valid + imbalanced | 1 | 2.2% (expected — a graph-based rule with byproduct loss, see `src/validation/atom_conservation.rs`'s own doc comment) |

Raw output: `data/finding4_pilot_2026-08-23/shard_{0..4}.out` (gitignored,
local only). Combined sha256 (all 5 files concatenated in shard-index
order): `65f87abf4827e0d80d715c6afb80bc933e44055aac996cb984c5328ea5e74f99`.

### The 6 Invalid+balanced steps

| rule | target | precursors |
|---|---|---|
| `extracted_824` | `O=C2NCC(O2)Cc1ccccc1` | `C(=NC)=O.OCCc1ccccc1` |
| `cc_single_cleavage` | `C1CC[C@H](C)NC[C@@H]1C` | `C1CCCC[C@H](N1)C.C` |
| `extracted_109` | `C1CCCC[C@H](N1)C` | `C(C)(CCCC)=O.C(CCN)C` |
| `extracted_112` | `c2ccc1CCC(c1c2)=O` | `C(C(=O)Cl)C.c1ccc(C)cc1` |
| `extracted_4255` | `C2c1cc(c(F)cc1C(O2)=O)F` | `O=C.C(=O)O.c1(F)ccccc1F` |
| `co_aliphatic_cleavage` | `O=C(O[C@@H]1CCCNC1)N` | `C1CCNCC1.NC(O)=O` |

No chemistry classification is asserted here — that is separate,
per-step work, tracked outside this pilot doc (`co_aliphatic_cleavage`'s
row is being investigated individually, frozen as a fixture, not by
re-running search — see the project's own working notes for that
sub-investigation's method and disposition).

## Why this is not comparable to Phase 31

**Correct statement of this result**: *Among 46 steps from routes obtained
within the 90-second per-target deadline, 6/46 (13.0%) were Invalid and
atom-balanced. Because 274/300 targets timed out, this result is strongly
censored and is not directly comparable with the Phase 31 72.2% figure.*

This is **not** evidence that RENKIN's Invalid rate improved from 72.2%
(Phase 31, `e20dc8c`) to 13.0% now. The 46 steps here are a **right-censored
sample**: only routes findable within 90 seconds are represented at all, and
Issue #128's own root cause (chematic's `canonical_smiles` combinatorial cost
on locally-symmetric molecules — Boc/tBu/pivaloyl groups, rings, cages)
means the 90-second survivors are structurally non-representative of the
full 300-target draw, very likely undersampling exactly the kind of
protecting-group-bearing molecules common in real synthesis. Whether the
true population-wide Invalid rate has actually improved, stayed flat, or
even worsened cannot be determined from this pilot.

The 91.3% timeout figure is scoped the same way: **this fixed 300-target
sample, at this exact configuration, saw 91.3% time out** — this must not be
generalized to "91.3% of real molecules" or any broader population claim.

## Why the timeout was not extended, and why 4,907-target remeasurement was not run

Both were explicitly out of scope for this round:

- Raising the timeout (e.g. to 180s or 300s) would push total wall-clock
  into multi-hour territory for the same 300-target sample, would not
  eliminate the timeout-survivor bias (only shift which targets survive),
  and would conflate two different things being measured (validator
  correctness vs. raw search speed) in one run.
- The underlying cost is chematic-side and not yet fixed upstream
  (chematic#372, open). Spending more compute against the same
  not-yet-optimized canonicalization path produces diminishing diagnostic
  value.
- A remeasurement against this *same* 300-target manifest (same sample file,
  same seed, same config) after any upstream chematic#372 fix lands is the
  highest-value next full-sample run — directly comparable timeout-rate,
  route count, step count, and validation-distribution deltas against this
  pilot's own numbers, not just against Phase 31's much older baseline.

## Next steps (tracked outside this document)

1. `co_aliphatic_cleavage`'s single Invalid+balanced step (the row most
   plausibly a validator false negative rather than a genuine template
   error, given the others show clearer structural/connectivity defects on
   inspection) — investigated as a frozen fixture, not by re-running search.
2. `extracted_112` and `cc_single_cleavage` — candidates for minimal
   reproduction fixtures if their apparent defects (an intermolecular
   disconnection standing in for what should be an intramolecular
   cyclization; a methane "leaving group" with no real chemical handle)
   are confirmed on closer inspection.
3. Re-run this exact 300-target manifest once chematic#372 lands a fix, to
   get a real before/after comparison free of Phase-31-vs-now confounds.
