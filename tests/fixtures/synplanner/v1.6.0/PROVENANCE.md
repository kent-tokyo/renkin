# SynPlanner v1.6.0 fixture provenance

Unlike the AiZynthFinder fixtures in this repo (real `aizynthcli` search output,
captured once), these are **not** the output of a real MCTS planning search.
SynPlanner's route export (`SynPlanner==1.6.0`) requires a trained
policy/value model checkpoint (`.ckpt`, PyTorch Lightning) to run an actual
search, and downloading/running one was explicitly out of scope for this
round ("model download が必要な重い planning" was excluded).

Instead, following the same precedent RENKIN already uses for Syntheseus
(`tests/fixtures/syntheseus/*/PROVENANCE.md`): these fixtures are
**genuine reaction chemistry, constructed via SynPlanner's real dependency
`chython`'s public reaction-SMILES parser, run through SynPlanner's own,
unmodified, real export function** (`synplan.chem.reaction.routes.write_routes_json`,
which internally calls `build_route_trees` → `_make_json_v1`). No route
JSON was ever hand-typed. Every field in the committed files is exactly
what SynPlanner's real code produced from real input objects.

## Software

- **SynPlanner**: `1.6.0` (PyPI, `pip install SynPlanner==1.6.0` into a
  clean, disposable `python3.13` venv). Resolved dependency versions
  actually installed (not just the declared `>=` constraints):
  `torch==2.13.0`, `torch-geometric==2.8.0.post1`,
  `pytorch-lightning==2.6.5`, `rdkit==2026.3.5`, `chython-synplan==1.101`
  (pinned exact by SynPlanner itself), `chytorch-synplan==1.70`,
  `chytorch-rxnmap-synplan==1.7`, `huggingface-hub==1.28.0`.
- **Capture date**: 2026-08-23 (UTC).
- **Venv lifecycle**: created solely to generate these fixtures, removed
  immediately afterward (disk headroom was a live constraint: ~14GB free
  before this install, ~1.6GB consumed by the venv). Not a persistent
  RENKIN dev dependency.

## Real code path exercised (confirmed by reading the actual v1.6.0 source
after installing, not guessed from the PyPI page)

`write_routes_json(routes_dict, file_path, route_metadata=None, *, strict=False)`
in `synplan/chem/reaction/routes/io/json.py` delegates to
`build_route_trees(...)`, which calls the private `_make_json_v1(...)`.

**The real input contract is *not* a `RouteNode` dict** — it's
`routes_dict: dict[int, dict[int, chython.ReactionContainer]]`, mapping
`route_id -> {step_id -> Reaction}`. `_make_json_v1` walks this backward
from `max(steps)` (the reaction that produces the final target), recursing
into each reactant that an *earlier* step's product structurally matches
(via `chython`'s own molecule equality / canonical-SMILES lookup — atom-map
numbers are stripped before this comparison, confirmed empirically: two
occurrences of the same molecule with *different* atom-map numbers still
link correctly). A reactant with no earlier producing step in the given
`routes_dict` is treated as a stock leaf.

**Genuinely new finding, not obtainable from reading the TypedDict
declaration alone**: through this real code path, `in_stock` is **always**
a definite boolean (`true` for every stock leaf, `false` for every
reaction-produced molecule) — there is no code path in `_make_json_v1`
that leaves it absent or `None`. It is also **not independently
re-verified against any stock database** by this function: "no producing
reaction in the given `routes_dict`" is the entire criterion. This matters
for the adapter design doc's stock-handling section (§3 of
`docs/design/synplanner-adapter-v1.md`) — RENKIN's existing
"ambiguous/`None` leaf" finding (`AmbiguousLeafStatus`, used for AiZynthFinder's
`in_stock: None` and Syntheseus's `is_purchasable: None`) may simply never
trigger for SynPlanner-sourced routes, at least not via this export path.

`rule_id`/`rule_source`/`rule_key`/`step_id`/`tree_node_id` are **not**
derived automatically from the `Reaction` objects — they only appear if the
caller passes a `route_metadata: dict[route_id, dict[step_id, dict]]`
argument, whose contents get `dict.update()`-merged directly onto the
reaction node (siblings of `type`/`smiles`/`children`, not nested under
`meta`). `meta` (a *separate* field) comes from `chython.ReactionContainer.meta`
itself, a real, independently-settable dict on the reaction object.

**Top-level shape, confirmed by running the real function, not assumed
from the schema doc**: `write_routes_json` writes `{route_id: RouteNode}` —
an **object keyed by route-id string** — not a bare `RouteNode` and not a
top-level array. This is a distinguishing structural signal worth using
for `--format auto` detection (see the adapter design doc).

## Fixture A: `route_1_two_step.json`

A genuine 2-step chain, exercising nested `reaction`/`mol` recursion, one
stock leaf, and per-step rule provenance on both reaction nodes.

- **Construction** (exact, verbatim):
  ```python
  import chython
  from synplan.chem.reaction.routes import write_routes_json

  step1 = chython.smiles("[CH3:1][CH2:2][Br:3]>>[CH3:1][CH2:2][OH:3]")  # bromoethane -> ethanol
  step2 = chython.smiles("[CH3:1][CH2:2][OH:3]>>[CH3:1][CH2:2][Cl:3]")  # ethanol -> chloroethane (target)

  routes_dict = {1: {1: step1, 2: step2}}
  route_metadata = {
      1: {
          1: {"rule_id": 4821, "rule_source": "extracted", "rule_key": "chy:4821"},
          2: {"rule_id": 17, "rule_source": "handcrafted", "rule_key": "chy:0017"},
      }
  }
  write_routes_json(routes_dict, "route_1_two_step.json", route_metadata=route_metadata, strict=True)
  ```
- Both reaction SMILES are individually valid chython reaction objects but
  the pair is **not mass-balanced as written** (no explicit leaving-group
  byproducts) — the same simplification RENKIN's own existing cross-tool
  test fixtures already use for structural-only test chemistry (e.g. the
  `CO>>C.O` methanol case in `tests/cross_tool_audit.rs`). This exercises
  the adapter's structural/schema handling, not a claim about real
  synthetic feasibility.
- `strict=True` succeeded with **zero diagnostics** — both reactions
  linked cleanly (`step1`'s product "CCO" structurally matched `step2`'s
  reactant).
- **Output SHA-256**: `4f13103e8efe1d67e415592fd92c3a988a37b77ff98bca656fd6f4551af2c827`
- **Determinism**: re-running the exact same construction code a second
  time in the same session produced a byte-identical file (verified before
  committing).

## Fixture B: `route_3_full_fields.json`

A single-step route, purpose-built to exercise every optional `RouteNode`
field in one place: `meta` (from `chython.ReactionContainer.meta`, real and
independently settable), `step_id`, `tree_node_id`, `rule_id`,
`rule_source`, `rule_key` (all from `route_metadata`).

- **Construction** (exact, verbatim):
  ```python
  import chython
  from synplan.chem.reaction.routes import write_routes_json

  step = chython.smiles("[CH3:1][CH2:2][OH:3]>>[CH3:1][CH2:2][Cl:3]")
  step.meta["confidence"] = 0.83
  step.meta["template_library_version"] = "v3"

  routes_dict = {7: {1: step}}
  route_metadata = {7: {1: {"step_id": 1, "tree_node_id": 42, "rule_id": 17,
                            "rule_source": "handcrafted", "rule_key": "chy:0017"}}}
  write_routes_json(routes_dict, "route_3_full_fields.json", route_metadata=route_metadata, strict=True)
  ```
- `strict=True` succeeded with zero diagnostics.
- **Output SHA-256**: `9c982493f69be86d349efd96c32a255b1af9eddc8d975ea821f11622ea1562dd`
- Route ID `7` is deliberately non-sequential/non-`1` here, confirming the
  top-level object's key really is the caller-supplied route ID, not a
  fixed/renumbered index.

## Malformed-route behavior (confirmed by running the real code, not documented as a separate fixture file)

Reproduced directly rather than committed as a JSON artifact, since RENKIN's
own convention for malformed-input testing (see the AiZynthFinder adapter's
`structurally_corrupt_route_fails_loud_not_silently` unit test) is a
hand-built Rust test struct, not a captured malformed fixture:

```python
broken = chython.smiles("[CH3:1][CH2:2][OH:3]>>")   # zero products
write_routes_json({1: {1: broken}}, "x.json", strict=True)
# -> raises synplan.chem.reaction.routes.contracts.RouteExportError
#    with diagnostics = (RouteDiagnostic(route_id=1, stage="route_tree_export",
#      message="Route could not be represented as a valid v1 route tree",
#      exception_type=None),)

write_routes_json({1: {1: broken}}, "x.json", strict=False)
# -> returns cleanly, result.routes == {} (route silently dropped),
#    result.diagnostics carries the same RouteDiagnostic for inspection.
```

Confirms the earlier source-reading finding empirically: malformed routes
are dropped with an explicit, inspectable diagnostic under `strict=False`,
or raised as `RouteExportError` under `strict=True` — never emitted as a
JSON `null` child.

## What this round did not attempt

- No real MCTS-searched route (would require a downloaded trained
  policy/value checkpoint — explicitly out of scope).
- No fixture demonstrating `in_stock: null`/absent — per the finding above,
  this real export code path appears structurally incapable of producing
  one; not faked to satisfy a checklist.
- RouteCGR / clustering / quality-scoring output — not exercised; those
  modules operate on already-exported routes/route sets, not the base
  export path this round focused on.
