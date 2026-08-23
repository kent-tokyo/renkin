# RENKIN vs. SynPlanner v1.6.0 — Competitive Baseline (Phase 0)

Status: **Phase 0 complete.** Artifact provenance, dependency/license
audit, and a real-object route-export investigation done against the
actual installed `SynPlanner==1.6.0` package (installed into a disposable
venv, removed immediately after — see
`tests/fixtures/synplanner/v1.6.0/PROVENANCE.md`). No RENKIN code changed.
This doc is one of two Phase 0/Phase 1-PR1 deliverables (the other is
`docs/design/synplanner-adapter-v1.md`); the "今回の実行範囲" (this
round's scope) explicitly excludes MCTS/model training/benchmarking —
this comparison is provenance and structural, not a route-quality
benchmark.

## 0. What this is, in one paragraph

SynPlanner is a real, actively developed, MIT-licensed CASP toolbox
(GitHub `Laboratoire-de-Chemoinformatique/SynPlanner`) that ships an
end-to-end pipeline RENKIN does not currently have: data curation, atom
mapping, rule extraction, trainable policy/value models, and MCTS
planning, plus route comparison/clustering/quality analysis and an HTML/
GUI layer. RENKIN's own strengths — a Rust-native core, WASM execution, a
policy-aware fail-closed audit pipeline, and cross-planner route auditing
— sit in a different, complementary part of the same problem space. This
document classifies each capability axis strictly against **evidence
gathered this round** (exact artifact, real source reading, or a genuine
fixture generated via SynPlanner's own code), never against assumption or
prior general knowledge of similar tools.

## 1. Exact artifact identity

- **Repository**: `github.com/Laboratoire-de-Chemoinformatique/SynPlanner`.
- **Tag**: `v1.6.0` → commit `38f929a69d5dc9d51823b355bf69dd00cc32bb9f`
  (`target_commitish: main`), published 2026-08-04T21:39:52Z (confirmed
  via `gh api repos/.../releases/tags/v1.6.0`).
- **PyPI package name is `SynPlanner`** (not `synplan`, the import name) —
  `pip install SynPlanner==1.6.0`. Wheel-only release: `synplanner-1.6.0-
  py3-none-any.whl`, 363,580 bytes. **No sdist published for 1.6.0.**
- **License: MIT** (confirmed via GitHub's `license` API) — no
  non-commercial or academic-only restriction on the code itself. (Model
  weights/datasets distributed separately via HuggingFace may carry their
  own, separate license terms — not audited this round.)
- **Confirmed by installing it**, not just reading the PyPI page: resolved
  dependency versions actually pulled were `torch==2.13.0`,
  `torch-geometric==2.8.0.post1`, `pytorch-lightning==2.6.5`,
  `rdkit==2026.3.5`, `chython-synplan==1.101` (pinned exact by SynPlanner
  itself), `chytorch-synplan==1.70`, `chytorch-rxnmap-synplan==1.7`,
  `huggingface-hub==1.28.0`, plus `streamlit`/`streamlit-ketcher`
  (GUI), `ipykernel`/`ipywidgets` (Jupyter), `matplotlib`/`pandas`/
  `pydantic`. Total installed venv size: **1.6GB**. Python `>=3.10,<3.15`
  required.

## 2. Comparison table

Each row classified as **SynPlanner wins** / **RENKIN wins** / **parity**
/ **not measured** — never guessed into a category. "Not measured" is a
real, tracked gap, not a soft "probably fine."

| Axis | Verdict | Evidence |
|---|---|---|
| Data curation (custom reactions in) | SynPlanner wins | Real, documented CLI pipeline (`reaction_mapping`, `reaction_standardizing`, `reaction_filtering`) exists and ships; RENKIN has no equivalent today. |
| Atom mapping | SynPlanner wins | Requires `chython-synplan==1.101` pinned exact; a "two ways to map" doc page is referenced in the v1.6.0 CHANGELOG (one GPU-based). RENKIN has zero atom-mapping capability of its own — it consumes mapping from source tools/adapters or reports `not_evaluable`. |
| Rule extraction | SynPlanner wins | Real shipped `rules_extraction.yaml` config confirmed (`min_popularity`, `single_product_only`, `environment_atom_count`, `include_rings`, etc.) — a full configurable extraction pipeline. RENKIN's own `scripts/extract_templates.py` is a much simpler rdchiral-based extractor with no comparable per-atom retention config. |
| Model training (policy/value) | SynPlanner wins | Real, documented `ranking_policy_training` CLI command and PyTorch Lightning training path exist and ship. RENKIN has no training capability at all today. |
| A* / beam search | Parity | RENKIN has both (`--search-mode a-star` / beam, plus coverage mode). SynPlanner's own MCTS is a different algorithm family (see next row) — not a direct comparison, but both tools have *a* working configurable search. |
| MCTS | SynPlanner wins | Real, shipped `configs/planning_*.yaml` confirm `search_strategy: expansion_first\|evaluation_first`, `max_iterations`/`max_tree_size`/`max_time`/`max_depth`, UCB/backprop config. RENKIN has no MCTS implementation. |
| Custom stock | Parity | RENKIN supports a `--stock` SMILES file today. SynPlanner's own stock-configuration mechanism was not independently audited this round (not measured beyond the `in_stock` field's export semantics — see the adapter design doc). |
| Route export | Parity | Both tools export a JSON route representation. SynPlanner's `RouteNode` schema is real and richer in one dimension (`rule_id`/`rule_source`/`rule_key` provenance) but carries **no atom-mapping field at all** — confirmed by reading `contracts.py`/`io/json.py` source and by generating real fixtures this round (see §3/§5 below and the adapter design doc). RENKIN's own native/AiZynthFinder/Syntheseus formats already handle 3 real tool shapes today. |
| Route comparison | Not measured | SynPlanner's `synplan.chem.reaction.routes.representation`/`analysis` modules exist (confirmed present) but their comparison semantics weren't read from source this round. RENKIN has no dedicated route-comparison feature today (a future `RouteDeltaGraph`, per the user's own longer-term roadmap, is unimplemented). |
| Route clustering | SynPlanner wins (real, RENKIN has none) | Real, documented "group routes by strategic bonds" feature (`synplan.chem.reaction.routes.clustering`), citing a real paper (Gilmullin et al., chemrxiv 2025). Not independently verified beyond the module existing and the one-line description. RENKIN has no clustering feature. |
| Route quality (scoring) | SynPlanner wins (real, RENKIN has none as a single score) | Real "competing-sites scoring for functional-group selectivity" feature (`synplan.chem.reaction.routes.quality`), citing Westerlund et al., chemrxiv 2025 — exact scoring formula not read from source this round (not measured). RENKIN's own audit pipeline reports many *independent* findings rather than one quality score — different design, not a worse one, see the "auditability" row. |
| Protection-strategy awareness | Not measured | Neither tool's protecting-group handling was investigated this round. |
| Typed API | Parity | RENKIN ships a typed Python `AuditRouteReport` (`renkin.audit_route_report`) alongside its string API. SynPlanner exposes a real Python API (confirmed: `write_routes_json`, `build_route_trees`, etc.) but whether it has an equivalently typed/dataclass-based *report* API (as opposed to raw functions) wasn't audited. |
| Reproducibility | RENKIN wins | RENKIN's `AuditManifest` records input/stock SHA-256, policy, and `renkin_version` deterministically for every audit (`docs/guides/audit-reproducibility-contract.md`). SynPlanner's route export is deterministic *given the same reaction objects* (confirmed empirically this round — reran fixture generation, byte-identical output) but has no equivalent manifest/provenance-record convention of its own that was found. |
| Auditability (cross-tool, policy-aware) | RENKIN wins | This is RENKIN's core differentiator and SynPlanner has no equivalent: a fail-closed, policy-aware (`informational`/`standard`/`strict`) audit pipeline that never lets policy change the underlying finding set (`policy_never_changes_the_finding_set_only_the_...` tests, `src/bridge/audit.rs`), applicable across native/AiZynthFinder/Syntheseus routes today. |
| Deterministic mode | Parity-leaning-RENKIN | RENKIN's whole audit path is deterministic by construction (no model inference in the audit itself). SynPlanner's *planning* search involves model inference (inherently less deterministic run-to-run unless seeded identically) — but SynPlanner's *route export* from a fixed set of already-decided reactions is itself deterministic (confirmed this round). Different layers being compared; not apples-to-apples. |
| Browser / WASM | RENKIN wins | RENKIN ships real WASM builds (`wasm-pack`, published to npm, live in the Playground). SynPlanner is a pure Python/PyTorch stack with no browser execution path — confirmed by its dependency list (`torch`, `torch-geometric`, etc., none of which run in a browser). |
| Install footprint (base) | RENKIN wins | RENKIN's Rust core / CLI / WASM ship with **no heavy ML dependency required** for audit functionality. SynPlanner's install is **1.6GB** (measured this round, venv total) purely to import its route-export function — `torch`/`torch-geometric`/`pytorch-lightning`/`rdkit` are hard, non-optional dependencies of the `SynPlanner` PyPI package itself, not just of its training path. |
| Offline operation | Parity | RENKIN's core operates fully offline. SynPlanner's route *export* function (what this round exercised) also required no network access once installed — but real *planning* needs a downloaded model checkpoint first (not measured how large, or whether re-downloadable offline afterward). |
| Cross-planner compatibility | RENKIN wins | RENKIN already normalizes 3 real external formats (native, AiZynthFinder, Syntheseus) into one shared `RouteDocument`, with a 4th (SynPlanner) now designed (not yet implemented, see the adapter doc) against a real fixture. SynPlanner has no equivalent cross-tool ingestion — it operates on its own route format only, as far as this round's audit found. |
| Benchmark evidence | Not measured | No benchmark was run this round (explicitly out of scope — "重い benchmark は行わない"). Neither tool's solved-rate/latency/memory was measured against the other. This is the largest remaining gap before any "RENKIN beats SynPlanner" claim would be meaningful, per the user's own Phase 7 "Final SynPlanner-Surpass Gate." |

## 3. The single most important confirmed finding: no ONNX, no swappable inference contract

SynPlanner's policy/value models are loaded as raw PyTorch Lightning
`.ckpt` files by filesystem path (`--policy_network path/to.ckpt`); the
README's own quick-start example confirms this
(`--policy_network synplan_data/policy/supervised_gps/v1/v1/ranking_policy.ckpt`).
Combining a filtering+ranking checkpoint pair is explicitly refused if
their rule-head counts don't match (a real, documented compatibility
guard) — but there is no documented export path to a lighter runtime
format (ONNX or otherwise), and no evidence of one anywhere in the
package's dependency list or config files read this round.

RENKIN's own "ONNX + manifest, no PyTorch required at inference" design
(already shipped, used by the existing reranker/bond-index scorers) is
therefore a **confirmed real differentiator**, not an assumed one — this
is the single clearest piece of evidence for why RENKIN's eventual
policy/value model support (the user's own longer-term Phase 5) should
stay ONNX-first rather than adopting SynPlanner's PyTorch-native pattern.

## 4. Route export schema — what's real, what's not measured

Read directly from `synplan/chem/reaction/routes/contracts.py` and
`io/json.py` at the v1.6.0 tag, then confirmed by actually generating real
fixtures through the installed package this round (full detail in
`tests/fixtures/synplanner/v1.6.0/PROVENANCE.md` and
`docs/design/synplanner-adapter-v1.md`):

- `RouteNode` is a plain `TypedDict` — `type`, `smiles`, `children`,
  `in_stock` (mol only), `meta`/`step_id`/`tree_node_id`/`rule_id`/
  `rule_source`/`rule_key` (reaction only, several only populated if the
  caller passes optional metadata).
- **No *dedicated* atom-mapping field in the schema — but the reaction
  node's own `smiles` string does carry atom maps**, confirmed by this
  round's real fixtures (e.g. `[CH3:1][CH2:2][OH:3]>>[CH3:1][CH2:2][Cl:3]`).
  Whether SynPlanner-sourced routes land in `forward_validation:
  not_evaluable` or actually get replayed therefore depends on whether a
  future adapter passes that `smiles` string through unchanged to
  `forward.rs`'s existing `has_atom_mapping` check — unlike AiZynthFinder
  and Syntheseus, this is **not settled** yet; see the adapter design
  doc's §7 open question.
- The real top-level file shape is `{route_id: RouteNode}` — an object
  keyed by route ID, not a bare tree or an array (confirmed by running
  the real exporter, not assumed from the TypedDict declaration alone).
- Malformed routes are dropped with a logged warning, or raise
  `RouteExportError` under `strict=True` — confirmed both ways by running
  real (deliberately broken) input through the installed package.

## 5. Not measured — explicit list, not silently dropped

- Real install size for a *minimal* SynPlanner install (this round
  installed the full package including GUI/Jupyter extras — no attempt
  was made to find a lighter extras subset).
- Cold-import time.
- `RouteCGR`/clustering/quality-scoring exact algorithm internals (module
  existence and one-line description confirmed; implementation not read).
- The exact atom-mapping tool/model used by the GPU-based mapping option
  referenced in the CHANGELOG (not independently confirmed).
- Any real MCTS-searched route's actual output (would require a
  downloaded trained checkpoint — out of scope this round).
- Any benchmark comparison (solved rate, latency, memory) — entirely out
  of scope this round per explicit instruction.

## 6. What this changes for RENKIN's roadmap (context, not a decision made here)

This doc doesn't decide anything about later phases — it's evidence for
decisions the user's own longer-term plan will make later. Two things
worth flagging for that future reading, though: (1) SynPlanner's real
rule-provenance fields (`rule_id`/`rule_source`/`rule_key`) are new
information RENKIN's `ReactionEvidence` enum doesn't carry for *any*
adapter today — a genuine design question for Phase 1 PR2, not unique to
SynPlanner; (2) the "no ONNX" finding in §3 is direct, confirmed support
for keeping RENKIN's own future policy/value model work ONNX-first rather
than following SynPlanner's PyTorch-checkpoint pattern.
