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
| `accepted_products` | array of arrays of SMILES strings | yes, non-empty, every inner array non-empty | One or more accepted correct product multisets for this reaction. More than one entry means more than one reported/acceptable outcome is correct (e.g. competing literature reports) — a candidate matching *any* entry counts as correct. Two entries that canonicalize to the identical multiset are deduped (outer list only — multiplicity *within* one multiset is preserved), so a repeated entry never inflates `ndcg_at_10`'s ideal DCG or `num_products`'s corpus-level source. |
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
- `conflicting_reaction_ids` — the corpus's own `reaction_id` was reused for
  a row that canonicalizes to a genuinely different reaction (different
  reactants and/or accepted products). `reaction_id` is documented above as
  never used for identity/splitting precisely because corpora can't be
  trusted to keep it unique; the later row becomes an `input_invalid` output
  row rather than silently accepted under a non-unique key.
- `conflicting_group_keys` — see "Reaction identity" next: this rejects the
  whole reaction, not just one row.

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
  across train/val/test. If two duplicate rows for the SAME reaction
  identity supply two DIFFERENT non-empty explicit `group_key` values,
  neither can be trusted over the other — the whole reaction is rejected
  (`conflicting_group_keys`, `InvalidReactionAttempt` reason
  `"conflicting_group_key"`) rather than silently keeping whichever value
  was seen first.

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
3. **`train-extracted`** — recognized by the frozen protocol, but **rejected
   as a hard error in this PR**. Loading it exactly like `file` and merely
   stamping a different provenance label would be a claim, not a verified
   guarantee: this harness has no way to check that the given `--templates`
   file was actually extracted from the train split only. Use
   `--template-source file` if you accept responsibility for that split
   boundary yourself. A future version will require `--template-manifest
   <path>` attesting `{templates_sha256, source_corpus_sha256,
   split_protocol_version, included_split: "train"}`, hard-validated before
   the file loads — until then, `train-extracted` stays blocked rather than
   silently trusting an unverifiable label.
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
including `--template-source`/`--templates`/`--strict`.

`--strict` turns every counted-and-continued data-quality issue (malformed
corpus JSON, wrong schema version, an unparseable SMILES, empty reactants/
accepted products, a conflicting `group_key`/`reaction_id`, a per-row
prediction failure, `proposal_status: capped_unknown`, or incomplete
reproducibility provenance) into a whole-run hard error instead — for a
formal/reportable benchmark run where any of those would silently make the
numbers meaningless. It never fails on a legitimate proposal/ranking/stereo
miss or a genuinely empty candidate pool — those are real outcomes. Off by
default: without it, every one of these issues still lands in
`corpus_stats`/`corpus_warnings`/`diagnostics`/per-row fields, nothing is
ever silently dropped either way.

### Row schema (one row per reaction, `FORWARD_BENCH_REPORT_SCHEMA_VERSION = 2`)

| Field | Meaning |
|---|---|
| `reaction_id`, `source_line`, `split` | Identity and leakage-safe split. |
| `leakage_group_id` | The group key `split` was actually derived from (explicit corpus `group_key`, or the deterministic reactant-hash fallback) — `null` only for `input_invalid` rows, where reactants never canonicalized and no group key could ever be computed (a `prediction_error` row still has one: its reactants canonicalized fine, prediction itself just failed). Carried onto every row so the leakage-safety guarantee is auditable from `rows.jsonl` alone. |
| `reaction_class` | Pass-through from the corpus, or `null`. |
| `reactants_canonical`, `accepted_products_canonical` | Canonicalized inputs, exactly as matched against. |
| `num_reactants` | `reactants_canonical.len()`. |
| `accepted_product_count_min`, `accepted_product_count_max`, `accepted_product_count_mixed` | Min/max accepted-product count across every entry in `accepted_products_canonical`, and whether they differ — replaces a single `num_products` field, which could only describe the first accepted outcome's arity and silently misrepresented a reaction with several accepted outcomes of different arity. Independent of the outer list's order or its C4 dedup. |
| `has_stereochemistry` | Auto-detected from `@`/`/`/`\` in any canonical reactant/accepted-product SMILES — never trusted from corpus metadata, since it is fully derivable. |
| `candidate_count`, `raw_outcomes` | Final merged-candidate count, and total `run_reactants` outcomes attempted before validity/no-op filtering or merging (the denominator `invalid_product_rate`/`no_op_rate` are computed against). |
| `correct_candidate_present`, `best_correct_rank` | Stereochemistry-aware presence/best rank (0-based). |
| `correct_ranks_top10` | Every rank (0-based, `< 10`) whose candidate matches ANY `accepted_products_canonical` entry, not just the best one — feeds `ndcg_at_10`'s multi-positive credit assignment. `best_correct_rank`/`top1_hit`/`top5_hit`/`top10_hit`/mean/median rank are unaffected and still derived from the single best rank alone. |
| `best_correct_rank_stereo_ignored` | Best rank under the loosened comparison, present only when the outcome below is `hit` — always `<= best_correct_rank` when both are present. |
| `top1_hit`, `top5_hit`, `top10_hit` | `best_correct_rank < 1/5/10`. |
| `stereochemistry_aware_hit` | Identical to `top10_hit`, reported under its own name so it sits directly next to `stereochemistry_ignored_outcome` for a same-row before/after read. |
| `stereochemistry_ignored_outcome` | One of `hit`, `no_hit`, or `unsupported` (a tri-state, not a plain bool, so an uncomputable comparison is never conflated with a clean miss — see "Stereochemistry comparison" above). **`hit` while `stereochemistry_aware_hit` is `false` is the diagnostic signal for "constitution right, stereochemistry wrong."** |
| `invalid_candidate_count`, `no_op_candidate_count` | From the underlying `ForwardStats` (`invalid_outcomes_rejected`/`no_op_outcomes_rejected`). |
| `application_warning_count`, `application_error_count`, `templates_attempted`, `templates_matched`, `graph_rules_skipped`, `rules_loaded` | From the underlying prediction report (`ForwardStats`) — `graph_rules_skipped` (empty-`smirks` rules, never counted as a parse failure) and `templates_matched` (a subset of `templates_attempted` that had at least one slot match) are per-row, not just summed in the report header. |
| `elapsed_ms` | Wall-clock time for this reaction's one `predict_products_detailed` call. **Not deterministic across runs** — see below. |
| `failure_reason` | One of `hit_top1`, `hit_top5`, `hit_top10`, `hit_beyond_10`, `correct_absent_empty_pool`, `correct_absent_nonempty_pool`, `input_invalid`, `prediction_error`. Deliberately coarse (Phase 1 legacy shape) and now *derived* from the four orthogonal fields below (`derive_failure_reason`), so the two representations can never disagree — see `failure_reason_is_uniquely_derivable_from_orthogonal_statuses`. Classifying *why* a template failed to apply (missing forward SMIRKS, atom-mapping mismatch, stereochemistry mismatch, …) is still Phase 2 territory, not this harness. |
| `input_status` | `valid` or `invalid` — whether this row's input canonicalized at all. `invalid` iff `failure_reason == input_invalid`. |
| `proposal_status` | `covered` (a correct candidate is present in the pool), `missed_empty_pool`, `missed_nonempty_pool`, `capped_unknown` (the pool was truncated before absence could be confirmed — wired for correctness but unreachable with this harness's own `max_results: usize::MAX` predict config), `error` (prediction itself failed), or `not_attempted` (input invalid). Orthogonal to `ranking_status`: every `covered` row has a concrete `ranking_status`, every other row has `not_applicable`. |
| `ranking_status` | `top1`/`top5`/`top10`/`beyond10` when `proposal_status == covered`, else `not_applicable`. |
| `stereo_status` | `exact_hit`, `stereo_only_hit` ("constitution right, stereochemistry wrong"), `no_hit`, `unsupported`, or `not_applicable` (prediction never attempted). Mirrors `stereochemistry_aware_hit`/`stereochemistry_ignored_outcome` as an orthogonal-status view; `unsupported` is never silently collapsed to `no_hit`. |
| `provenance` | Full `RunProvenance` (see "Reproducibility provenance" below) — duplicated onto every row (not just the report header) so a single row, taken out of context, still fully describes what produced it. |

A constructor-level check (`check_status_consistency`) rejects any row whose
`input_status`/`proposal_status`/`ranking_status`/`stereo_status`/
`best_correct_rank` contradict each other — every row this harness emits
passes it; the four status fields are trustworthy on their own without
cross-referencing `failure_reason`.

### Aggregate metrics

Computed once overall, once per split (`train`/`val`/`test`), and once per
breakdown bucket (`reaction_class`, `num_reactants`,
`accepted_product_arity`, `stereochemistry_presence`, `template_source`,
`candidate_pool_size`, `failure_reason`, `input_status`, `proposal_status`,
`ranking_status`, `stereo_status`):

`accepted_product_arity` buckets are `"1"`/`"2"`/`"3+"` when every accepted
outcome for a reaction has the same product count, `"mixed:{min}-{max}"`
(each end bucketed the same way, e.g. `"mixed:1-2"`, `"mixed:1-3+"`) when
they don't, and `"<missing>"` for a row with no accepted-products info at
all (an `input_invalid` row) — replaces the old `num_products` breakdown,
which could only bucket by the first accepted outcome's arity.

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

  `ndcg_at_10` is multi-positive, binary-relevance NDCG@10: every candidate
  (up to rank 9) matching ANY entry in `accepted_products_canonical`
  counts as relevant, not just the single best-ranked one. Ideal DCG is
  computed against `min(10, accepted_products_canonical.len())` — the true
  number of distinct accepted outcomes for that row (after outer-list
  dedup) — rather than always assuming exactly one. A ranking that
  surfaces two of three accepted outcomes in the top 10 scores higher than
  one that surfaces only one, and lower than one that finds all three.
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

### Reproducibility provenance

Every report's `provenance` block (also duplicated onto every row) carries,
beyond `renkin_forward_version`/`template_source`/`rules_file_sha256`:

| Field | Meaning |
|---|---|
| `rules_content_sha256` | SHA-256 over the sorted `(template_id, smirks)` pairs of the rule set actually loaded — populated for every `template_source`, including `embedded` (`rules_file_sha256` is `null` there, since there's no file to hash). Two runs with a different rule *file* but identical rule *content* hash identically here. |
| `split_protocol_version` | Versions the split algorithm itself (bucket cutoffs, hash scheme) — separate from `FORWARD_BENCH_REPORT_SCHEMA_VERSION`, so a future change to how train/val/test buckets are assigned can be detected independently of a schema change. |
| `binary_sha256` | SHA-256 of the currently-running `renkin-forward` executable's own bytes, best-effort (`null` if `std::env::current_exe()` or the read fails). **Deliberately excluded from `reproducibility_sha256`** — Rust builds are not bit-reproducible even from identical source, so including it would make every independently-built binary report a spurious mismatch. |
| `cargo_lock_sha256` | SHA-256 of the workspace `Cargo.lock`, embedded at compile time (`include_str!`) rather than read at runtime — an installed or copied binary has no reliable relative path back to the source tree. |
| `config_sha256` | SHA-256 over the deterministic run configuration (`template_source`, `split_protocol_version`, `train_max_bucket`, `val_max_bucket`). Deliberately excludes `corpus_path` (a filesystem path, not corpus content — `corpus_sha256` already captures content identity). |
| `reproducibility_sha256` | SHA-256 over every row's serialized fields, in `source_line` order, with `elapsed_ms` stripped from each row first — scoped to `rows` only, not the full report, since `overall`/`by_split`/`breakdowns` are pure deterministic functions of `rows` with no hidden per-run state. Two independent runs over the same corpus/rules/`template_source` must produce an identical value; see `tests/bench.rs::benchmark_is_deterministic_modulo_timing_fields`. |
| `reproducibility_excludes` | Published list (`["elapsed_ms", "latency_ms", "binary_sha256", "corpus_path"]`) documenting the full "reproducible modulo what" contract. Only `elapsed_ms` is actually stripped by `reproducibility_sha256` itself; the other three never appear inside a row in the first place — this field exists so the contract doesn't have to be inferred from reading the hash function's source. |

### Diagnostic counts

The top-level report's `diagnostics` block gives report-wide totals without
requiring a reader to re-sum `rows.jsonl` themselves. Every field is
transcribed from an already-computed source (never independently
re-derived), named in its own doc comment:

| Field | Source |
|---|---|
| `warning_counts_by_code` | `corpus_warnings[].code`, counted. |
| `template_application_errors`, `graph_rules_skipped`, `templates_attempted`, `templates_matched` | Row-wise sum of the identically-named `BenchRow` field. |
| `invalid_outcomes_rejected`, `no_op_outcomes_rejected` | Transcribed from `overall.invalid_outcomes_rejected`/`overall.no_op_outcomes_rejected` (the same values `invalid_product_rate`/`no_op_rate`'s numerators already use). |
| `raw_outcomes` | Transcribed from `overall.n_raw_outcomes`. |
| `template_parse_rejections_by_reason` | Always an empty map in this harness: `load_rules_for_source` rejects the whole rule set on any single rule's parse failure (a hard error before any row is computed), so a completed run has zero *partial* template-parse rejections by construction. A non-empty map would require `ForwardStats`/rule loading to grow a genuine per-rule rejection-reason breakdown first. |

### `--strict`

Off by default. When passed, every one of the following becomes a
whole-run hard error instead of a counted-and-continued data-quality issue:
malformed corpus JSON, wrong schema version, an unparseable reactant/
product SMILES, empty reactants/accepted products, a conflicting explicit
`group_key` or reused `reaction_id`, a per-row prediction failure
(`proposal_status: error`), `proposal_status: capped_unknown`, or a missing
`binary_sha256` (incomplete reproducibility provenance). Template load/
manifest/provenance failure (e.g. `train-extracted`, an unreadable
`--templates` file) is already an unconditional hard error regardless of
`--strict`.

`--strict` deliberately does **not** fail on a legitimate proposal miss,
ranking miss, stereo mismatch, or a genuinely empty candidate pool — those
are real benchmark outcomes, not data-quality problems. Non-strict mode
never silently drops anything either: every one of the conditions above
still lands in `corpus_stats`/`corpus_warnings`/`diagnostics`/per-row
fields regardless of the flag.

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
