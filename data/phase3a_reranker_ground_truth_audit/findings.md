# Phase 3A: retro candidate-pool reranker ground-truth audit

Issue #101 competitive-breakthrough program, Phase 3 (`real candidate pool
-> real labels -> offline reranker gate`). This document is the required
pre-implementation audit: confirm the existing schema/pipeline rather than
guessing a new one, and record full provenance for the real-label input
data before any pool generation happens. No pool has been generated yet
(that's Phase 3B) -- this is audit + real-label generation only.

Branch `feat/reranker-real-data-gate-101`, base `origin/master@8ecac2f`
(PR #102, Issue #101 Phase 1 diagnostics, merged). Separate worktree from
`renkin-open-state-dominance` (PR #104, open-state dominance) -- that PR is
untouched by this work and stays in draft per the standing prohibition.

## 1-9. Existing schema/pipeline (confirmed against code, not assumed)

All of the following were confirmed by reading `src/candidate.rs`,
`src/pool_export.rs`, `scripts/train_reranker.py`, `scripts/tests/*`, and
`docs/guides/reranker-candidate-pools.md`, and by exercising the real code
(not just reading it -- see "End-to-end validation" below).

1. **Candidate pool generation**: `propose_one_step(group_id, target_smiles,
   rules, config) -> CandidatePool` (`src/candidate.rs:1350`), one rule
   application pass over a target molecule, deduplicated/merged into
   `ReactionCandidate`s by `merge_into_candidates`. `ProposalMode`:
   `Exhaustive` (every rule tried -- what Phase 3 uses throughout, offline
   maximum-coverage), `BondIndexed{top_k}` (mirrors `--bond-index`),
   `ScorerConditioned{input,top_k}` (mirrors an active NN scorer). No CLI/
   binary driver exists yet for pool generation at scale -- confirmed by
   grep, `src/bin/*.rs` has zero references to `pool_export`/
   `propose_one_step`. Building that driver is Phase 3B's job.
2. **Group index generation**: `pool_export.rs`'s `TargetPoolRecord` (one
   row per `(group_id, target)`), cross-checked against candidate rows by
   `group_id` in `verify_index_consistency` -- every `group_id` in the
   candidate rows must have exactly one consistent `(target_id,
   target_smiles)` entry in the index, or export hard-errors.
3. **Labels generation**: `train_reranker.py::load_labels`, schema v1
   (`{schema_version, group_id, target_id, correct_precursor_sets}`),
   frozen in `LABELS_SCHEMA_VERSION`. A candidate row is positive iff
   `tuple(sorted(row["precursor_smiles"]))` exact-string-matches one entry
   of `correct_precursor_sets` (`label_and_split_rows`). This is the
   generation method Phase 3A implements this round (see "Real-label
   generation" below) -- **for real data, not just the schema**.
4. **`target_id` vs `group_id` semantics**: `target_id` is the leakage-safe
   split key (hashed below); `group_id` is one LambdaMART ranking group.
   Multiple `group_id`s CAN share a `target_id` (e.g. two literature routes
   to the same product) -- `pool_export.rs` enforces `group_id` uniqueness
   in the index but never requires `group_id == target_id`. For this
   dataset we set `group_id = target_id` (1 group per USPTO-50k target,
   since each target is exactly one product molecule) -- see "Design
   decision" below for why multi-route products still work under this
   convention.
5. **train/val/test split**: `target_split_bucket` /
   `split_for_target` -- `bucket = int.from_bytes(SHA256(target_id)[:4],
   "big") % 100`; `bucket < 70` train, `< 85` val, else test
   (`TRAIN_MAX_BUCKET=70`, `VAL_MAX_BUCKET=85`). Operates on `target_id`,
   **independent of** which USPTO-50k split (train/val/test) the raw
   reaction came from -- see "Split hygiene" below for why this matters.
6. **Proposal mode**: `Exhaustive` for all of Phase 3 (offline evaluation,
   not mirroring any specific runtime retrieval narrowing).
7. **Stock provenance**: not yet exercised this phase (Phase 3A is
   proposal/labels only, no stock-reachability filtering happens before
   the reranker gate -- `availability`-arm features use stock membership,
   but that's Phase 3E, not this document).
8. **Template provenance**: `data/templates_extracted_500.smi` (used
   throughout Issue #101's Phase 1/2 gates, and reused here for
   consistency) was extracted **exclusively from the USPTO-50k TRAINING
   split** -- confirmed via commit `b4e113a` ("Phase 14: auto template
   extraction from USPTO-50k training set"), `scripts/extract_templates.py`,
   and `docs/benchmark.md`/`README.md`. **This means template extraction
   has zero exposure to the test-split reactions used as Phase 3's retro
   targets and ground-truth labels** -- the leakage direction the user
   flagged first in Phase 3A's instructions never occurs.
9. **Feature schema**: `FEATURE_NAMES_V1`, 18 features
   (`src/candidate.rs:558-591`); indices 14-17
   (`fraction_precursors_in_stock`, `all_precursors_in_stock`,
   `max_template_log_frequency`, `mean_template_log_frequency`) are not
   exercised by this document -- noted here only because the last two are
   unconditionally `missing:true` in every export produced by the current
   code (Phase 3B+ will hit this; only arm H's
   `impute_frequency_features` back-fills them from a train-frozen
   frequency table today).
10. **Gate thresholds**: `GATE_THRESHOLDS` in `train_reranker.py:1151-1155`
    -- `top1_hit_rate_min_delta=0.01`, `mean_reciprocal_rank_min_delta=0.01`,
    `top10_hit_rate_max_regression=0.002`, plus a paired-bootstrap top-1 95%
    CI lower bound `> 0`. Unmodified; not touched this round (Phase 3G's
    job to apply, not this document's).

## Real-label generation: dataset provenance

**Dataset**: `bisectgroup/USPTO_50K` (Hugging Face), config `default`, hub
revision `08a575f0546b2be57242997fd45f684d6814d5a9` (`train`/`val`/`test`
splits, `40008`/`5001`/`5007` rows, fields `id`/`class`/`reactants`/
`product`, atom-mapped SMILES). Already present in the local HF cache
(`~/.cache/huggingface/{hub,datasets}/.../bisectgroup___uspto_50_k/...`
pinned to this exact revision) -- no network fetch was needed or performed
this round.

| file | SHA-256 |
|---|---|
| `uspto_50_k-train.arrow` | `11f66b6245c8901bdcfbcaecd35fb608c695c9302b20f632de2cc39d6f82509a` |
| `uspto_50_k-val.arrow` | `0fa85449257ab2fc17ff5b49c87bc2481b0138f5411d101f075e58a813f9db68` |
| `uspto_50_k-test.arrow` | `f539b551b080418924d565177351fc6d9b5d2f2b622ce71eab3812041cbc2816` |

**Preprocessing command** (one-time raw-dump step, needs `pyarrow`, which
the project's own `.venv` does not have installed -- run with any Python
that has `pyarrow`):

```python
import pyarrow as pa, pyarrow.ipc as ipc, json
path = "<HF_CACHE>/datasets/bisectgroup___uspto_50_k/default/0.0.0/08a575f0546b2be57242997fd45f684d6814d5a9/uspto_50_k-test.arrow"
with pa.memory_map(path, "r") as source:
    table = ipc.open_stream(source).read_all()
with open("data/uspto50k_raw_test_split.jsonl", "w") as f:
    for r in table.to_pylist():
        f.write(json.dumps({"id": r["id"], "class": r["class"],
                             "reactants": r["reactants"], "product": r["product"]},
                            sort_keys=True) + "\n")
```

Produces `data/uspto50k_raw_test_split.jsonl` (5,007 rows, gitignored --
mechanically regenerable from the pinned revision above), SHA-256
`c810404508bbf7a4a5154828c322596c09d0c8c999646616a161271487054550`.

The rest of the pipeline (`scripts/generate_real_labels.py`) needs only the
stdlib plus the new `renkin-canonicalize` binary (`src/bin/canonicalize.rs`,
`cargo build --release --bin renkin-canonicalize`) -- no exotic Python deps
in the committed, repeatable step.

**Raw record accounting** (from `generate_real_labels.py`'s own summary,
`data/reranker_labels_uspto50k_test.summary.json`):

| count | value |
|---|---|
| raw test-split records | 5,007 |
| product SMILES parse failures | 0 |
| reactant-fragment parse failures | 0 |
| unique canonical products (test split) | 5,003 (4 exact within-split duplicates) |
| targets in `sample_full_sorted.jsonl` | 4,903 |
| targets successfully labeled | 4,903 |
| targets unmatched (no ground truth found) | 0 |
| targets with >1 distinct recorded route (test-split only) | 2 |

**Rejection reasons**: none of the pipeline's own steps rejected anything
(0 parse failures, 0 unmatched targets against the existing 4,903-target
corpus). One historical discrepancy is worth recording honestly rather than
re-deriving: the raw test split has 5,007 records / 5,003 unique canonical
products, but the existing `data/uspto50k_test.smi` (committed mid-2026,
commit `00706e6`, no generator script checked in) accepted only 4,907
lines / 4,903 unique products -- **100 raw unique products are absent from
that historical file**. Investigated this session: not explained by parse
failure (0/100), multi-fragment products (0/100), or an obvious element/
heavy-atom-count pattern (11-60 heavy atoms, no size-based filter visible).
The original one-off generation script was never committed, so the exact
rejection rule from mid-2026 isn't reconstructable from the repo. This does
**not** block Phase 3A: `sample_full_sorted.jsonl` (the corpus every Phase
1/2 gate in this program has used) already reflects whichever 4,903 targets
were accepted back then, and this document reuses that existing target set
as-is rather than re-deriving a new one -- so the historical rejection
reason is a known gap, not a live discrepancy in this round's own pipeline.

## Split hygiene: test-split-only labels (a design decision, not a default)

`correct_precursor_sets` is populated **only from raw rows in the USPTO-50k
TEST split** -- reactant sets recorded for the same product elsewhere in
train/val are never folded in, even though USPTO-50k's split is by reaction
(not by product), so a handful of products could in principle have
additional recorded routes there. Checked directly: of the 100 raw test
products absent from the accepted target corpus, only 1 also appears as a
train-split product and 2 as train-split reactant fragments -- cross-split
product overlap is rare here, but the decision doesn't depend on that being
small.

**Why test-only, not train+val+test**: `train_reranker.py` re-splits the
4,903 targets into its *own* train/val/test buckets by `SHA256(target_id)`,
completely independent of which USPTO-50k split the target's product SMILES
originated from. Enriching a target's ground truth from the USPTO train
split would mean a target that lands in the reranker's *test* bucket could
have labels partly derived from the same reactions
`templates_extracted_500.smi` was built from -- the leakage direction the
user's Phase 3A instructions named explicitly
("validation/testの情報をtemplate extraction...へ漏らさない"), just running
train-to-test instead of test-to-train. Within the test split itself,
aggregating multiple distinct routes for the same product (found for 2 of
4,903 targets) is safe and intentional -- that's exactly what the schema's
list-of-lists `correct_precursor_sets` is for, and it never crosses a split
boundary.

## Design decision: `group_id = target_id`

USPTO-50k gives each target exactly one canonical product molecule, so
Phase 3 sets `group_id = target_id` for every labeled row (verified: 4,903
unique `group_id`s == 4,903 unique `target_id`s == 4,903 label rows,
asserted in `generate_real_labels.py` and re-verified by loading the
output through the real `train_reranker.py::load_labels`). This does not
lose the 2 multi-route targets found within the test split -- both keep all
their distinct routes as separate entries inside one group's
`correct_precursor_sets` list, matching `GroupLabel`'s intended semantics
(the docstring: "multiple accepted correct precursor multisets per group
are allowed"). The `group_id != target_id` case in the schema exists for a
different scenario (multiple independent *datasets* sharing a product) that
doesn't arise for a single-source corpus like this one.

## Canonicalization: RENKIN's own canonicalizer, not RDKit

`train_reranker.py::label_and_split_rows` matches candidates to labels via
**exact string equality** on `tuple(sorted(precursor_smiles))`. RENKIN's
`propose_one_step`/`merge_into_candidates` write `precursor_smiles` using
`chem_env::to_canonical` (backed by `chematic`'s canonicalizer) -- which
produces a **different textual form** than both RDKit's canonical SMILES
and the older canonical form baked into `data/uspto50k_test.smi` (bracket-
atom aromatic notation, from an earlier chematic version). Confirmed
empirically: feeding `data/uspto50k_test.smi`'s own stored SMILES back
through the current canonicalizer changes ~99.75% of a 2,000-row sample
(1,995/2,000) -- the stored strings are stable identifiers, not literal
current-canonical text, and were never meant to be compared byte-for-byte.

Consequently, real labels are canonicalized through a new small binary,
`src/bin/canonicalize.rs` (`renkin-canonicalize`, registered in
`Cargo.toml`) -- batch stdin/stdout wrapper around
`chem_env::{mol_from_smiles, to_canonical}`, mirroring the existing
`renkin-fp` batch-utility pattern (`src/bin/fp.rs`). This guarantees the
labels file's `correct_precursor_sets` are written in the *exact same*
canonical form that `propose_one_step` will produce for `precursor_smiles`
at pool-export time, since both go through the identical function in the
same built binary. Using RDKit or any other toolkit's canonical form here
would have silently produced near-zero label matches downstream, with no
error to signal the mismatch.

## End-to-end validation (not just schema-level)

Two independent checks, beyond "does the loader accept the file":

1. **Real loader round-trip**: `train_reranker.py::load_labels(...)` loads
   all 4,903 rows with zero errors (sorted-fragment check, non-empty-set
   check, no duplicate/conflicting `group_id` -- all pass on real data, not
   synthetic fixtures).
2. **Reachability spot-check** (`examples/label_spotcheck.rs`, one-off, not
   a permanent test): for the first 40 labeled targets, ran the real
   `propose_one_step(Exhaustive)` against `data/templates_extracted_500.smi`
   (500 rules) and checked whether the labeled ground-truth precursor set
   appears verbatim among the generated candidates. Result: **27/40 (68%)
   conditional-positive at the exhaustive one-step level, 0/40 targets
   produced zero candidates**. This is the same "conditional coverage"
   concept `train_reranker.py`'s metrics module measures formally
   (Phase 3E+), and a healthy, plausible number given the 500-template
   library is a real subset of all possible USPTO-50k disconnections (not
   every reaction type is covered) -- it also confirms the canonicalization
   fix above actually works end-to-end: a broken alignment would show
   near-0/40, not 27/40.

## Deliverables this round

- `src/bin/canonicalize.rs` + `Cargo.toml` `[[bin]] renkin-canonicalize`
  entry -- new, small, general-purpose (not reranker-specific) batch
  canonicalization utility.
- `scripts/generate_real_labels.py` -- the repeatable, stdlib-only label
  generation pipeline (full docstring covers the one-time raw-dump
  prerequisite, split hygiene, and canonicalization rationale above).
- `data/reranker_labels_uspto50k_test.jsonl` (gitignored, 4,903 rows) +
  `data/reranker_labels_uspto50k_test.summary.json` (gitignored,
  machine-readable accounting matching the table above) -- both
  deterministically regenerable from the committed script plus the pinned
  HF revision.
- `examples/label_spotcheck.rs` -- one-off validation, kept for
  reproducibility, not part of the CI test suite (needs gitignored data
  files; confirmed `cargo test --workspace` does not execute it, only
  compiles it as part of the normal build-all-targets pass).
- This document.

## Explicitly not done this round (per the standing Phase 3 scope)

No candidate pool has been generated (Phase 3B). No pool-generation driver
exists yet (Phase 3B's job). No baseline arms, LightGBM training, bootstrap,
or formal gate have run (Phase 3E-3G). PR #104 (open-state dominance)
untouched, still draft. No adaptive beam, no timeout/beam-width change, no
500/4,903-target route-search run, no posting to Issue #101, no
tag/release.
