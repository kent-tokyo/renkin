# Phase 3A Round 2: competitive-benchmark quarantine + structural atom-map fix

A review of Round 1's real-label pipeline (PR #105, commit `58baa71`) found
two problems before Phase 3B (candidate-pool generation) should start.
Both are fixed and re-verified in this round; PR #105 stays draft.

## Blocker 1: reranker train/val would have contaminated the competitive benchmark

**The problem.** Round 1 derived `correct_precursor_sets` from the
existing 4,903-target benchmark corpus (`sample_full_sorted.jsonl`, used by
every Issue #101 Phase 1/2 gate) and intended to re-split those same 4,903
targets 70/15/15 (`train_reranker.py`'s `SHA256(target_id)` bucket) for
reranker train/val/test. That is fine as an isolated offline evaluation --
but if the trained reranker were later integrated into search and re-run
against this same benchmark to compare against AiZynthFinder (the eventual
500/4,903-target route-search comparison this whole program exists to
win), roughly 70% of the "competitive" targets would have been in the
reranker's own training set. Any resulting "beats AiZynthFinder" claim
would be invalid on inspection.

**The fix: quarantine the benchmark, source train/val from USPTO-50k's own
original train/val splits.**

```
USPTO-50k original train  ->  reranker TRAIN
USPTO-50k original val    ->  reranker VALIDATION
---------------------------------------------------  never crossed
existing 4,903 targets    ->  reranker FORMAL TEST  ->  500/4,903-target
(USPTO-50k original test)                               AiZynthFinder comparison
```

Training on USPTO-50k's original train split is *not* leakage -- it is
ordinary supervised learning, and Round 1 already confirmed
`templates_extracted_500.smi` (the rule set every candidate pool will be
generated from) is extracted **exclusively** from that same train split
(commit `b4e113a`). The reranker seeing train-split reactions adds nothing
templates didn't already have access to. What must never happen is the
reranker's train/val seeing anything whose *product* matches a quarantined
benchmark target.

### A. Competitive benchmark quarantine

`scripts/generate_benchmark_quarantine_manifest.py` freezes
`sample_full_sorted.jsonl` (4,903 targets) as a **FORMAL TEST QUARANTINE**:
every target is canonicalized (`renkin-canonicalize --clear-atom-maps`),
the resulting identity set is written to
`data/phase3a_reranker_ground_truth_audit/benchmark_quarantine_target_identities.txt`
(gitignored, regenerable), and a manifest records corpus path/SHA-256,
identities-file SHA-256, `n_targets=4903` (verified equal to the distinct
identity count -- the script hard-errors if any target fails to
canonicalize or if there are unexpected duplicates), and the source
dataset revision.

### B/C. Train/val labels with decontamination

`scripts/generate_train_val_labels.py` reads the raw USPTO-50k train
(40,008 reactions) and val (5,001 reactions) splits, canonicalizes every
product, and **drops any reaction whose product matches a quarantined
benchmark identity** -- reported, not silently absorbed:

| | raw reactions | unique products | overlapping benchmark | rows removed |
|---|---|---|---|---|
| train | 40,008 | 39,736 | 68 | 81 |
| val | 5,001 | 4,993 | 7 | 7 |

Overlap is small (~0.17%/0.14%) but the check doesn't depend on that --
every one of the 4,903 quarantined identities is checked against every
train/val product, not sampled.

**Cross-split hygiene.** USPTO-50k splits by *reaction*, not by *product*,
so the same product can legitimately have recorded reactions in both train
and val. After per-split decontamination, no `target_id` may appear in
both -- the deterministic rule (fixed here, not the only valid choice, but
recorded so a future reviewer doesn't have to reverse-engineer it): **train
wins**. Any val target_id also present in train is dropped from val
entirely, not merged. This removed 63 more val rows (62 distinct
target_ids).

| | before dedup | after benchmark + cross-split dedup | labeled groups | distinct target_ids |
|---|---|---|---|---|
| train | -- | -- | 39,927 | 39,668 |
| val | 4,994 | 4,931 | 4,931 | 4,924 |

Three hard assertions run at the end of every `generate_train_val_labels.py`
invocation (not just reported -- the script raises `AssertionError` and
refuses to write output if any fails): `train_target_ids & val_target_ids
== {}`, `train_products & benchmark_identities == {}`, `val_products &
benchmark_identities == {}`. All three passed on this run.

### D. target_id / group_id semantics for train/val (deliberately different from the formal test set)

`target_id` is a stable, **split-independent** identity derived from the
canonical product SMILES (`sha256(product)[:16]`, prefixed
`uspto50k#product:`) -- the same product always gets the same target_id
regardless of which split or how many raw reactions produced it. This is
what makes the cross-split overlap check in Section C possible at all.

`group_id` is **per raw reaction row**
(`uspto50k_{train,val}#L{line_number}`), deliberately **not** collapsed to
one group per target_id the way the formal test corpus is. Rationale: for
train/val, multiple literature routes to the same product are meant to be
separate training/validation examples (denser supervision, and it matches
how the raw dataset naturally provides one example per reaction); for the
small formal test set, collapsing multi-route targets into one group
avoids double-counting the same target in top-1/MRR aggregate coverage
metrics. Two different purposes, two different collapsing rules -- both
intentional, recorded here so they don't look like an inconsistency.

Since `target_id` doesn't encode the product SMILES, a companion lookup
(`data/reranker_targets_uspto50k_{train,val}.jsonl`, `{target_id,
canonical_smiles}`) is written alongside the labels for pool generation and
spot-checking to consume.

### E. `train_reranker.py --split-manifest`

The existing `SHA256(target_id) % 100` bucket (`split_for_target`) would
misclassify most of these train/val target_ids (their split is now
assigned by dataset origin, not by hash). `train_reranker.py` gained an
explicit override, consulted by all five of `split_for_target`'s call
sites via a module-level `_SPLIT_OVERRIDE` (empty by default -- omitting
`--split-manifest` leaves every existing caller's behavior byte-for-byte
unchanged; confirmed via the pre-existing `--self-test` and the full
`scripts/tests/` suite, both still green):

```
--split-manifest <jsonl>   # {"target_id": ..., "split": "train"|"val"|"test"}
```

`load_split_manifest` hard-validates: no duplicate/conflicting assignment
per target_id, no target_id unknown to the current `--groups` file, no
target_id from `--groups` missing an assignment (exact set equality, not
subset -- a manifest built against a different run must fail loudly, not
silently mix hash-bucket and manifest splits). 9 new tests in
`scripts/tests/test_reranker_labels.py::SplitManifestTests` cover the
override mechanism and all four validation failure modes; full suite is
276/276 (up from 267), `--self-test` still passes.

`generate_train_val_labels.py` emits the corresponding manifest
(`data/reranker_split_manifest.jsonl`, 49,495 rows: 39,668 train + 4,924
val + 4,903 test, zero conflicting duplicates).

## Blocker 2: the atom-map strip was a text-level regex, not a structural operation

**The problem.** `generate_real_labels.py` stripped atom maps via
`re.compile(r":\d+").sub("", smiles)`. `:` is also SMILES bond syntax
(explicit aromatic/double bonds), so this can delete a ring-closure digit
that happens to follow an explicit bond symbol instead of an atom map.
Concrete, verified example: mapped benzene with every ring bond written
explicitly, `[cH:1]:1:c:c:c:c:c:1`, regex-stripped to `[cH]:c:c:c:c:c` --
canonicalizes to `ccccc[cH]`, an **open chain**, not benzene
(`c1ccccc1`). "5,007 raw records parsed without error" (Round 1's own
check) never proved the *structure* was preserved, only that the corrupted
string still happened to be syntactically valid SMILES.

**The fix.** `chem_env::clear_atom_maps` (`src/chem_env.rs`) operates on
the parsed molecule graph: rebuilds via `MoleculeBuilder`, atom-by-atom,
clearing only `Atom.atom_map` (a real `Option<u16>` field on chematic's
`Atom`, confirmed structurally typed, not string-encoded) while every
other property -- element, charge, isotope, aromaticity, chirality,
hydrogen count, wildcard, stereo groups, stereo neighbor order, bond
directions -- is copied verbatim (atoms/bonds re-added in identical order,
so index-keyed side tables stay valid; see the function's doc comment for
why that's the correctness argument, not an assumption). Exposed via
`renkin-canonicalize --clear-atom-maps`. 10 new Rust tests
(`chem_env::clear_atom_maps_tests`) cover: normal + multi-digit atom maps,
isotope, formal charge, tetrahedral stereo (including that `@`/`@@` still
differ after clearing), aromatic atoms, disconnected fragments,
already-unmapped input (no-op), heavy-atom/bond-count preservation, and
the exact regex-corruption fixture above (pinned so it can't silently stop
demonstrating the failure mode). `generate_real_labels.py`'s regex is
deleted; both label scripts now call `renkin-canonicalize --clear-atom-maps`
exclusively via a shared `scripts/reranker_label_common.py` helper (kept in
one place specifically so the two pipelines can't drift onto different
atom-map handling again).

**Regenerated the formal test labels and diffed against the pre-fix
file**: 4,903/4,903 rows identical, 0 changed. Per instruction, this is
recorded honestly as "no observed difference on this specific corpus," not
generalized to "the old regex was safe" -- the raw USPTO-50k SMILES simply
never happened to combine an explicit `:` bond with a ring-closure digit
in this particular 4,903-target sample. The fixture above proves the old
code *would* have silently corrupted a molecule that did.

## Spot-check (Section I, not a formal accuracy claim)

`examples/label_spotcheck.rs`, generalized to accept `--labels`/`--targets`,
run against all three corpora with the real `propose_one_step(Exhaustive)`
against `templates_extracted_500.smi`:

| corpus | sample | ground-truth reachable | zero-candidate |
|---|---|---|---|
| formal test | 10 | 8/10 | 0/10 |
| train | 30 | 18/30 (60%) | 0/30 |
| val | 30 | 21/30 (70%) | 0/30 |

Non-degenerate across all three -- not a coverage estimate (small,
non-random samples), just confirmation the pipeline works end-to-end on
real train/val data too, not only the already-validated test corpus.

## Determinism

Every generator script (`generate_real_labels.py`,
`generate_benchmark_quarantine_manifest.py`, `generate_train_val_labels.py`)
re-run against the same inputs a second time and diffed byte-for-byte
against its first output: all six output files (`reranker_labels_uspto50k_
{test,train,val}.jsonl`, `reranker_split_manifest.jsonl`,
`reranker_targets_uspto50k_{train,val}.jsonl`) identical.

## Section J go/no-go: all items GO

- benchmark 4,903 fully quarantined -- yes, manifest + identity digest recorded.
- train/val target_id overlap -- 0 (asserted).
- benchmark vs train overlap -- 0 (asserted, post-decontamination).
- benchmark vs val overlap -- 0 (asserted, post-decontamination).
- structural atom-map removal -- done, 10 Rust regression tests.
- labels deterministic -- confirmed, byte-identical repeat run.
- manifest/provenance fixed -- quarantine manifest + train/val summary JSON, both with SHA-256s of every input/output.
- spot checks non-degenerate -- 8/10, 18/30, 21/30 reachable; 0 zero-candidate targets across all three corpora.

Phase 3B (100/500/full candidate-pool generation) is next, not started
this round. PR #104 (open-state dominance) untouched, still draft. No
adaptive beam, no timeout/beam-width change, no runtime reranker
integration, no 4,903-target route-search run, no posting to Issue #101,
no tag/release.
