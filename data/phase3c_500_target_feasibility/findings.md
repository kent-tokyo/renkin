# Phase 3C: 500-target TRAIN/VAL feasibility (Issue #101)

Per the user's explicit direction, this round is centered on **TRAIN/VAL**,
not TEST -- reranker design decisions must not be made while looking at
benchmark-test candidate coverage or rank distributions. The formal 500-target
TEST pool generated in Phase 3B was pipeline smoke only and is not
re-examined here. **No formal TEST evaluation happens in this document.**

## A real bug this staged feasibility check caught (Phase 3C-2)

While validating the train-500 pool against the real split-manifest via
`train_reranker.py::load_split_manifest`, group `uspto50k_train#L35` failed
exact-coverage validation: its driver-derived `target_id` did not appear
anywhere in `data/reranker_split_manifest.jsonl`.

Root cause, isolated via direct `renkin-canonicalize` round-trips: RENKIN's
canonical-form tie-break is **sensitive to whether the parsed `Molecule`
still carries atom-map annotations**. `propose_one_step` derives `target_id`
by calling `to_canonical` on the molecule it built internally; the original
label-generation pipeline's `target_id` for this one molecule was
canonicalized via a different atom-map state (or a different `chematic`
version) than what this binary now produces. Both forms are individually
stable/idempotent under repeated canonicalization -- this is not
non-determinism, it is a real (and narrow) canonicalization-identity
discrepancy between two otherwise-valid canonical SMILES for the same
molecule. Confirmed narrow: exactly 1 mismatch across 1,000 group checks
(500 train + 500 val).

**Fix applied** (`src/pool_export.rs`, `src/bin/pool_gen.rs`): the driver no
longer silently accepts whatever `target_id` `propose_one_step` returns. It
now compares `pool.target_id` against the caller's own `target_id` for every
group and, on any mismatch, drops that group's candidates entirely and
records it under a new `ProposalStatus::TargetIdMismatch` (distinct from
`Ok` and `TargetParseFailed`) rather than exporting candidates under an
identity nobody asked for. `n_groups_target_id_mismatch` is now a first-class
counter in the driver's summary output. This converts a defect that would
otherwise surface later as an opaque `load_split_manifest`/
`label_and_split_rows` failure into a counted, visible rejection at export
time -- the same fail-closed discipline `TargetParseFailed` already applied
to unparseable targets.

Not fixed in this round (deliberately out of scope): the underlying
`chematic`/`to_canonical` atom-map-sensitivity itself. This is an upstream
canonicalization-algorithm question, not a pool-export defect, and the
project's established practice is to file it with a minimal reproducer
rather than patch around it inside a driver. Minimal reproducer:
- Mapped SMILES: `[CH3:1][N:2]1[CH2:3][c:4]2[cH:5][c:6]([Cl:7])[cH:8][cH:9][c:10]2-[n:11]2[c:12]([Br:13])[n:14][n:15][c:16]2[CH2:17]1`
- `renkin-canonicalize --clear-atom-maps` (maps cleared before ranking): `N3(C)Cc1n(c2ccc(cc2C3)Cl)c(nn1)Br`
- Same input canonicalized with maps preserved, then maps stripped from the *already-ranked* output text: `n12c(nnc1CN(C)Cc3cc(ccc23)Cl)Br`
- Both forms independently idempotent under repeat canonicalization.

## 500-target runs (post-fix)

Both from decontaminated `data/reranker_groups_uspto50k_{train,val}.jsonl`,
first 500 lines (input-file order -- deterministic, not a random sample),
`Exhaustive` proposal mode, `data/templates_extracted_500.smi` (500 rules),
no stock (`stock=None`, see Phase 3C-6 below).

| metric | train-500 | val-500 |
|---|---|---|
| groups requested | 500 | 500 |
| groups succeeded (`Ok`) | 499 | 500 |
| parse failures | 0 | 0 |
| target_id mismatches (new guard) | 1 | 0 |
| zero-candidate groups | 0 | 0 |
| candidate rows | 14,112 | 13,643 |
| candidates/group p50 / p90 / p95 / p99\* / max | 27 / 45 / 50 / 63 / 67 | 25 / 45 / 50 / 65 / 83 |
| wall clock | 48.33 s | 43.58 s |
| peak RSS (`/usr/bin/time -l`, maximum resident set size) | 34.6 MB | 26.3 MB |
| pool file size | 14,306,607 bytes (13.6 MiB) | 13,825,880 bytes (13.2 MiB) |

\*p99 computed post-hoc from the sorted candidate-count list (driver reports p50/p90/p95/max natively).

Provenance (`manifest_{train,val}_500.json`): `renkin_git_commit`,
`cargo_lock_sha256`, `chematic_version=0.11.0`, `rules_content_hash` (500
rules), `target_input_sha256` (hash of the full groups-input file --
`--limit` truncates the run but not this hash, a known and accepted
limitation, not a blocker), `candidate_jsonl_sha256`,
`target_group_index_sha256`. Both content hashes are of the exact bytes
written, not independently recomputed.

## Real-loader validation (Phase 3C-2)

Run through the actual `train_reranker.py` machinery via new script
`scripts/phase3c_coverage_diagnostics.py` (not a reimplementation --
`validate_manifest`, `validate_pool_rows`, `label_and_split_rows`,
`compute_arm_group_metrics`, `summarize_coverage` are all called directly),
with `--split-manifest` as required.

| check | train-500 | val-500 |
|---|---|---|
| unlabeled groups | 0 | 0 |
| split assignment mismatch | 0 | 0 |
| schema mismatch | 0 (validate_manifest/validate_pool_rows both pass) | 0 |
| candidate/group index mismatch | 0 | 0 |
| zero-positive groups dropped? | no -- retained (174/500) | no -- retained (177/500) |
| test-target leakage into train/val | none (split-manifest assignment for every touched target_id matches the requested split) | none |

## Proposal coverage diagnostics (Phase 3C-3) -- pipeline feasibility only

**Not a reranker-accuracy gate.** No hyperparameter/feature changes made
while producing or reading this table.

| metric | train-500 | val-500 |
|---|---|---|
| groups with >=1 positive | 326 / 500 (65.2% of all groups) | 323 / 500 (64.6%) |
| groups excluded by target_id-mismatch guard (data-gen defect, not a coverage gap) | 1 | 0 |
| groups with genuine zero positive (real proposal attempt, no match) | 173 / 499 | 177 / 500 |
| coverage rate over proposal-attempted groups | 326 / 499 = 65.33% | 323 / 500 = 64.6% |
| positive count / group (mean, all 500) | 0.653 | 0.646 |
| best-positive original-rank p50 / p90 / p95 / max | 3 / 17 / 22 / 50 | 3 / 18 / 24 / 41 |
| rank_1 | 74 / 326 (22.7%) | 77 / 323 (23.8%) |
| rank_2-10 | 182 / 326 (55.8%) | 172 / 323 (53.3%) |
| rank_11-50 | 70 / 326 (21.5%) | 74 / 323 (22.9%) |
| rank_over_50 | 0 | 0 |

**Interpretation** (pipeline observation, not a design decision): coverage
holds at ~65% at 500-scale, consistent with Phase 3B's 100-target result
(67/100 = 67%). Among covered groups, fewer than 1 in 4 correct answers are
already at rank 1 under `propose_one_step`'s own rule-firing order; the
large majority (77%) sit at rank 2-50, with more than half of all covered
groups' positives specifically in rank 2-10. This is consistent with
substantial reranker headroom on the covered subset, alongside a real,
separate ~35% proposal-coverage gap that reranking cannot address. Both
signals as the user anticipated; no development decision is made from this
observation in this document.

## Determinism (Phase 3C-5)

Train-500 re-run from scratch (fresh `--pool-output`/`--groups-output`/
`--manifest-output` paths): `candidate_jsonl_sha256` and
`target_group_index_sha256` identical to the first run; `diff` of pool
JSONL, group index JSONL, **and manifest JSON** all empty (byte-identical).
The manifest carries no drifting fields (git SHA/timestamp) that would have
broken this for identical input.

## Stock (Phase 3C-6)

`stock=None` throughout (`stock_identity: null`,
`stock_compound_count: null` in both manifests) -- deliberate, per the
user's direction to keep this first formal reranker stock-agnostic.
`fraction_precursors_in_stock`/`all_precursors_in_stock` features are
`not_computable` for every row; never faked as `0`.

## Scale extrapolation (Phase 3C-4)

Full corpus sizes: train 39,927 groups, val 4,931 groups.

Linear-in-groups extrapolation from the 500-target measurements above:

| metric | full TRAIN (39,927) | full VAL (4,931) |
|---|---|---|
| candidate rows | ~1,126,900 | ~134,550 |
| pool file size | ~1.14 GB | ~136 MB |
| wall clock | ~64 min | ~7 min |

Peak RSS does **not** get the same linear treatment -- the driver holds the
full `candidate_rows: Vec<CandidateRow>` in memory before writing, a known
scaling concern. Fitting fixed+linear from two comparable (`stock=None`,
unaffected-by-the-mismatch-guard) points -- Phase 3B's 100-target run
(12.7 MB) and this round's val-500 run (26.3 MB) -- gives roughly 9.6 MB
fixed + ~31 KB/group. Point projection: **~1.25 GB full train, ~160 MB full
val**. Given cross-run measurement noise (train-500 measured 34.6 MB under
different system load) and the non-linear risk the Vec design carries at 80x
this scale, conservative band: **1.2-3.7 GB full train, 150-500 MB full
val**.

**GO/NO-GO against the user's stated criteria:**
- full train+val generatable in realistic time: **yes** (~71 min combined)
- disk requirement reasonable: **yes** (~1.3 GB combined)
- projected peak RSS safe: **yes** -- multi-GB is pre-authorized as GO
  "if the execution environment safely supports it"; this machine has
  17.2 GB total RAM. (Noted, not a blocker: system memory pressure was high
  at measurement time -- ~102 MB reported unused via `top` -- worth the
  user's awareness before running other heavy jobs concurrently with the
  full-scale generation, though the projected RSS itself is well within the
  machine's total capacity for a single batch process.)
- no crash/schema/group-loss: **yes**, confirmed at 500-scale; the new
  fail-loud mismatch guard actively prevents *silent* group-loss at full
  scale too (any additional mismatches will be counted, not swallowed)
- non-degenerate positive coverage: **yes**, ~65% coverage with real rank
  spread (see Phase 3C-3 table)

No streaming/chunked exporter redesign undertaken -- correctly out of scope
per the user's explicit "no premature optimization for performance alone"
instruction; nothing observed at 500-scale indicates non-linear wall-clock
growth, and the RSS projection stays GO even under the conservative band.

**Verdict: GO.** Proceeding to Phase 3D (full TRAIN + full VAL pool
generation; formal TEST still not touched).
