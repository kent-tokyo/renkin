# Phase 3D.5: Canonical Identity Safety Audit (Issue #101)

Triggered by the user's explicit instruction after seeing Phase 3D's 123
target_id-mismatch exclusions (109 train + 14 val, 0.27%/0.28%): before
starting LightGBM training, determine whether the same canonicalization
non-idempotence corrupts **precursor** ground-truth labeling (candidate/
label matching is exact-string-based), not just target identity. This
document covers Steps 1-7 (decision) and 8 (issue-ready reproducer, not
filed). Step 0 (provenance snapshot, code commit to PR #105) is
`provenance.md` in this directory. **LightGBM training has not started.
Formal TEST has not been generated or evaluated. PR #104 untouched. PR #105
still draft.**

## Step 1: target canonicalization mismatch ledger (123 rows)

`scripts/phase3d5_build_mismatch_ledger.py` -> `mismatch_ledger.json`. For
every one of the 109 train + 14 val mismatches:

- **123/123**: `requested_target_id != propose_one_step`'s derived form
  (confirms every flagged mismatch is real, none spurious).
- **123/123**: the drift is a single step then stable (`A -> B -> B`) --
  zero cases needed a third canonicalization pass, zero cycles.
- **123/123**: `map_cleared_representation` (fresh `--clear-atom-maps` run
  on the raw mapped USPTO product) **exactly equals** `requested_target_id`
  -- the stored label value is reproducible from the documented pipeline; it
  is not itself corrupted or drifted. The divergence is entirely in
  `propose_one_step`'s *second*, map-free re-canonicalization pass.
- **123/123**: a label row exists for every mismatched group_id (all were
  genuinely labeled training/val examples, not orphans).

**This flips the initial framing**: the label corpus is internally
consistent and reproducible; `propose_one_step`'s internal re-derivation is
the outlier, re-deriving an identity nobody stored for ~0.27% of inputs. See
the corrected root-cause section in
`data/phase3c_500_target_feasibility/findings.md` (an earlier version of
that doc incorrectly attributed this to "atom-map presence sensitivity" --
disproven by this audit's own reproducer, where atom maps are absent from
both sides of the divergence).

## Step 2: corpus-wide target idempotence audit

`target_ids_{train,val,test}_unique.txt` -> canonicalize -> canonicalize
again, compare all three states. Counts (deduplicated unique target_id
strings per split, matching Round 2's own dedup counts exactly -- 39,668 /
4,924 / 4,903):

| corpus | n unique | n non-idempotent | rate | pattern |
|---|---|---|---|---|
| TRAIN | 39,668 | 109 | 0.275% | 100% single-drift-then-stable |
| VAL | 4,924 | 14 | 0.284% | 100% single-drift-then-stable |
| **TEST** | 4,903 | **13** | 0.265% | 100% single-drift-then-stable |

**New finding, not previously known**: the quarantined TEST corpus (never
before audited for this -- Phase 3B/3C only ran candidate generation on
100/500 of the 4,903, this is a string-only check across all 4,903) shows
the identical ~0.27% rate. This is a corpus-wide, split-independent
property of `to_canonical`, not an artifact of one label-generation script.
No candidate generation was run against TEST -- string-level canonicalize
only, per the explicit instruction.

## Step 3: precursor canonicalization audit

Extracted every unique precursor SMILES string from (a) ground-truth
`correct_precursor_sets` in the labels files, and (b) `precursor_smiles` on
every row of the full TRAIN/VAL candidate pools (985,896 and 131,163 unique
strings respectively), re-canonicalized each once, compared:

| source | split | n unique | n drifted | rate |
|---|---|---|---|---|
| candidate `precursor_smiles` | train | 985,896 | 1,959 | 0.199% |
| candidate `precursor_smiles` | val | 131,163 | 324 | 0.247% |
| label `correct_precursor_sets` | train | 43,526 | 88 | 0.202% |
| label `correct_precursor_sets` | val | 6,797 | 19 | 0.280% |

Same order of magnitude and single-drift-then-stable pattern as the target
strings, on **both** sides of the eventual match -- this confirms the
phenomenon is not confined to target identity, and that it is two-sided
(neither the label side nor the candidate side is uniquely "the stable
one"). This is exactly why Step 4's false-negative count is non-zero: a
false negative requires the label-side and candidate-side representations
of the same precursor to have landed on *different* attractors, which can
happen regardless of which side drifted.

**Character of the drift, checked with an independent toolkit (RDKit
2026.03.4, confined to this audit only -- never wired into
`train_reranker.py`)**: of the 2,283 drifted candidate-precursor pairs
(train 1,959 + val 324), RDKit could parse both sides of 2,245; **2,245/2,245
(100%) have identical InChI** -- same molecule, same absolute
stereochemistry, on every checkable case. ~98.9% of drifts are pure
stereo-descriptor (`@`/`@@`) re-encodings with the rest of the string
byte-identical; the remaining ~1.1% show the same full-reordering pattern
already characterized for target strings. The 38 RDKit-unparseable pairs
(1.7% of drifts) fail identically on both sides of each pair for reasons
unrelated to the stereo/reordering drift itself (not investigated further --
orthogonal to this audit's question, and too small to matter: 38 of
~1.1M total precursor strings).

**No chirality-flip bug**: this was the most serious open question raised by
the user, and it is resolved negatively -- zero cases where RDKit's
stereo-aware InChI disagreed between the two canonical forms of a drifted
precursor pair.

## Step 4: false-negative diagnostic (the number that actually matters)

Reused `train_reranker.py::label_and_split_rows` unmodified for the
baseline (real exact-string-multiset matching, real split assignment).
For every group the real loader currently marks zero-positive, checked
whether pushing both the candidate's precursor SMILES and the label's
`correct_precursor_sets` through one more `renkin-canonicalize` pass (using
the Step 3 canon maps) produces a match the raw string comparison misses.
**Diagnostic only -- no file, label, or matching-policy change.**

| split | groups (labeled) | coverage before | coverage after normalization | groups flipped zero-to-positive |
|---|---|---|---|---|
| train | 39,798 | 66.96% (26,648/39,798) | 67.15% (26,723/39,798) | 75 / 13,150 zero-positive (0.57%) |
| val | 4,916 | 66.17% (3,253/4,916) | 66.50% (3,269/4,916) | 16 / 1,663 zero-positive (0.96%) |

**This is the decision-relevant number the user pre-registered a threshold
for.** The user's own framing: "66.9% -> ~67.0%" is proceed; "66% -> 72%"
would be a different situation requiring the 1/3 zero-positive population to
be re-examined as a false-coverage-gap problem. The measured shift is
+0.19pp (train) and +0.33pp (val) -- squarely in the "proceed" range, fully
explained by the already-characterized single-drift-then-stable mechanism,
not a large hidden pool of miscounted zero-positive groups.

## Step 5: independent structural check (cut down per review)

A dedicated graph-isomorphism layer was **not built**. Research confirmed no
canonical-string-independent, atom-map-agnostic equivalence function exists
in chematic or RENKIN that is safe to use at this scale (VF2 exists in
chematic but is private to RENKIN, stereo-blind, and the project has
already deliberately removed a VF2-based identity fallback elsewhere due to
false positives from subgraph-only matching -- see `src/chem_env.rs`'s own
comment on this). Instead, the evidence already in hand is stronger than a
graph-isomorphism check would add:

- All 123 target mismatches: `map_cleared_representation ==
  requested_target_id` (both forms trace to the identical raw mapped input
  via a documented, reproducible path) and `A -> B -> B` (both forms are
  individually stable).
- 2,245/2,245 RDKit-parseable precursor drift pairs: identical InChI.

Two canonicalizations of the same input molecule, both reproducible from
the same source, one of them independently InChI-confirmed at scale, is
sufficient identity evidence for this audit's purpose without a bespoke
graph comparator.

## Step 6: benchmark quarantine re-audit (widened, not just the 123)

Rather than checking only the 123 known mismatches against the quarantine
set (done first, 0 hits -- see below), built the complete widened check
using Step 2's full corpus-wide idempotence data: for each of
TRAIN/VAL/TEST, `widened_set = stored_target_ids ∪ their_own_one-pass_
canonical_form`. This catches *any* drift-masked overlap across the entire
corpus, not just the 123 already-known cases.

| check | overlap found |
|---|---|
| 123 mismatch targets (both requested and derived form) vs. original quarantine (4,903) | 0 |
| TRAIN widened (39,777) vs. TEST widened (4,916) | **0** |
| VAL widened (4,938) vs. TEST widened (4,916) | **0** |
| TRAIN widened (39,777) vs. VAL widened (4,938) | **0** |

**No new overlap discovered.** The original decontamination counts stand
unchanged: 81 train + 7 val benchmark overlaps removed, 62 additional val
target_ids removed for cross-split dedup (train wins) -- verified against
`data/phase3a_reranker_ground_truth_audit/round2_split_hygiene.md` Section
B/C, not re-derived from memory. Per the user's own CASE A/B/C rule, "new
overlap > 0" would force treating the current train/val labels/pools as
unusable for formal training; the measured result is 0, so this condition
does not trigger.

## Step 7: decision -- CASE A

All three of the user's CASE A conditions hold:

1. **Precursor false-negative increment is small and explained**: +0.19pp
   (train) / +0.33pp (val), fully accounted for by the single-drift-then-
   stable canonicalization mechanism characterized in Steps 1-3, not a
   large unexplained gap.
2. **Benchmark quarantine leakage = 0**: confirmed by the widened check
   (Step 6), stronger than the narrow check the user asked for.
3. **The 123 exclusions are mechanical, not label/outcome-dependent**: the
   `TargetIdMismatch` guard added in Phase 3C fires purely on string
   comparison before any label is consulted -- it cannot selectively drop
   positive-label groups.

**Verdict: proceed with the current TRAIN/VAL pools as-is.** No
regeneration of labels/groups/split-manifest/pools. The 123 excluded
targets are recorded as `proposal_generation_excluded_target_id_mismatch`
(already the case via `ProposalStatus::TargetIdMismatch`), not silently
folded into the zero-positive coverage-gap count -- see the corrected
denominators already added to `data/phase3c_500_target_feasibility/findings.md`
and `data/phase3d_full_pool/findings.md` in the prior round.

**Denominator, explicit** (per the user's CASE A instruction) -- three
different counts are in play per split, kept distinct rather than collapsed:

| | train | val | meaning |
|---|---|---|---|
| all group records | 39,927 | 4,931 | post-Round-2-decontamination raw group count |
| minus target_id-mismatch (123) | 39,818 | 4,917 | groups that *could* train if candidates existed |
| minus target_id-mismatch AND zero-candidate | 39,798 | 4,916 | groups that actually contribute >=1 pool row |

The honest "usable training groups" denominator for a ranking loss is the
third row -- **39,798 train + 4,916 val = 44,714** -- since a group with
zero candidate rows contributes nothing to pairwise/listwise ranking loss
regardless of why it has none. The 20 train / 1 val zero-candidate groups
(Phase 3D's own accounting) remain a real, disclosed proposal-coverage gap,
correctly counted in the coverage-rate denominator (Phase 3D's findings.md),
just not in the ranking-loss training-group count. The 123 target_id-mismatch
exclusions are the only ones this document (Phase 3D.5) is responsible for;
they are mechanical (string comparison only, label-blind) as required by
condition 3 above.

## Step 8: upstream chematic issue (not filed)

Issue-ready reproduction package: `chematic_issue_ready_reproducer.md` in
this directory. Contains the minimal reproducer, chematic version/commit
pinning, independent RDKit confirmation, corpus-wide frequency measurements
across three independent corpora, and a flagged (not confirmed) area of
chematic's own source that may be relevant to maintainers
(`canonical_partition::VertexColor.atom_map`). **Not posted to GitHub** --
outward-facing action requiring the user's explicit go-ahead, per Phase
3D.5's own scope.

## What was explicitly NOT done, per scope

- No LightGBM training.
- No formal TEST pool generation or evaluation (Step 2's TEST audit was
  string-only canonicalization, never `propose_one_step`/candidate
  generation).
- No runtime reranker integration.
- No changes to PR #104.
- No Ready-flip or merge of PR #105 (still draft).
- No GitHub issue posted.
- No tag/release.
- No 4,903-target route-search comparison run.
