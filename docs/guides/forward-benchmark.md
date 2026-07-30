---
title: "Forward-Prediction Benchmark Protocol and Harness"
description: "The frozen Phase 0 benchmark protocol and the Phase 1 renkin-forward benchmark harness for measuring forward-prediction proposal coverage and ranking quality, separately."
---

# Forward-Prediction Benchmark Protocol and Harness

This guide documents `renkin-forward benchmark`: a standalone, deterministic
harness that measures whether RENKIN's forward-prediction candidate pool
*contains* the right answer (proposal coverage) and, separately, how well it
*ranks* it (conditional/end-to-end top-1/5/10, MRR, NDCG@10). It is "PR A"
(Phase 0 + Phase 1) of
[issue #61](https://github.com/kent-tokyo/renkin/issues/61).

**Scope.** This PR delivers the frozen protocol and the harness only. It
does **not** attempt proposal-coverage improvements, a forward-specific
reranker, an acceptance gate, or a generative-model decision — those are
Phase 2 through Phase 5 of issue #61, each its own future PR. Nothing in
this guide or the harness's fixture run is a production-accuracy claim.

## Phase 0 — the frozen protocol

Freezing this design *before* writing the harness (and before any real
corpus is run through it) is the point: template extraction, normalization,
and thresholds must never be decided by looking at validation/test data.

### The corpus is never bundled

Like RENKIN's existing [ORD evidence import](reaction-evidence.md#importing-from-ord-open-reaction-database),
a real reaction corpus (ORD, USPTO, or anything else) is **user-supplied**.
This repository does not, and will not in this phase, commit or bundle a
real benchmark corpus. `renkin-forward benchmark --corpus <path>` takes a
local file path; nothing is fetched, scraped, or downloaded.

**The redistribution question doesn't stop at the input.** `--output-rows`
carries the corpus's own `reaction_id` and raw, un-canonicalized
`reactants_original` text verbatim onto every row, alongside the
canonicalized/derived fields — this is what makes a single row
self-describing (see "Row schema" below), but it also means the output is
not a strictly-derived summary. Before sharing or publishing benchmark
output produced from a restricted corpus, confirm your redistribution
rights cover the identifiers and reactant text you supplied, not just the
report's aggregate metrics.

### Corpus schema (`FORWARD_BENCH_CORPUS_SCHEMA_VERSION = 1`)

JSONL: one JSON object per line, one line per raw reaction record.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `schema_version` | integer | yes | Must equal `1`. A different value is counted (`wrong_schema_version`) and rejected, never guessed at. |
| `reaction_id` | string | yes | The corpus's own identifier (e.g. a USPTO/ORD/patent record ID). Not used for identity or splitting — see below. |
| `reactants` | array of SMILES strings | yes, non-empty | The true reactants for this outcome. Reagents should be separated out by the corpus *preparation* step where the source dataset permits it — this harness canonicalizes and matches whatever it is given, it does not itself classify reactant vs. reagent. |
| `accepted_products` | array of arrays of SMILES strings | yes, non-empty, every inner array non-empty | One or more accepted correct product multisets for this reaction. More than one entry means more than one reported/acceptable outcome is correct (e.g. competing literature reports) — a candidate matching *any* entry counts as correct. |
| `reaction_class` | string | no | Free-form label, used only for the `reaction_class` breakdown dimension. |
| `group_key` | string | no | Explicit patent-family/chronological grouping key (see "Leakage prevention" below). Omit to use the deterministic reactant-hash fallback. |

A blank line is skipped silently (whitespace, not data). Every other
rejection increments a counter in `CorpusLoadStats` and is never silently
dropped:

- `malformed_json` — the line isn't even valid `CorpusRow` JSON (missing or
  wrong-typed required field, broken syntax). No `reaction_id` can be
  trusted from a line that failed to parse at all, so this category alone
  produces no output row, only the counter plus a bounded
  (`CorpusLoadWarning`, capped like every other bounded-diagnostics list in
  this crate — see [Forward Enumeration](forward-enumeration.md)) diagnostic.
- `wrong_schema_version`, `empty_reactants_or_products`, `unparseable_smiles`
  — the line has a `reaction_id`, so it becomes a real output row with
  `failure_reason: "input_invalid"` and every other field null/zero/empty —
  see "Row schema" below.
- `duplicate_records_merged` — see "Reaction identity" next.

### Reaction identity vs. group key — never conflated

Mirroring the retrosynthesis reranker's `target_id`/`group_id` distinction
(`scripts/train_reranker.py`, PR #59), two different hashes exist here and
are never used for each other's purpose:

- **Reaction identity** (dedup key): SHA-256 of the domain-separated
  (sorted canonical reactants, sorted canonical accepted-product multisets)
  pair. Two corpus lines that agree on both are the same record; the second
  and any further occurrence increments `duplicate_records_merged` and is
  dropped, keeping the first-seen metadata.
- **Group key** (the leakage-safe *split* key): the corpus's own
  `group_key` if supplied (Phase 0: "prefer patent-family or chronological
  grouping where metadata permits"), otherwise SHA-256 of the sorted
  canonical **reactants alone** — deliberately excluding accepted products.
  Two reactions that share reactants but report *different* accepted
  products (a genuinely ambiguous/multi-outcome reaction) still land in the
  same split; splitting them apart would leak the reactants' identity
  across train/val/test.

### Leakage-safe splitting

`split_bucket(group_key) = SHA-256(group_key)[0..4] as u32 mod 100`, then:

- `[0, 70)` → **train**
- `[70, 85)` → **val**
- `[85, 100)` → **test**

Same algorithm and the same 70/85 cutoffs as
`scripts/train_reranker.py`'s `TRAIN_MAX_BUCKET`/`VAL_MAX_BUCKET` — kept
numerically identical across the two independent benchmarks (Rust and
Python cannot share a literal constant, so both are hand-kept in sync; if
either changes, change both and both docs). Splitting is *always* computed
this way by the harness itself — a corpus cannot declare its own split and
have the harness simply trust it, since that would make the harness's
leakage guarantee only as strong as whatever produced that declaration.
Supplying a real `group_key` (so a chronological or patent-family boundary
is respected) is the intended way to get a more realistic split than the
reactant-hash fallback.

A row whose input never canonicalized (`failure_reason: "input_invalid"`)
has no group key and reports `split: "unknown"` — excluded from every
per-split aggregate, counted only in the overall one.

### Template source modes

Phase 0 requires four named modes; **only the first three are implemented
in this PR** (`ScorerConditioned` requires a scorer, which does not exist
until Phase 3/4):

1. **`embedded`** (default) — `renkin::chem_env::default_rules()` only.
2. **`file`** — *only* the rules in `--templates <path>`, loaded via the
   same strict loader `predict`/`enumerate` use for an explicit file (hard
   error if missing/unreadable/empty). **Never merged with the embedded
   defaults** — this is the opposite of `predict`/`enumerate`'s
   `--templates`, which *extends* the embedded set. Phase 0 is explicit that
   the harness must not "silently substitute the embedded fallback corpus
   for an intended external template set", and that cuts both ways: an
   explicit file must not be silently diluted by the embedded set either. A
   stray `--templates` under the default `embedded` source is a hard error,
   not a silently-ignored flag.
3. **`train-extracted`** — mechanically identical to `file`: the rule file
   is used exactly as given. The only difference is the label recorded in
   `provenance.template_source`. **This harness cannot verify that the
   file's templates were actually extracted from the train split only** —
   that is the responsibility of whatever produced the file (a future
   Phase 2 extraction tool). Mislabeling a val/test-derived template set as
   `train-extracted` would not be caught here; it would show up as
   suspiciously strong val/test metrics, which is exactly why per-split
   breakdown is a required report field.
4. **`scorer-conditioned`** — named by the frozen protocol so the mode
   space is documented in full, but rejected with a clear error naming
   Phase 3/4 if requested. Not silently downgraded to another mode.

### What counts as "correct"

A candidate is correct if its canonical product multiset exactly equals
*any* entry in `accepted_products` (stereochemistry-aware), or, for the
separate stereochemistry-ignored dimension, if the stereo-flattened forms
match (see "Stereochemistry comparison" below).

**Byproduct/leaving-group policy is a corpus-preparation decision, not a
harness heuristic.** RENKIN's templates routinely omit reagents, leaving
groups, and small byproducts (see `predict`'s
[atom/charge-balance diagnostic](forward-prediction.md)) — whether
`accepted_products` for a given reaction should include such a byproduct is
up to whoever curates the corpus, consistently, for every row. The harness
performs no implicit normalization here beyond canonicalization; if a
corpus's convention differs between rows, results between those rows are
not comparable. Document your corpus's own convention alongside it.

### Stereochemistry comparison

`predict`'s `canonical_smiles` output does distinguish stereochemistry: both
tetrahedral (`@`/`@@`) and double-bond (`/`/`\`) markers survive
canonicalization and are compared exactly for the stereochemistry-aware
dimension.

For the stereochemistry-*ignored* dimension, the flattened form is produced
**structurally**: every atom's chirality is reset and every E/Z directional
bond is normalized to a plain single bond, via chematic's public
`MoleculeBuilder` API (rebuilding the molecule atom-by-atom and bond-by-bond
with those two adjustments, then re-canonicalizing) — not a text-level
strip of `@`/`@@`/`/`/`\` characters. A textual strip was tried first and
rejected: `canonical_smiles` decides bracket-vs-organic-subset notation at
canonicalization time (an atom needing brackets only for its now-removed
atom-map or stereo marker no longer needs them once cleared *before*
canonicalizing), so stripping the already-generated *text* left a redundant
bracket behind (`[C]`/`[OH]`) instead of collapsing to the true canonical
spelling (`C`/`O`) — silently comparing against a non-canonical string. This
was caught empirically while fixing a related leakage bug in the atom-map
handling below, and is exactly the failure mode a structural clear avoids.

If clearing stereochemistry from an already-canonical SMILES ever fails to
re-parse (not observed, but not assumed impossible), the comparison for
that whole row reports `stereochemistry_ignored_outcome: "unsupported"` —
**never** a silent fallback to the stereochemistry-aware string, which
would fabricate a "no worse than aware" result the harness never actually
computed. See `bench::tests::stereo_ignored_canonical_collapses_tetrahedral_and_ez`
and `stereo_ignored_canonical_never_falls_back_to_the_stereo_aware_string`.

## Phase 1 — the harness

### Usage

```bash
renkin-forward benchmark \
  --corpus corpus.jsonl \
  --output-rows rows.jsonl \
  --output-report report.json
```

`--output-rows` is required and always a file (row-level output can be
large; it is never printed to stdout). `--output-report` is optional — if
omitted, the report is printed to stdout instead (this subcommand's only
stdout output). See `renkin-forward benchmark --help` for every flag,
including `--template-source`/`--templates`.

### Row schema (one row per reaction, `FORWARD_BENCH_REPORT_SCHEMA_VERSION = 2`)

| Field | Meaning |
|---|---|
| `reaction_id`, `source_line`, `split` | Identity and leakage-safe split. |
| `leakage_group_id` | The group key `split` was actually derived from (explicit corpus `group_key`, or the deterministic reactant-hash fallback) — `null` only for `input_invalid` rows, where reactants never canonicalized and no group key could ever be computed (a `prediction_error` row still has one: its reactants canonicalized fine, prediction itself just failed). Carried onto every row so the leakage-safety guarantee is auditable from `rows.jsonl` alone. |
| `reaction_class` | Pass-through from the corpus, or `null`. |
| `reactants_canonical`, `accepted_products_canonical` | Canonicalized inputs, exactly as matched against. |
| `num_reactants` | `reactants_canonical.len()`. |
| `num_products` | `accepted_products_canonical[0].len()` — the primary accepted answer's product count (a reaction with several accepted outcomes of different arity still gets one well-defined count). |
| `has_stereochemistry` | Auto-detected from `@`/`/`/`\` in any canonical reactant/accepted-product SMILES — never trusted from corpus metadata, since it is fully derivable. |
| `candidate_count`, `raw_outcomes` | Final merged-candidate count, and total `run_reactants` outcomes attempted before validity/no-op filtering or merging (the denominator `invalid_product_rate`/`no_op_rate` are computed against). |
| `correct_candidate_present`, `best_correct_rank` | Stereochemistry-aware presence/best rank (0-based). |
| `best_correct_rank_stereo_ignored` | Best rank under the loosened comparison, present only when the outcome below is `hit` — always `<= best_correct_rank` when both are present. |
| `top1_hit`, `top5_hit`, `top10_hit` | `best_correct_rank < 1/5/10`. |
| `stereochemistry_aware_hit` | Identical to `top10_hit`, reported under its own name so it sits directly next to `stereochemistry_ignored_outcome` for a same-row before/after read. |
| `stereochemistry_ignored_outcome` | One of `hit`, `no_hit`, or `unsupported` (a tri-state, not a plain bool, so an uncomputable comparison is never conflated with a clean miss — see "Stereochemistry comparison" above). **`hit` while `stereochemistry_aware_hit` is `false` is the diagnostic signal for "constitution right, stereochemistry wrong."** |
| `invalid_candidate_count`, `no_op_candidate_count` | From the underlying `ForwardStats` (`invalid_outcomes_rejected`/`no_op_outcomes_rejected`). |
| `application_warning_count`, `application_error_count`, `templates_attempted`, `templates_matched`, `graph_rules_skipped`, `rules_loaded` | From the underlying prediction report (`ForwardStats`) — `graph_rules_skipped` (empty-`smirks` rules, never counted as a parse failure) and `templates_matched` (a subset of `templates_attempted` that had at least one slot match) are per-row, not just summed in the report header. |
| `elapsed_ms` | Wall-clock time for this reaction's one `predict_products_detailed` call. **Not deterministic across runs** — see below. |
| `failure_reason` | One of `hit_top1`, `hit_top5`, `hit_top10`, `hit_beyond_10`, `correct_absent_empty_pool`, `correct_absent_nonempty_pool`, `input_invalid`, `prediction_error`. Deliberately coarse: classifying *why* a template failed to apply (missing forward SMIRKS, atom-mapping mismatch, stereochemistry mismatch, …) is Phase 2 territory, not this harness. |
| `provenance` | `{renkin_forward_version, template_source, rules_file_sha256}` — duplicated onto every row (not just the report header) so a single row, taken out of context, still fully describes what produced it. |

### Aggregate metrics

Computed once overall, once per split (`train`/`val`/`test`), and once per
breakdown bucket (`reaction_class`, `num_reactants`, `num_products`,
`stereochemistry_presence`, `template_source`, `candidate_pool_size`,
`failure_reason`):

- `valid_input_rate` — valid rows (a real candidate pool was attempted) /
  total rows.
- `proposal_coverage` — rows with a correct candidate present in their pool
  / valid rows.
- **`conditional`** (only rows with a correct candidate present — "given
  that ranking is possible at all, how good is it") and **`end_to_end`**
  (every valid row; a coverage miss contributes 0, it is never excluded)
  — **never conflated**, each reporting `top1_hit_rate`/`top5_hit_rate`/
  `top10_hit_rate`/`mrr`/`ndcg_at_10`; `conditional` additionally reports
  `mean_best_correct_rank`/`median_best_correct_rank` (undefined,
  meaninglessly, for `end_to_end`, since a miss has no rank).

  <!-- ponytail: single-relevant-item NDCG@10 -->
  `ndcg_at_10` uses the single-relevant-item convention (ideal DCG = 1.0,
  i.e. exactly one accepted product multiset expected at rank 0), not full
  graded relevance across every candidate that happens to match one of
  several accepted outcomes. Issue #61 asks for one NDCG@10 number, not
  multi-outcome credit assignment.
- `n_raw_outcomes` — summed `raw_outcomes` over valid rows: the explicit
  denominator for both rates below, reported directly rather than left for
  a reader to re-derive from `rows.jsonl`.
- `invalid_product_rate`, `no_op_rate` — summed `invalid_candidate_count`/
  `no_op_candidate_count` divided by `n_raw_outcomes`, across valid rows.
- `candidate_count_distribution`, `latency_ms` — `{min, p50, p90, p95, max,
  mean}`, nearest-rank percentiles.

### Determinism

Two runs on the same corpus, rules, and `--template-source` produce
byte-identical output **except**: each row's `elapsed_ms`, and every
`latency_ms` block in the report (overall, per-split, per-breakdown-bucket).
Every other field — including all of `candidate_count_distribution` (which
is over *counts*, not timings) — is stable. See
`tests/bench.rs::benchmark_is_deterministic_modulo_timing_fields` for the
enforced check.

### Fixture corpus

`crates/renkin-forward/tests/fixtures/forward_bench_corpus.jsonl` is a
small, **hand-authored, synthetic** corpus — not a real benchmark, and not
an accuracy claim. It exercises every `failure_reason` the harness can
emit, `duplicate_records_merged` accounting, `malformed_json` accounting,
and both stereochemistry outcomes, all against the embedded default rule
set. See `tests/fixtures/README.md`: every `accepted_products` entry was
derived by actually running `renkin-forward predict --report` against the
fixture's reactants and reading off a real candidate at the intended rank
— never hand-guessed chemistry.

One observation surfaced while building it, and shaped a design decision
rather than getting papered over: for the same two reactants
(`C[C@H](N)C(=O)O` + `CCO`), the specific tetrahedral-tag spelling
(`[C@H]` vs. `[C@@H]` — **opposite enantiomers, not two spellings of one
thing**) of the rank-5 candidate differed depending on whether the
reactants were passed to `predict_products_detailed` in the corpus's
original text/order or in this harness's own pre-canonicalized,
pre-sorted rewrite of them. That is a construction-path dependency in the
prediction path itself (the same class of issue as the chematic 0.8.1
`canonical_smiles` fix — see the `[Unreleased]` CHANGELOG entry above it),
and it matters here specifically: a benchmark must measure the engine as a
caller actually invokes it, not the harness's own preprocessing. Feeding
pre-normalized reactants would have made every stereo-bearing reaction's
`stereochemistry_aware_hit` an artifact of this harness rather than a fact
about the engine.

The fix: [`BenchReaction`] keeps the corpus's reactant text verbatim, in
the corpus's own order, in a separate `reactants_original` field, and
**that**, not `reactants_canonical`, is what gets passed to
`predict_products_detailed`. `reactants_canonical` remains what reaction
identity, the group key, `has_stereochemistry`, and every reported row
field are computed from. Root-causing exactly where in the prediction path
the tag-spelling divergence originates is left for Phase 2 — this harness's
job was to not manufacture the discrepancy itself, which the check above
confirms it no longer does.

## Non-goals for this PR

Everything from Phase 2 onward in issue #61 is explicitly out of scope
here:

- proposal-coverage improvements (Phase 2);
- a forward-specific learned reranker or its offline evaluation (Phase 3);
- the numerical acceptance gate (Phase 4);
- any generative-model decision (Phase 5);
- a real-corpus accuracy claim — no external corpus has been run through
  this harness as part of this PR; the only reported numbers come from the
  synthetic fixture above and carry no production-accuracy meaning.

## Next steps

- [Forward Reaction Prediction](forward-prediction.md) / [Forward Enumeration](forward-enumeration.md) — the engine this harness measures.
- [Reaction Evidence Metadata](reaction-evidence.md) — the established pattern for user-supplied external corpora this guide follows.
- [Candidate Pools and Reranker Training](reranker-candidate-pools.md) — the retrosynthesis-side analog this protocol mirrors (leakage-safe splitting, conditional/end-to-end metrics, paired bootstrap).
