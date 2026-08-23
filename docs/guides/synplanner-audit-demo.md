---
title: "Audit a Real SynPlanner Route with RENKIN: A 5-Minute Demo"
description: "A copy-pasteable walkthrough of renkin audit-route against real SynPlanner 1.6.0 output, including why forward validation genuinely passes here — unlike AiZynthFinder or Syntheseus."
---

# Audit a Real SynPlanner Route with RENKIN

*SynPlanner has a real native route export. RENKIN audits it directly.*

[SynPlanner](https://github.com/Laboratoire-de-Chemoinformatique/SynPlanner)
ships its own real route-export function
(`synplan.chem.reaction.routes.write_routes_json`). RENKIN Bridge consumes
that export directly — no custom exporter package needed, unlike Syntheseus
— through the exact same tool-neutral pipeline RENKIN's own routes and
AiZynthFinder's routes already go through: structural integrity, stock, and
declared-reaction forward-replay validation, each reported independently,
rolled up into a route-level `pass`/`fail`/`partial` verdict.

This page walks through it end to end against **real** SynPlanner 1.6.0
output — not a hand-authored example — so every command and every line of
output below is something you can reproduce yourself from a checkout of this
repo.

## The fixtures

`tests/fixtures/synplanner/v1.6.0/` has fixtures from two separate real
capture rounds — full provenance (exact package/checkpoint versions,
licenses, construction code, SHA-256 checksums) is in the two sibling
`PROVENANCE.md` files.

- `route_1_two_step.json` / `route_3_full_fields.json` — Phase 0: real
  `chython` reaction objects run through SynPlanner's own real exporter
  (`write_routes_json`), not hand-typed JSON. `route_3_full_fields.json`'s
  chemistry is deliberately a toy substitution (its atom map reuses one
  number across an O→Cl identity change) to exercise every optional schema
  field at once — not a claim of real synthetic feasibility (see
  `PROVENANCE.md`).
- `real_planning_route_1step.json` / `real_planning_route_2step.json` —
  Phase 1 PR1.5: sliced verbatim from a genuine, CPU-only, 167-route MCTS
  search for aspirin, run through SynPlanner's real `synplan planning` CLI
  end to end (real pretrained `synplanner-gps` checkpoints, MIT-licensed).

This walkthrough uses the real-planning fixtures — they're the ones that
demonstrate SynPlanner's standout property below.

## Step 1: audit without a configured stock

```bash
renkin audit-route tests/fixtures/synplanner/v1.6.0/real_planning_route_1step.json \
  --format synplanner \
  --output human
```

```text
1 routes audited — 0 pass, 0 fail, 1 partial
route 1/1: PARTIAL
  - stock: StockNotProvided
```

`stock: StockNotProvided` is the same honest "we didn't check" result every
adapter reports without `--stock` — see the
[AiZynthFinder demo](aizynthfinder-audit-demo.md) for why that's correct,
not weaker. Notice what's *not* here: no `forward: MissingAtomMapping`, no
`ForwardValidationNotEvaluable`. That's not an omission — see Step 3.

## Step 2: audit with a configured stock

```bash
renkin audit-route tests/fixtures/synplanner/v1.6.0/real_planning_route_1step.json \
  --format synplanner \
  --stock /tmp/synplanner_demo_stock.smi \
  --output human
```

where `/tmp/synplanner_demo_stock.smi` contains the route's own two real
precursors:

```text
CC(=O)OC(C)=O acetic_anhydride
O=C(O)c1ccccc1O salicylic_acid
```

```text
1 routes audited — 1 pass, 0 fail, 0 partial
route 1/1: PASS
```

A genuine `PASS` — not `PARTIAL`. This is the one adapter in RENKIN today
where that's possible on a real, unmodified export: the reaction's own
`smiles` field carries a real, usable atom map, so declared-reaction forward
replay actually runs and actually succeeds. Step 3 explains why.

## Step 3: why forward validation genuinely passes here

RENKIN's declared-reaction-replay check needs an atom-mapped SMIRKS to know
which atom in the product came from which atom in the reactants.
AiZynthFinder's route metadata *optionally* carries one; Syntheseus's
`reaction_smiles` is a plain `reactants>>product` string with no mapping at
all on every real Syntheseus route today — both stay `not_evaluable` (see
the [Syntheseus demo](syntheseus-audit-demo.md)'s own Step 3).
SynPlanner is different: its reaction node's `smiles` field is a real,
atom-mapped SMIRKS by construction, and — confirmed against this route's
real, CPU-only MCTS-searched output, not just a hand-built example — that
map is internally valid (no duplicate or orphan atom numbers) and consistent
across every step boundary in the route, even though SynPlanner's own export
code documents its default path as using non-cross-step-reconciled
numbering. `renkin audit-route` doesn't invent this evidence — it's simply
what SynPlanner's own exporter already writes; RENKIN just doesn't discard
it the way it has no choice but to for the other two adapters (see
`docs/design/synplanner-adapter-v1.md` §7 for the full audit trail, and
`scripts/tests/test_synplanner_real_route_fixture.py` /
`src/bridge/synplanner.rs`'s own unit tests for the exact numbers: 317/317
reaction nodes with valid maps, 150/150 cross-step boundaries consistent,
across a real 167-route search).

**This isn't a claim that every SynPlanner route always replays.** It's
scoped to what was actually tested: one target (aspirin), one preset
(`synplanner-gps`), the exporter's default (non-`--reconcile-mapping`)
path. A route whose declared reaction genuinely doesn't reproduce the target
still correctly reports `fail`, not a forced pass — see the next section.

## A route that correctly fails

`route_3_full_fields.json` (Phase 0) is deliberately toy chemistry: its
SMIRKS reuses one atom-map number across an O→Cl identity change, which
isn't valid atom-mapped chemistry (the same map number is supposed to track
the *same* atom, and oxygen isn't chlorine).

```bash
renkin audit-route tests/fixtures/synplanner/v1.6.0/route_3_full_fields.json \
  --format synplanner \
  --output human
```

```text
1 routes audited — 0 pass, 1 fail, 0 partial
route 1/1: FAIL
  - UnaccountedTargetElement
  - ForwardReactionNotReproduced
  - stock: StockNotProvided
```

`UnaccountedTargetElement` catches the missing chlorine source (the product
has a Cl atom no declared precursor supplies), and the real replay engine
correctly refuses to reproduce the target from this transformation. This is
the honest complement to Step 2's `PASS`: RENKIN's forward validation is
running real chemistry against SynPlanner's real declared reactions either
way, not pattern-matching structure alone — a route with real, valid
chemistry passes, and a route without it fails, on the same adapter, same
code path, no special-casing either direction.

## What `--format auto` does here

Every command above passes `--format synplanner` explicitly. Omit it and
`--format auto` (the default) detects the same result on its own, by
looking for a top-level object whose keys all parse as non-negative
integers (route IDs) and whose values are themselves objects with
`"type": "mol"` at their root — checked ahead of RENKIN-native's own
`{"target": ..., "routes": [...]}` shape and AiZynthFinder's batch
`{"schema": ..., "data": ...}` shape, since all are top-level objects and
this is the more specific signal.

```bash
renkin audit-route tests/fixtures/synplanner/v1.6.0/real_planning_route_1step.json \
  --output human
```

```text
1 routes audited — 0 pass, 0 fail, 1 partial
route 1/1: PARTIAL
  - stock: StockNotProvided
```

**Not yet supported**: the separate `--export_routes` "public contract"
wrapper format SynPlanner's CLI can also emit (`manifest.json` +
`results.json.gz`, a target-SMILES-keyed list rather than a route-ID-keyed
object). Only the `{route_id: RouteNode}` shape every fixture above uses is
recognized today — a deliberate, tracked scope boundary, not a silent gap
(see `docs/design/synplanner-adapter-v1.md`).

## Compatibility

| Source | Verified version | Input |
|---|---|---|
| SynPlanner | 1.6.0 | `write_routes_json`'s `{route_id: RouteNode}` export only — not the separate `--export_routes` wrapper format |

See the [AiZynthFinder demo](aizynthfinder-audit-demo.md) and
[Syntheseus demo](syntheseus-audit-demo.md) for those adapters' own
compatibility rows.

**Compatibility-verified and forward-validation-capable are two different
claims — and unlike the other two adapters, both happen to hold here.**
Confirmed for real, CPU-only MCTS-searched output from one target
(aspirin) and one preset (`synplanner-gps`); not yet re-verified against
other presets, `--reconcile-mapping`, or a batch/multi-target search. See
the [Audit Reproducibility and Compatibility Contract](audit-reproducibility-contract.md)
for what "verified" means across every adapter in general.

## Hit a compatibility problem?

If `audit-route --format synplanner` rejects real SynPlanner output, or a
real route gets flagged `not_evaluable`/`fail` in a way that looks wrong,
please
[open an adapter-compatibility issue](https://github.com/kent-tokyo/renkin/issues/new?template=adapter_compatibility.yml) —
it's set up to capture exactly what turns a real report into the next test
fixture.
