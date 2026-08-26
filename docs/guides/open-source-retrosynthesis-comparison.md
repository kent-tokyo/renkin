# Open-source retrosynthesis planner comparison (Issue #66)

[Issue #66](https://github.com/kent-tokyo/renkin/issues/66) asks how RENKIN
compares to existing retrosynthesis tools — specifically ASKCOS,
AiZynthFinder, and commercial platforms — and what value RENKIN adds. This
page documents a **reproducible, fair-condition empirical comparison**
between RENKIN and open-source planners, built to answer the parts of that
question an automated harness can actually answer, and to be explicit about
the parts it cannot.

## Scope: commercial platforms are excluded

**Commercial retrosynthesis platforms (SciFinder, Reaxys, and similar) are
out of scope for this empirical benchmark.** They differ from RENKIN and
the open-source planners compared here in access conditions, licensing, and
reproducibility — there is no way to give an outside reader the means to
independently re-run or verify a number computed against a platform they
cannot themselves access under the same terms. This comparison is limited
to planners whose code, and at minimum some usable model/data
configuration, are publicly available. See
[`askcos-feasibility-issue-66.md`](../comparison/askcos-feasibility-issue-66.md)
for why even an open-source-adjacent project (ASKCOS) doesn't fully clear
that bar this round.

**No superiority claim is made or licensed by this document or its
artifacts.** Every metric here is either a raw completion/latency/memory
measurement or a coarse, disclosed-limitation post-hoc structural check —
never a chemistry-correctness or route-quality judgment. See
"Semantic firewall" below.

## Why the existing comparison table isn't a matched comparison

[`docs/comparison/open-source-retrosynthesis-tools.md`](../comparison/open-source-retrosynthesis-tools.md)
already carries an explicit warning that its planner-comparison table is
**not a matched-condition comparison**: building-block counts, template
counts, and evaluation setups differ significantly across the systems it
lists, and the cited AiZynthFinder/Retro\*/ASKCOS numbers come from their
own original publications, not a common run. That table is **not rewritten
by this work** — replacing it is deferred to a future PR, after a formal
500- or 4,903-target measurement using the infrastructure documented here.
This page's own 100- and 500-target results (below) are a separate, new,
narrower measurement: same targets, same hardware, same timeout, run by this
project, for RENKIN and AiZynthFinder only.

## Three measurement arms

| Arm | RENKIN stock | RENKIN templates | AiZynthFinder stock | AiZynthFinder policy | Framing |
|---|---|---|---|---|---|
| **A — native** | `data/building_blocks.smi` (402 unique) | `data/templates_extracted_500.smi` (500 templates) | official public ZINC stock (MIT, via Figshare) | official public USPTO ONNX model (CC BY 4.0, via Zenodo) | "what a user gets from each project's own recommended public configuration" — a comparison of full public distributions, **not** an engine-only comparison. AiZynthFinder's result on this sample was highly sensitive to the configured stock (see Arm B). |
| **B — shared-stock** | the shared 393-compound stock (`data/comparison/shared_stock/shared_stock.smi`) | **unchanged** | the SAME shared 393-compound stock, written directly as an HDF5 InChIKey table (`scripts/compare_shared_stock.py`) — a guaranteed zero-diff identity, not a `smiles2stock` conversion (see "Provenance") | **unchanged** (official public model) | The shared-stock arm does **not** isolate search-engine quality: policy calibration, search budgets, template/model sources, and internal stock-matching semantics (e.g. RENKIN's VF2 fallback — see `data/comparison/results_100/per_target_audit.md`) remain different between the two tools. Its primary metric is the common **`route_to_shared_stock` rate** (post-hoc, independently re-verified for both tools identically) — tool-native `route_found` is reported alongside as a secondary/informational field only, and a tool-native "solved" route that fails the independent check never counts toward the primary numerator. |
| **C — RENKIN vendored-500** | 402 compounds | 500 templates | — | — | RENKIN is measured under the reproducible vendored-500-template configuration on current `master`, explicitly **not** the historical 5,000-template "corrected baseline" (986/756/43 out of 4,907, frozen to commit `e20dc8c`, renkin 0.15.5, chematic 0.4.25) — the two are different corpora/configurations and must never be compared as if refreshing the same number. |

Arm C exists because RENKIN's own `CHANGELOG.md` states the post-`e20dc8c`
4,907-target remeasurement "remains not run" against the historical
5,000-template configuration — the historical numbers are over 100 commits
and a chematic major-version bump old, and were produced against a larger,
differently-sourced template set. This 100-target round is a fresh,
independent RENKIN measurement under today's vendored-500-template
configuration; it is not a "refresh" of the historical number and the two
must be read as separate measurements, not compared directly.

A full "engine-only" comparison (same stock **and** same templates/policy)
is not attempted: AiZynthFinder's policy is a trained neural model, not a
swappable SMIRKS list, so matching it to RENKIN's rule set would require
retraining or template-injection — out of scope here, and not something a
template-count-matching exercise could achieve honestly anyway.

## Deterministic target sampling

Population: `data/uspto50k_test.smi`, the USPTO-50k test split (4,907
candidate lines; header cites the `bisectgroup/USPTO_50K` Hugging Face
mirror; a documented gap — see "Known gaps" below — is that the header's
"5007 reactions" claim doesn't match the file's 4,907 data rows, and no
in-repo record traces the exact HF revision/date this file was derived
from).

Algorithm (implemented in `scripts/compare_sampling.py`, frozen output
checked into `data/comparison/`):

1. Canonicalize every candidate line's SMILES via **RDKit** (not chematic —
   see "Canonicalizer choice" below).
2. Group by canonical SMILES; where a canonical form has more than one raw
   line, keep the lowest-numbered line and record the rest as a duplicate
   (4 duplicate groups found; 0 unparseable lines).
3. Compute `SHA-256("renkin-issue66-sample-v1|" + canonical_smiles)` for
   every surviving unique target.
4. Sort ascending by that hash (tie-break: ascending canonical SMILES
   string).
5. The first 100 rows are the feasibility sample; the first 500 are a
   future validation sample; all 4,903 are the formal set. **All three are
   prefixes of the one sorted list computed in step 4** — never three
   independently-derived samples — so `sample_100 == sample_500[:100] ==
   sample_full[:100]` holds by construction, not by coincidence.

Frozen artifacts: `data/comparison/sample_manifest.json` (accounting: raw
line counts, duplicate groups, unparseable count, hash function, tie-break
rule, corpus/list SHA-256 sums) and `data/comparison/sample_full_sorted.jsonl`
(the ordered list itself — `sample_rank`, `target_id`, `canonical_smiles`,
`source_line_number`, `sample_key` per row).

**This round only runs the 100-target feasibility sample.** The 500- and
4,903-target rounds are explicitly not started — see "500/full run gate"
below.

## Canonicalizer choice

Every SMILES in this comparison — sample selection, route normalization,
stock-leaf matching — is canonicalized with **RDKit**, not chematic's
`canonical_smiles`. This is a deliberate choice, not an oversight: RENKIN's
own `src/validation/forward.rs` documents that chematic's canonicalizer is a
stable fixed point but **not** a true graph invariant — the same molecule,
written two different ways, can canonicalize to two different strings that
never converge, even under repeated re-canonicalization. Since RENKIN's own
SMILES emission shares that canonicalizer's notation lineage, using it as
the sole leaf-matching primitive would make RENKIN's own building blocks
match more often than AiZynthFinder's chemically-identical-but-differently-written
leaves — a metric biased toward RENKIN by construction. RDKit is a neutral
third party here: it's what AiZynthFinder itself emits natively, it's
already an installed dependency in this project's `scripts/` tooling, and
its canonical SMILES is a much closer approximation to a true invariant.

This does not make canonical-SMILES matching flawless — see "What this
validation does not claim" below for the specific ceilings (tautomers,
differing stereochemistry conventions) that remain regardless of
canonicalizer.

## Route selection

Both adapters report only the **rank-1 route** (the tool's own top-ranked
proposal) per target for every common metric. Using "best of N routes
passes" for any headline rate would favor whichever tool happens to return
more candidate routes per target — the same class of unfairness the
canonicalizer choice above avoids. The one exception is
`duplicate_route_within_target`, a target-level informational check that
looks across *all* returned routes for a target, precisely because it's
checking for redundancy across the full set, not picking a winner.

## Hardware and run conditions

| Field | Value |
|---|---|
| Host OS/arch | macOS, Apple Silicon (arm64) |
| Host RAM / CPUs | 16 GB / 10 cores |
| Docker Desktop VM allocation | **~7.65 GiB / 10 vCPU** — materially less RAM than the host; every container `--memory` cap in this comparison is sized against the VM's ceiling, not the host's |
| RENKIN execution | **native** (not containerized) — see "Why RENKIN is not containerized" |
| AiZynthFinder execution | `linux/arm64` Docker container (no amd64 emulation) — see `docker/aizynthfinder.Dockerfile` |
| Container resource cap | `--cpus 8 --memory 6g --memory-swap 6g` (headroom left for the Docker VM's own daemon processes) |
| Concurrency | strictly sequential — one target in flight at a time, one tool's full sample run to completion before the other starts |
| Per-target external timeout | wrapper-enforced wall-clock deadline, authoritative regardless of what either tool's own budget parameter is set to |
| Network policy | setup phase (image build, `pip install`, public model/stock download) uses the network; the AiZynthFinder measurement container runs with `--network none` |

### Why RENKIN is not containerized

An earlier design considered containerizing RENKIN too, to remove the
native-vs-container asymmetry from cross-tool latency/memory comparisons.
That asymmetry is real, but the fix isn't worth its cost here: this
comparison's own latency methodology (see "Latency comparison firewall"
below) already excludes any "N× faster" headline claim computed from
raw wall-clock time, precisely because RENKIN's search budget is
combinatorial (depth × beam) while AiZynthFinder's is temporal (`time_limit`)
— the two aren't measuring the same kind of stopping condition regardless
of where either process runs. Paying for a second Dockerfile and a
cgroup-vs-`getrusage` reconciliation would buy comparability for a claim
this project has already committed not to make. **The honest, disclosed
limitation instead**: `total_elapsed_ms` and `peak_rss_bytes` are not
directly comparable between RENKIN (native, `/usr/bin/time -l`,
`rss_measurement_method="usr_bin_time_v"`, an exact per-process high-water
mark) and AiZynthFinder (containerized, `docker stats` polling,
`rss_measurement_method="docker_stats_sampled"`, a coarser ~300ms-interval
sample that can miss short spikes and includes container-level overhead).
Every row records which method produced its numbers so this is never
silently conflated.

## Provenance

Recorded per tool, per `configuration_id` (see `PlannerComparisonRow` schema
below): tool name/version, source (git commit for RENKIN; container image
digest for AiZynthFinder), install method, dependency lock (`Cargo.lock`
hash / `pip freeze` lock), model/template/stock identity and SHA-256 (both
raw file hash and, separately, the harness's own canonicalized
`stock_set_sha256`, since a tool's own loader and this harness's RDKit-based
loader do not have to agree on exact counts), configuration, command line,
license identifiers (code/model/stock recorded **separately**, since
AiZynthFinder's code, model, and stock can carry different licenses),
download source and timestamp.

AiZynthFinder's public data licenses (verified directly, not assumed):
**ZINC stock file: MIT** (Figshare article 12334577); **USPTO ONNX models:
CC BY 4.0** (Zenodo record 7797465). Both are unambiguous open licenses —
this was the one open question flagged during feasibility research, and it
resolved cleanly in favor of using AiZynthFinder's default public
configuration for Arm A.

**Shared-stock construction (`scripts/compare_shared_stock.py`), a guaranteed
zero-diff identity, not a lossy conversion:** AiZynthFinder's default HDF5
stock format keys molecules by **InChIKey**, not SMILES (its
`InMemoryInchiKeyQuery` loader reads a single `inchi_key` column via
`pandas.read_hdf(path, "table")` — confirmed by reading the installed
package's source directly, and by inspecting a converted file). An earlier
version of this arm converted `data/building_blocks.smi` through
`smiles2stock`'s own SMILES-reading pipeline, which silently dropped
directional (E/Z) bond stereo for at least one compound (fumaric acid),
leaving a residual, unexplainable round-trip mismatch
(`roundtrip_identity_confirmed=false`) that was never an acceptable basis
for a "shared stock" claim. This arm now **bypasses `smiles2stock`
entirely**: every line of `data/building_blocks.smi` is parsed directly with
RDKit (`Chem.MolFromSmiles`) inside the AiZynthFinder container, its InChIKey
computed via `Chem.MolToInchiKey` — the *exact same call* AiZynthFinder's own
`Molecule.inchi_key` property makes on its search candidates at runtime
(confirmed by reading `aizynthfinder.chem.mol` source inside the container)
— and the resulting `{inchi_key}` table is written directly to HDF5, with
no separate "conversion" step left to disagree with AiZynthFinder's own
runtime lookup. **Policy** (recorded in
`data/comparison/shared_stock/shared_stock_manifest.json`): shared-stock
identity is RDKit's `MolToInchiKey` of the parsed source SMILES, stereo/
isotope/charge exactly as present in the source line — no stripping, no
"modulo X" exception. Fumaric acid's stereo-bearing InChIKey
(`...{}-OWOJBTEDSA-N` rather than the flat key `smiles2stock` produced) is
now correctly preserved, by construction rather than by exception. Before
building the full stock, this bypass approach was validated on a toy
4-compound hand-built HDF5 (aniline, acetic anhydride, acetic acid, acetyl
chloride) against the real container and the real `aizynthcli`: it
correctly loaded the hand-built file (`Compounds in stock: 4`) and solved
acetanilide via exactly the expected two-precursor route, both leaves
correctly flagged `in_stock=true`.

Of the 449 non-comment source lines, 9 remain excluded because RDKit itself
cannot parse them (3 unambiguous SMILES syntax errors in the checked-in
file, 6 aromaticity/kekulization-ambiguous heterocycles RENKIN's own parser
accepts — see "Known gaps"; this is a file-content defect, not a chemistry
limitation the shared set legitimately can't represent), and 47 further
lines collapse into an already-seen InChIKey (duplicate compounds under
different notations) — leaving **393 unique compounds**, written to both
`data/comparison/shared_stock/shared_stock.smi` (fed to RENKIN as
`--building-blocks`) and `data/comparison/shared_stock/shared_stock.hdf5`
(fed to AiZynthFinder). A read-back check (write the HDF5, then read it back
inside the same container) confirms zero missing/extra keys —
`roundtrip_identity_confirmed=true`, verifying HDF5 serialization fidelity
only, since there is no separate conversion step left to verify.

## Common schema: `PlannerComparisonRow` v1

One JSONL row per (target, tool, comparison_mode). Full field-by-field
semantics live in `scripts/compare_schema.py`'s module docstring and
dataclass; highlights:

- `tool` is a **closed enum**: `"renkin"` or `"aizynthfinder"` only. No
  commercial platform name can be constructed here — enforced by three
  tests (`scripts/tests/test_compare_schema.py`): exact set equality (not
  merely "these two are present"), deserialization rejection of any other
  name, and a source-grep deny-list scan of the schema-defining files.
- Tool-native fields (`route_found`, `tool_reported_route_count`) and
  harness-computed post-hoc fields (`route_tree_parseable`,
  `all_leaves_in_configured_stock`, `target_element_accounting_status`) are
  **always kept separate** — see "Semantic firewall".
- Everything tool-specific (RENKIN's `atom_balanced`/`nodes_expanded`/etc.,
  AiZynthFinder's `iterations`/`time_limit_s`/etc.) lives in a `tool_specific`
  object, namespaced under the tool's own key, and is never promoted into a
  common field.
- `raw_output_sha256` and `normalized_route_sha256` are recorded per row.
  The normalized hash is computed over a **tool-agnostic route DAG**
  (`scripts/compare_route_graph.py`) that both RENKIN's native route JSON
  and AiZynthFinder's `trees` output normalize into — the same proposed
  disconnection hashes identically regardless of which tool produced it
  (verified directly: `test_same_disconnection_hashes_identically_across_tools`).

## Common post-hoc validation

Applied identically to both tools' normalized routes (`scripts/compare_validation.py`):

- **`route_tree_parseable`** — structural well-formedness (single root,
  root SMILES matches the requested target, no cycles, every SMILES parses,
  every non-leaf has children, every leaf's stock status is unambiguous).
- **`reaction_steps_parseable`** — every step's reactant/product SMILES
  independently parseable, no residual self-loops.
- **`all_leaves_in_configured_stock`** — exact canonical-SMILES match
  against the stock **actually configured for that row's comparison_mode**
  (native or shared_stock) — never assumed to be RENKIN's 402 by default.
  A leaf the tool itself flags as unresolved (`is_stock_leaf=false`) is
  recorded separately from a leaf the tool *claims* is in stock but the
  harness's independent lookup misses (`leaf_claimed_stock_not_matched`) —
  conflating those two would hide a real adapter/tool discrepancy inside an
  honest "incomplete route" case.
- **`target_element_accounting_status`** — a **directional, per-element**
  heavy-atom check, NOT exact mass conservation: for every step, the
  target's count of each element must not exceed the sum over all
  precursors (precursors may legitimately carry *more* atoms — the excess is
  an untracked forward-reaction byproduct, like water in an esterification).
  Status is one of `accounted` / `unaccounted_target_element` /
  `not_evaluable`. This is stricter than RENKIN's own internal
  molecular-weight-based check: e.g. "chlorobenzene from bromobenzene" would
  *pass* RENKIN's own MW inequality (bromobenzene is heavier) but correctly
  reports `unaccounted_target_element` here (no precursor accounts for the
  product's chlorine). This is by design, and means RENKIN's own routes can
  legitimately score worse on this common check than on RENKIN's own
  internal diagnostic — the two are different checks with different
  tolerances and must never be shown as if they were the same number.

- **`validator_confirmed_route_found`** / **`not_evaluable`** — a stricter,
  validator-gated companion to tool-native `route_found`, computed
  identically for both tools from the checks above: `True` only when
  `route_found` is `True` **and** `route_tree_parseable`,
  `reaction_steps_parseable`, and `target_element_accounting_status ==
  "accounted"` all hold; `False` when `route_found` is `True` but any of
  those found a concrete defect; `null` (and unset) whenever `route_found`
  is not `True` (nothing to validate). `not_evaluable` is `True` in the one
  case that's ambiguous rather than confirmed-bad: `route_found` is `True`
  but `target_element_accounting_status == "not_evaluable"`. Solve rate
  (`route_found_rate`) and route quality
  (`validator_confirmed_route_found_rate`) are reported as two separate
  aggregate rates, both with `all_sampled` as their denominator so they're
  directly comparable to each other — never conflated into one number.

### What this validation does not claim

> Target-element-accounted (`target_element_accounting_status=accounted`)
> means the route's heavy-atom bookkeeping is internally consistent under a
> simple, one-directional per-element inequality check — it is NOT exact
> mass conservation, is not validated against real reaction feasibility,
> mechanism, or literature precedent, and must never be read as
> "chemically correct" or "chemically valid". All-leaves-in-configured-stock
> is an exact canonical-SMILES string match against the stock actually
> configured for that run; it does not account for tautomers, and does not
> account for the two tools' potentially differing stereochemistry
> conventions — a leaf that is the same molecule as a stock entry in every
> chemically meaningful sense can still be reported as missing if its
> SMILES notation diverges in either of those ways. No route in this
> benchmark has been reviewed by a human chemist, and no accuracy or
> correctness claim about any individual route, or about either tool's
> routes in aggregate, is licensed by this harness alone. Tool-native
> "solved" and this harness's post-hoc "accepted" are reported as separate
> metrics and must never be merged into one number.

This exact text is machine-checked (`test_caveat_text_explicitly_bans_chemical_correctness_label`)
and lives alongside every generated report, not only here.

## Semantic firewall

- Tool-native "solved"/"route_found" and this harness's post-hoc "accepted"
  (structurally valid, stock-grounded, target-element-accounted) are always
  separate metrics.
- RENKIN's own internal validator is never used to grade AiZynthFinder's
  routes, and nothing from AiZynthFinder's stack grades RENKIN's routes —
  each tool is judged only by (a) its own self-report, (b) the identical
  common checks above, or (c) human review (not performed in this round).
- **Latency comparison firewall — no cross-tool inference-latency
  comparison is made in this round.** RENKIN's search budget is
  combinatorial (depth × beam — an unsolved target can terminate quickly
  once the beam is exhausted); AiZynthFinder's is temporal (`time_limit` —
  an unsolved target burns close to the full configured budget by
  construction), so an "N× faster" claim over *all* targets would measure
  the two budget definitions against each other, not the two search
  engines. Beyond that: every AiZynthFinder measurement in this round is a
  **cold-start** per-target Docker container invocation (container startup
  plus policy-model/stock load on every single target, the same per-target
  process-spawn cost the RENKIN adapter also pays, but AiZynthFinder's
  model/stock load is far heavier) — there is no persistent-worker or
  warm-latency arm in this round to separate `initialization_ms` from
  per-target `planning_ms`. `total_elapsed_ms` (both tools, all rows) and
  the `solved_only_total_elapsed_ms_percentiles` aggregate field are
  reported **only as raw, disclosed deployment-cost numbers, per tool, side
  by side with both tools' literal budget parameters** — never narrated as
  a comparative inference-latency claim, and never described as "licensed
  for direct comparison." A genuine warm-latency arm (separating
  `initialization_ms`, `planning_ms`, `cold_start_ms`, and throughput) is
  explicitly deferred to a future round.
- No route-accuracy or "RENKIN is better/worse" claim is licensed by any
  number in this document or its artifacts.

## Paired statistics (n=100: descriptive only)

Since both tools run on the identical target sample, every comparison is
**paired**. `scripts/compare_paired_report.py` joins the two tools' rows on
`target_id` and drives `scripts/compare_stats.py`, which implements:

- Paired bootstrap (resampling whole target-pairs, never each tool's
  results independently) for route-found-rate, latency, and memory
  differences — 10,000 iterations, fixed seed `1066`, 95% CI. This seed is
  a statistics-tool seed the harness controls; it has nothing to do with
  either retrosynthesis tool's own (documented or undocumented)
  determinism.
- An optional exact McNemar test (binomial-based, stdlib `math.comb`, no
  scipy dependency) on paired binary route-found outcomes, as a reference
  statistic alongside the bootstrap CI, never a replacement for it.

**At n=100, every number here is explicitly descriptive.** Wide confidence
intervals are expected and are shown, not narrated away — no
"statistically significant" claim is made at this sample size. The same
code ran unchanged at n=500 (see "500-target results" below); the
4,903-target full corpus remains not run.

## 500-target results

The n=500 round (Phase 1/2/3 of the formal Issue #66 protocol) is complete.
Full results, paired stats with exact McNemar p-values and 95% confidence
intervals, per-target audit, and reproduction commands:
`data/comparison/results_500/aggregate_report.md`.

Headline finding, stated with the same scoping discipline as this document's
n=100 numbers above but now with a statistically significant paired
difference at n=500: under this fixed 500-target sample, the shared
393-compound stock, and each tool's configured policy and search budget for
this run, RENKIN Conservative's `route_to_shared_stock` outcome was 9.8
percentage points higher than AiZynthFinder's (95% CI [7.0, 12.8], exact
McNemar p≈1.9e-11, RENKIN 73/500 vs AiZynthFinder 24/500). Per Arm B's own
framing above, this does not isolate search-engine quality in full — see
`aggregate_report.md`'s scoped interpretation for what is and is not
established by this number. The native-mode arm (Arm A) diverges in the
opposite direction (−48.6pt, AiZynthFinder ahead) driven by unmatched
conditions including, but not proven to be limited to, native stock size
(RENKIN ~402 vs AiZynthFinder ~17.4M compounds) — see the same document for
the full caveat. A Conservative-vs-Disabled ring-context-guard ablation
(Issue #72/#242) found no statistically significant difference at this
sample size (`data/comparison/results_500/conservative_vs_disabled.md`).

## RENKIN-specific diagnostics (`tool_specific.renkin`)

The RENKIN adapter (`scripts/compare_renkin_adapter.py`) wraps the **existing,
unmodified** `renkin` CLI — one subprocess per target, not a second batch
join against `renkin-bench`. `renkin-bench`'s own `BenchResult` has no route
tree (verified against source, not assumed), so a dual-binary join would
buy provenance-rich diagnostics that all land in `tool_specific.renkin`
anyway — explicitly excluded from every cross-tool metric — at the cost of
a whole new failure mode (two configs drifting apart, batch-vs-solo
cache-counter ambiguity). `tool_specific.renkin` is intentionally sparse as
a result: whatever the single per-target CLI response already contains
(confidence/convergency/success_probability/route_cost for a solved target;
nodes_expanded/matched_templates/stock_hits/beam_limit_hit/max_depth_reached
for an unsolved one), explicitly tagged `diagnostics_source:
"single_per_target_cli_call"`.

`--spectator-bond-policy` (v0.35.0's fail-closed gate) is a second,
**orthogonal** RENKIN-only policy axis alongside `--ring-context-policy` —
recorded as its own `spectator_bond_policy` field in the run manifest,
never folded into the same `configuration_id`/label as ring-context, and
run as its own separate arm the same way Conservative-vs-Disabled already
are. Only under `--spectator-bond-policy gated` (which the adapter always
pairs with `--search-diagnostics` so the CLI actually emits it) do the
common-schema fields `gated_out_candidate_count` (how many candidates the
search excluded for that target) and `gated_out_reasons` (`rule_name ->`
exclusion count) get populated; both stay `null` under `off`/
`diagnostics-only`, not `0`, since `0` legitimately means "gated, but
nothing was excluded".

## AiZynthFinder-specific notes (`tool_specific.aizynthfinder`)

`scripts/compare_aizynthfinder_adapter.py` drives `aizynthcli` inside
`docker/aizynthfinder.Dockerfile` (Python 3.11-slim, `linux/arm64`,
`aizynthfinder==4.4.1`, no `tf`/TensorFlow extra — onnxruntime is the
default and only inference engine this comparison uses). One container
invocation per target, the same per-target-timeout rationale as the RENKIN
adapter. AiZynthFinder exposes no documented random seed — its
Monte-Carlo-guided search is treated as potentially non-deterministic;
repeat-run variance characterization is deferred to a future round (n=100
uses a single run per target, per the frozen protocol).

**Output parsing (two real bugs found and fixed by the 5-target smoke
gate, before the 100-target run — see `data/comparison/smoke_gate_report.md`
for the full account):** `aizynthcli`'s `--output ... .json` is a pandas
`to_json(orient="table")` envelope (`{"schema": ..., "data": [<record>]}`),
not a bare records list; and a non-empty `trees` list does **not** mean
`is_solved=true` (AiZynthFinder always returns best-effort top-N candidate
routes regardless of whether any is fully stock-terminating) — `route_found`
tracks `record["is_solved"]` specifically, never `len(trees) > 0`.

**Native-mode stock-leaf validation is tool-trusted, not independently
re-verified.** AiZynthFinder's own startup log reports `Compounds in stock:
17,422,831` for the default ZINC configuration — too large to canonicalize
and independently re-check per row this round. For `comparison_mode=native`,
`all_leaves_in_configured_stock` reflects the tool's own per-leaf `in_stock`
claim, with an explicit `adapter_warning`
(`native_stock_trusted_not_independently_verified`) on every such row so
this is never silently conflated with shared-stock mode's genuine
independent re-verification (393 compounds, small enough to canonicalize
directly, same mechanism as the RENKIN adapter's own stock-leaf check).

## Known gaps (disclosed, not fixed in this round)

- `data/uspto50k_test.smi`'s header claims "5007 reactions" but the file
  has 4,907 data rows; no record in this repository traces the exact
  upstream Hugging Face revision/date this file was derived from, or
  explains the discrepancy. The file's own SHA-256 is stable and recorded,
  but "what exactly it's a hash of, from where, downloaded when" is an
  open provenance gap.
- USPTO-50k's own license/terms-of-use are not stated anywhere in this
  repository (unlike the ORD evidence corpus, which is explicitly
  documented as CC-BY-SA-4.0) — treat it as a research-use academic
  artifact derived from public US patent text, not an unambiguously
  OSI-licensed dataset.
- An independent RDKit re-parse of `data/building_blocks.smi` finds 393
  unique canonical/InChIKey structures / 9 parse failures — confirmed via
  the shared-stock construction (see "Provenance" above), which excludes
  the same 9 lines RDKit itself cannot parse — versus RENKIN's own loader's
  reported 402 unique / 3 parse failures. A parser-dependent ~8-9-compound
  gap (3 are unambiguous SMILES syntax errors in the checked-in file; 6 are
  heterocycle entries RDKit rejects on aromaticity/kekulization grounds
  that RENKIN's own parser currently accepts). This affects the stock
  file's data quality, not this comparison's sampling — worth a follow-up
  look, out of scope here. This is also why the shared-stock arm (393
  compounds) and RENKIN's native arm (402 compounds) are necessarily
  different-sized stocks — the shared arm is the intersection RDKit can
  independently verify, not RENKIN's full native list.
- The cross-tool 100/500/4,903-target sample (deduped by canonical SMILES,
  4 duplicate groups removed from the 4,907 raw `data/uspto50k_test.smi`
  rows) is a **different denominator** from RENKIN's historical
  986/756/43-out-of-**4,907** "corrected baseline" (frozen to commit
  `e20dc8c`). The two must never be compared directly — a rate computed
  against 4,903 targets is not commensurate with one computed against the
  original, non-deduped 4,907-row corpus.

## 500/full run gate

**500-target round: complete** (see "500-target results" above,
`data/comparison/results_500/`). **4,903-target full-corpus round: not
started**, and out of scope for this document's current results — per
explicit standing instruction, no run against the full corpus has been
performed. The gate conditions below were the criteria evaluated before the
500-target round was approved to proceed; they are retained here as a
historical record of that decision, not as an open gate for the 500-target
round (which has already run). A 4,903-target round would need its own,
separately-evaluated gate.

- the 100-target feasibility results are reviewed and judged worth scaling;
- ~~AiZynthFinder's repeat-run variance... characterized on at least 3
  independent repetitions~~ **done**: `data/comparison/results_100_repeatability/repeatability_report.md`
  characterizes 4 total runs per arm for AiZynthFinder (native and
  shared-stock) and 2 total runs per arm for RENKIN. Finding: both tools'
  solve-state (`route_found`) is stable across repeats (RENKIN
  byte-identical modulo one disclosed boundary-timeout target per arm;
  AiZynthFinder's solve/not-solve status unanimous across all 4 runs, both
  arms) — but AiZynthFinder's *specific route selection* has measurable
  run-to-run variance even among consistently-solved targets (9.1% of
  always-solved native targets, 1/4 shared-stock). A single-run result is
  no longer the sole basis for this round's paired comparisons;
- Issue #72 (extracted templates carry no ring-topology information, so
  ring-breaking disconnections go undetected) is resolved, or its scope at
  500/4,903 targets is explicitly disclosed — otherwise
  `target_elements_accounted_route_rate` at a larger scale would carry the
  same undiagnosed correctness gap this round's data does;
- the known gaps above (corpus provenance, building-block parser
  discrepancy) are either resolved or explicitly re-disclosed at the larger
  scale;
- compute/time budget for a much larger sequential sweep is confirmed
  (this Mac's Docker VM allocation and shared, non-dedicated hardware were
  adequate for 100 targets run sequentially; 500 or 4,903 is a materially
  larger commitment — note one of this round's own repeat runs needed a
  manual retry after AiZynthFinder's public-data mount was found empty,
  worth planning around for a longer unattended sweep).

## Reproduction

```bash
# 1. Sampling (offline, deterministic, already frozen and checked in)
python3 -m venv .venv-compare-66
.venv-compare-66/bin/pip install -r scripts/requirements-compare-66.txt
.venv-compare-66/bin/python scripts/compare_sampling.py \
    --corpus data/uspto50k_test.smi \
    --output-manifest data/comparison/sample_manifest.json \
    --output-list data/comparison/sample_full_sorted.jsonl

# 2. RENKIN binary
cargo build --release --bin renkin

# 3. AiZynthFinder image + public data (network required for this step only)
docker build --platform linux/arm64 -f docker/aizynthfinder.Dockerfile \
    -t renkin-compare-66/aizynthfinder:4.4.1 .
docker run --rm -v "$(pwd)/data/comparison/aizynthfinder_public_data:/public" \
    renkin-compare-66/aizynthfinder:4.4.1 download_public_data /public

# 4. Shared-stock construction (Arm B only) -- zero-diff by construction,
#    written directly to both a RENKIN-format .smi and an AiZynthFinder HDF5
.venv-compare-66/bin/python scripts/compare_shared_stock.py
cp data/comparison/shared_stock/shared_stock.hdf5 \
    data/comparison/aizynthfinder_public_data/shared_stock.hdf5

# 5. Run the 100-target feasibility sample, per tool per mode
.venv-compare-66/bin/python scripts/compare_run.py \
    --tool renkin --comparison-mode native --sample-size 100 \
    --output-rows data/comparison/results_100/renkin_native.jsonl \
    --output-aggregate data/comparison/results_100/renkin_native_aggregate.json

.venv-compare-66/bin/python scripts/compare_run.py \
    --tool aizynthfinder --comparison-mode native --sample-size 100 \
    --output-rows data/comparison/results_100/aizynthfinder_native.jsonl \
    --output-aggregate data/comparison/results_100/aizynthfinder_native_aggregate.json

.venv-compare-66/bin/python scripts/compare_run.py \
    --tool renkin --comparison-mode shared_stock --sample-size 100 \
    --output-rows data/comparison/results_100/renkin_shared_stock.jsonl \
    --output-aggregate data/comparison/results_100/renkin_shared_stock_aggregate.json

.venv-compare-66/bin/python scripts/compare_run.py \
    --tool aizynthfinder --comparison-mode shared_stock --sample-size 100 \
    --output-rows data/comparison/results_100/aizynthfinder_shared_stock.jsonl \
    --output-aggregate data/comparison/results_100/aizynthfinder_shared_stock_aggregate.json

# 6. Paired statistics (bootstrap + McNemar) and per-target join table,
#    per mode -- joins the two tools' rows from step 5 on target_id
.venv-compare-66/bin/python scripts/compare_paired_report.py --mode native \
    --renkin-rows data/comparison/results_100/renkin_native.jsonl \
    --aizynthfinder-rows data/comparison/results_100/aizynthfinder_native.jsonl \
    --output-stats data/comparison/results_100/paired_stats_native.json \
    --output-table data/comparison/results_100/paired_table_native.json

.venv-compare-66/bin/python scripts/compare_paired_report.py --mode shared_stock \
    --renkin-rows data/comparison/results_100/renkin_shared_stock.jsonl \
    --aizynthfinder-rows data/comparison/results_100/aizynthfinder_shared_stock.jsonl \
    --output-stats data/comparison/results_100/paired_stats_shared_stock.json \
    --output-table data/comparison/results_100/paired_table_shared_stock.json

# 7. 500-target round (complete, see "500-target results" above). Each arm is
#    its own independently resumable job -- --resume skips target_ids already
#    present in --output-rows, flushing+fsyncing every new row immediately.
#    --manifest-path records binary/commit/Docker/input-file hashes and host
#    environment at arm start and end. RENKIN's official configuration for
#    this comparison is --ring-context-policy conservative (Issue #72/#242);
#    Disabled is an ablation-only arm, not a headline arm.
.venv-compare-66/bin/python scripts/compare_run.py \
    --tool renkin --comparison-mode shared_stock --sample-size 500 --resume \
    --ring-context-policy conservative \
    --ring-context-sidecar data/ring_context_metadata_500.json \
    --output-rows data/comparison/results_500/renkin_conservative_shared_stock/rows.jsonl \
    --output-aggregate data/comparison/results_500/renkin_conservative_shared_stock/aggregate.json \
    --manifest-path data/comparison/results_500/renkin_conservative_shared_stock/manifest.json
# ... repeated per arm (renkin_conservative_native, aizynthfinder_shared_stock,
#     aizynthfinder_native, renkin_disabled_shared_stock, renkin_disabled_native)
#     with --tool/--comparison-mode/--ring-context-policy set accordingly.

# 8. Post-arm integrity verification (exact 500/500 coverage, no
#    duplicate/missing targets, schema validation, route_found<=>hash
#    invariant, manifest cross-check)
.venv-compare-66/bin/python scripts/compare_verify_arm.py \
    --rows data/comparison/results_500/<arm>/rows.jsonl \
    --manifest data/comparison/results_500/<arm>/manifest.json \
    --sample-list data/comparison/sample_full_sorted.jsonl --sample-size 500

# 9. Headline paired statistics, 500-target round
.venv-compare-66/bin/python scripts/compare_paired_report.py --mode shared_stock \
    --renkin-rows data/comparison/results_500/renkin_conservative_shared_stock/rows.jsonl \
    --aizynthfinder-rows data/comparison/results_500/aizynthfinder_shared_stock/rows.jsonl \
    --output-stats data/comparison/results_500/paired_stats_shared_stock.json \
    --output-table data/comparison/results_500/paired_table_shared_stock.json

.venv-compare-66/bin/python scripts/compare_paired_report.py --mode native \
    --renkin-rows data/comparison/results_500/renkin_conservative_native/rows.jsonl \
    --aizynthfinder-rows data/comparison/results_500/aizynthfinder_native/rows.jsonl \
    --output-stats data/comparison/results_500/paired_stats_native.json \
    --output-table data/comparison/results_500/paired_table_native.json
```

## Interpretation rules (summary)

1. Never merge tool-native "solved" with post-hoc "accepted".
2. Never call a `target_element_accounting_status=accounted` route
   "chemically correct" — it is a directional per-element inequality, not
   exact mass conservation, and not a chemistry-correctness judgment.
3. Never treat all-target latency (mixing combinatorial and temporal search
   budgets) as a comparative speed claim, and never describe solved-only
   latency as licensed for direct cross-tool inference-latency comparison —
   see "Latency comparison firewall".
4. Never use RENKIN's validator to judge AiZynthFinder's output or vice
   versa.
5. Never claim statistical significance from the n=100 round, and never
   treat a single AiZynthFinder run as sufficient evidence for a paired
   comparison given its undocumented search-seed behavior. (The n=500
   round's shared_stock arm *does* reach statistical significance — see
   "500-target results" — but rule 7 still applies to it.)
6. Never compare the 4,903-target cross-tool corpus against the historical
   4,907-row RENKIN-only "corrected baseline" as if they were the same
   denominator.
7. Never treat this document, or any number in it, as a superiority claim
   for RENKIN, AiZynthFinder, or any tool — including the n=500 shared_stock
   result. A statistically significant paired difference under this
   protocol's fixed sample, stock, and configured policies/budgets is not
   the same claim as "RENKIN's search capability is better." Never claim
   the native-mode difference is caused by stock size alone — the arm does
   not control for anything else, so only the *direction-reversal* between
   native and shared_stock is licensed as evidence of stock sensitivity.
8. Never report a cross-tool percentage-point difference without also
   giving both tools' raw numerator/denominator, the paired discordant
   counts, the 95% CI, and the exact p-value — a bare percentage is not
   auditable.
