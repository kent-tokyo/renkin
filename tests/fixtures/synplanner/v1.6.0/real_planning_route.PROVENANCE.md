# SynPlanner v1.6.0 real-planning-output fixture provenance

This is the Phase 1 PR1.5 follow-up to `PROVENANCE.md` (the original
Phase 0/PR1 fixtures, which were built by hand-constructing `chython`
reaction objects and running them through SynPlanner's own real exporter,
*without* running an actual MCTS search). These fixtures are different:
they come from a real, CPU-only, pretrained-model-backed retrosynthetic
planning search, run start-to-finish through SynPlanner's own standard
CLI entry point (`synplan planning`), not hand-built input.

## Why this round exists

The Phase 0/PR1 adapter design doc (`docs/design/synplanner-adapter-v1.md`
§7) left two open questions unresolved because hand-constructed fixtures
can't answer them:

1. Does a real MCTS-searched route's exported top-level JSON shape match
   what the hand-built fixtures showed?
2. Does the exported reaction `smiles` field retain atom maps from a real
   planning run, and are those maps consistent/usable for forward replay
   -- or was that only true of the specific hand-built examples?

Both are now resolved with real evidence. See "Findings" below.

## Model/checkpoint provenance

**HuggingFace repo**: `Laboratoire-De-Chemoinformatique/SynPlanner-data`
(confirmed via `synplan/utils/loading.py`'s `REPO_ID` constant, not
guessed). **License: MIT** (confirmed via the HF Hub API's `cardData.license`
field before any download was attempted). **Not gated** (`"gated": false`
in the HF API response) -- no authentication/access request needed.

**Preset used**: `synplanner-gps` (the CLI/`download_preset()` default),
downloaded via `synplan.utils.loading.download_preset("synplanner-gps")`,
which resolves `presets/synplanner-gps.yaml` and downloads every file it
lists. Preset description (from the YAML itself): "SynPlanner GPS++
release: USPTO rules with relaxed-degree extraction + GPS multihead
ranking policy".

| Component | Repo path | Size (bytes) | SHA-256 |
|---|---|---|---|
| Ranking policy checkpoint | `policy/supervised_gps/v1/v1/ranking_policy.ckpt` | 78,307,767 | `01cf37995b55a308ac285ce14d98196893673f0383f85d370afa01286d7cc7c7` |
| Filtering policy checkpoint (downloaded, unused this round) | `policy/supervised_gcn/v1/v1/filtering_policy.ckpt` | 312,497,465 | `a0d7e93690b447fae65d04af7ea5cdf1058d2831110dbb49b4d11ed2570e0f96` |
| Value network checkpoint | `value/supervised_gcn/v1/value_network.ckpt` | 15,856,133 | `dbace1da49c1d400f43f667f6e6691a869472835461218d958803c001c0ea34c` |
| Reaction rules | `policy/supervised_gps/v1/reaction_rules.tsv` | 6,937,537 | `46a34d9b59c8b9917808fdb7f99640afd8e0ff697e4964e7d18226c8fac6b382` |
| Building blocks | `building_blocks/emolecules-salt-ln/building_blocks.tsv` | 7,214,080 | `14e12f947bfa5d0f6cb9bff61715411d5d69189a31c4b4b759f0db8e65f92bda` |

The `filtering_policy.ckpt` was downloaded as part of the preset but never
loaded/used -- this round's config uses `evaluation_type: gcn` (ranking
policy for node expansion + value network for node evaluation), the
combination the preset's checkpoints were actually trained to be used
with together. `combined_policy` (filtering+ranking combined) was
deliberately not attempted: `configs/planning_combined_policies.yaml`
(GitHub, v1.6.0 tag) documents that `synplanner-gps`'s two policy heads
know different rule-set sizes (24094 vs. 11235 rules) and "loading them
together is refused" -- not the intended pairing for this preset.

**Redistribution check**: the committed fixture files below contain no
model weights, only molecule/reaction SMILES strings and small integer
route-tree structure derived by *running* the (MIT-licensed) model --
analogous in kind to the Phase 0 fixtures, which likewise derived output
from real tool execution rather than redistributing any input asset
verbatim. No license blocker.

## Software

Same disposable-venv discipline as the original PROVENANCE.md: `python3.13`
(`/opt/homebrew/bin/python3.13`), `pip install SynPlanner==1.6.0`, resolved
versions identical to the Phase 0 install (`torch==2.13.0`,
`torch-geometric==2.8.0.post1`, `pytorch-lightning==2.6.5`,
`rdkit==2026.3.5`, `chython-synplan==1.101`, `chytorch-synplan==1.70`,
`chytorch-rxnmap-synplan==1.7`, `huggingface-hub==1.28.0`). Capture date:
2026-08-23. Disk headroom checked before (~13GB free) and confirmed
reclaimed after cleanup; venv + downloaded checkpoints removed
immediately after fixture capture, same session, not deferred.

## Real planning run (not a hand-built `routes_dict`)

**Entry point used**: the actual `synplan planning` CLI command --
`synplan.interfaces.cli.planning_cli`, the same command SynPlanner's own
users run, not a call directly into `write_routes_json`.

**Exact command**:
```
synplan planning \
  --config planning_minimal.yaml \
  --targets targets.smi \
  --reaction_rules policy/supervised_gps/v1/reaction_rules.tsv \
  --building_blocks building_blocks/emolecules-salt-ln/building_blocks.tsv \
  --policy_network policy/supervised_gps/v1/v1/ranking_policy.ckpt \
  --value_network value/supervised_gcn/v1/value_network.ckpt \
  --results_dir results \
  --export_routes
```

**Config** (`planning_minimal.yaml`, adapted from the official
`configs/planning_value.yaml` at the v1.6.0 tag with only `max_iterations`
100->30 and `max_time` 600->120 reduced, per this round's "minimal budget"
requirement -- every other field is the unmodified official value):
```yaml
tree:
  max_iterations: 30
  max_tree_size: 1000000
  max_time: 120
  max_depth: 6
  search_strategy: evaluation_first
  ucb_type: uct
  c_ucb: 0.1
  backprop_type: muzero
  evaluation_agg: max
  exclude_small: True
  init_node_value: 0.5
  min_mol_size: 6
  epsilon: 0.0
  silent: True
node_evaluation:
  evaluation_type: gcn
node_expansion:
  top_rules: 50
  rule_prob_threshold: 0.0
  priority_rules_fraction: 0.5
```

**Target**: aspirin, `CC(=O)Oc1ccccc1C(=O)O` -- a single well-known
molecule, not a batch or benchmark set. CPU-only (`CUDA_VISIBLE_DEVICES=""`),
sequential, no model training.

**Result**: solved in 3 seconds (well under the 120s/30-iteration budget --
search stopped naturally on convergence, no artificial early-stop was
needed). **167 distinct winning routes** found for this one target (many
policy-ranked alternative disconnections at depth 1-2). No target/config/
checkpoint/timeout failure to report -- this did not hit the "no route
obtained" contingency.

**Determinism**: the entire search + export was re-run a second time,
independently, from scratch (fresh `results_dir`). Both the search result
(identical 167 routes, `diff` on the pretty-printed JSON is empty) and the
`--export_routes` artifact (`results.json.gz`, decompressed and
JSON-reparsed) were **byte-for-byte/structurally identical** across the
two runs. This is stronger than "export-stage only" determinism -- the
MCTS search itself was deterministic here (CPU-only, `epsilon: 0.0`, no
observed nondeterminism from library-level parallelism at this scale).

## Real code path exercised for export (confirmed by reading the actual
v1.6.0 source, cross-checked against what actually ran)

`run_search` (`synplan/mcts/search.py`) writes **three different real
output artifacts per solved target**, from two different exporter
functions -- a materially different picture than Phase 0's fixtures alone
suggested, since Phase 0 only ever called `write_routes_json` directly:

1. **`extracted_routes.json`** (`extract_routes(tree)` from
   `synplan/utils/visualisation.py`): a list-of-alternates format with
   `rule_key`/`policy_rank` on each reaction node, but **no reaction-level
   `smiles`/SMIRKS field at all** -- not usable for atom-mapping analysis
   or forward-replay, only for rule-choice/ranking inspection. Not
   committed as a fixture (schema out of scope for the adapter's atom-
   mapping question; noted here so a future reader doesn't assume it's
   the same shape as the other two).
2. **`extracted_routes_html/mapped_routes_{ti}.json`**: the actual
   `write_routes_json(routes_dict, ...)` output, where `routes_dict` comes
   from `extract_reactions(tree, reconcile_atom_mapping=reconcile_atom_mapping)`
   (default `False`, i.e. `--reconcile-mapping` was **not** passed this
   round -- the fast, "per-step-local" numbering path). This is the same
   function Phase 0 exercised with hand-built input; here it's fed a real
   `tree.synthesis_route()` result. **Top-level shape**: `{route_id:
   RouteNode}`, confirming Phase 0's finding holds for real search output
   too.
3. **`manifest.json` + `results.json.gz`** (only written because
   `--export_routes` was passed): `export_routes_artifact()`'s output,
   built from `build_target_routes(tree, reactions=routes_dict)` --
   **the same `routes_dict`, reused, not independently recomputed**. This
   is a *different, explicitly versioned* wrapper the source code itself
   calls "the public route-export contract"
   (`ROUTE_EXPORT_SCHEMA_VERSION = "synplan-routes/1"`, a module-level
   constant with an explicit "bump when the envelope/manifest shape
   changes" comment). Genuinely new finding, not visible from Phase 0's
   source reading alone (Phase 0 never exercised `--export_routes`):
   - `manifest.json` is a small, versioned envelope:
     `{"schema_version": "synplan-routes/1", "synplan_version": "1.6.0",
     "directives": {"adapter": "synplanner", "raw_results_filename":
     "results.json.gz"}}`. **`directives.adapter == "synplanner"` is an
     explicit, unambiguous, first-class format-detection signal** -- far
     more reliable than any structural heuristic on the bare `RouteNode`
     shape (which is what §3.2 of the adapter design doc had to propose
     in Phase 0, for lack of anything better at the time).
   - `results.json.gz` (gzip-compressed JSON) is keyed by
     **RDKit-canonical target SMILES**, not by an integer route ID:
     `{target_smiles: [RouteNode, RouteNode, ...]}`, `[]` for unsolved
     targets. Each `RouteNode` inside the list is structurally identical
     in shape to the `mapped_routes_{ti}.json` entries (verified this
     round by exact structural equality, not just "looks similar") --
     same inner schema, different outer wrapper.

**Route metadata fields confirmed absent from real CLI usage**: neither
`run_search` nor the CLI ever builds/passes a `route_metadata` argument to
`write_routes_json` or `build_target_routes`. So in real, standard `synplan
planning` usage, **`rule_id`/`rule_source`/`rule_key`/`meta`/`step_id`/
`tree_node_id` never appear** in either the internal or public-contract
`RouteNode` export -- they are opt-in machinery (confirmed in Phase 0)
that the shipped CLI itself simply never opts into. (Rule identity/rank
information *is* available in real output, but only via the separate,
differently-shaped `extracted_routes.json`, not via either `RouteNode`
export path.) This refines, not contradicts, Phase 0's finding -- the
opt-in mechanism is real, it's just never exercised by default usage.

## Atom-mapping findings (the central question this round exists to answer)

A standalone diagnostic script (not committed -- see the two `unittest`
test classes in `scripts/tests/test_synplanner_real_route_fixture.py`,
which assert the same properties directly against the committed fixture
files) audited **all 317 reaction nodes across all 167 real routes**:

- **100% (317/317)** have non-empty atom mapping on the reaction `smiles`
  field.
- **0** have a duplicate atom-map number on either the reactant or product
  side.
- **0** have a product-side atom-map number that doesn't trace back to a
  reactant-side atom (i.e. no orphan/fabricated atoms -- real conservation).
- **Cross-step consistency, across all 150 real parent-reactant /
  child-product boundary pairs found in the 2+-step routes: 100%
  byte-identical.** This is despite `route_cgr.py`'s own docstring
  describing the default (non-`--reconcile-mapping`) path as using
  "per-step-local" numbering that *skips* the expensive cross-step
  reconciliation (`compose_route_cgr`) -- the empirical result is that,
  for `tree.synthesis_route()`-derived routes, the fast path's numbering
  turns out to already be globally consistent in every observed case,
  not just "not guaranteed inconsistent." (Mechanistically plausible: the
  tree's backward-decomposition construction passes the *same* mapped
  molecule object from being one step's reactant to being the next step's
  decomposition target, rather than independently re-mapping it -- but
  this round's evidence is empirical, not a proof from source reading, and
  is scoped to this preset/target/config only.)
- **Forward-replay tested directly with RDKit** (not RENKIN's Rust code --
  a standalone check, no `ReactionEvidence::SynPlannerReaction` variant
  exists yet): for all 3 reaction nodes in the two committed fixtures,
  `AllChem.ReactionFromSmarts(smirks, useSmiles=True)` followed by
  `RunReactants` on the mapped reactants reproduces the target molecule
  (atom-map-stripped, canonical-SMILES-compared) exactly.

**Classification per this round's own A/B/C/D scheme: Category A** --
real planning reactions have valid, usable atom maps and are genuinely
forward-evaluable, for every case sampled this round. RENKIN's existing
`has_atom_mapping` gate in `src/bridge/forward.rs` (a `:` + ASCII-digit
scan) would pass every one of these 317 reactions unchanged -- confirmed
by applying the identical regex, not just asserted.

**Scope of this finding**: one target (aspirin), one preset
(`synplanner-gps`), one config (`evaluation_type: gcn`,
`--reconcile-mapping` not passed), CPU-only, 167 routes from a single
search. Not a claim about every SynPlanner preset, target, or
`--reconcile-mapping=true` path -- those would need their own evidence
before a Phase 1 PR2 implementation treats this as universally guaranteed.

## In-stock field

Every mol-leaf node across both real 167-route runs has `in_stock` set to
a definite `true`/`false` -- never absent or `null`. Consistent with
Phase 0's source-reading finding (the export code has no path that leaves
it unset), now reconfirmed against real search output at scale (not just
two hand-built examples).

## Committed fixtures

- `real_planning_route_1step.json` -- route `"2"`, a single-step real
  route (acetic anhydride + salicylic acid -> aspirin), sliced verbatim
  from the real `mapped_routes_0.json` (`write_routes_json` output).
  SHA-256: `b34bfc1adc0975b5a39152daaa749a61e1ab9eeefb7a5ad677987a69b0daf79c`
- `real_planning_route_2step.json` -- route `"33"`, a real 2-step route
  (via a benzyl-ester intermediate), the fixture that demonstrates
  cross-step atom-map consistency. Sliced verbatim from the same real
  `mapped_routes_0.json`.
  SHA-256: `ca733269f56931c785aed39794f243f0a7ead7f2aabce7a0821f2915dbbf1c21`
- `real_planning_export.manifest.json` -- the real `--export_routes`
  manifest, copied verbatim (only local filesystem paths were never part
  of its content to begin with -- it's a small, self-contained envelope).
  SHA-256: `8b8e6ddc8987e81fe65e9d7c79b2b2520498be878fb09befd370073c4e6a710f`
- `real_planning_export.results.json` -- the same two routes as the two
  files above (verified by exact structural equality against the real
  `results.json.gz` entries, not re-derived), re-wrapped in the real
  public-contract shape (`{target_smiles: [RouteNode, ...]}`), uncompressed
  for readability (the real artifact is gzip-compressed; only the
  compression was undone, the JSON content is untouched).
  SHA-256: `b1f9abfc899ab68050855b8f98f5c4f00bb13c537a14913c9814ec68f5dacaaa`

All four files were scanned for local paths, usernames, and tokens before
committing (`grep` for `k_tanabe`, `/Users/`, `/private/tmp`, `token`,
`api_key`, `secret`, `password` -- zero matches).

## What this round did not attempt

- Any preset other than `synplanner-gps`, or the `combined_policy`
  (filtering+ranking) path -- `synplanner-gps`'s two policy heads are
  documented as an unmatched pair for that use.
- `--reconcile-mapping` (the slow, CGR-based cross-step reconciliation
  path) -- this round's finding is that the *default* (fast) path already
  produced consistent numbering, so there was no unresolved question left
  to test on that flag for this target.
- GPU execution, model training, or any multi-target/benchmark-scale run.
- RouteCGR / clustering / quality-scoring modules -- out of scope, as in
  Phase 0.
