---
title: "Audit a Syntheseus Route with RENKIN: A 5-Minute Demo"
description: "A copy-pasteable walkthrough of renkin audit-route against real syntheseus-route-v1 fixtures, including why forward validation stays not_evaluable today."
---

# Audit a Syntheseus Route with RENKIN

*Syntheseus has no native route export. RENKIN owns the converter.*

[Syntheseus](https://github.com/microsoft/syntheseus) is a retrosynthesis
search library, not a route-export format — it has no `to_json()` on its
own `SynthesisGraph` type. RENKIN Bridge closes that gap with its own
`syntheseus-route-v1` interchange schema: an optional Python module,
[`renkin.syntheseus_exporter`](../api/python.md) (`pip install
renkin[syntheseus]`), walks a real `SynthesisGraph` object via its public
interface and produces JSON that `renkin audit-route --format syntheseus`
consumes through the exact same tool-neutral pipeline RENKIN's own routes
and AiZynthFinder's routes already go through.

## The fixtures

`tests/fixtures/syntheseus/0.7.2/` has two real, exporter-produced JSON
files — not hand-typed, not from a live model search (that needs a
checkpoint/GPU, out of scope for this adapter's own construction and
testing). Both are genuine `syntheseus.search.graph.route.SynthesisGraph`
objects built from Syntheseus's own public interface classes
(`Molecule`/`Bag`/`SingleProductReaction`), validated by Syntheseus's own
`assert_validity()`, then exported. Full provenance — exact construction
code, package versions, checksums — is in the sibling
[`PROVENANCE.md`](https://github.com/kent-tokyo/renkin/blob/master/tests/fixtures/syntheseus/0.7.2/PROVENANCE.md).

- `linear_two_leaf_route.json` — a single-step route, both leaves carrying
  full purchasability metadata.
- `convergent_route.json` — a deliberately non-tree route (a molecule
  produced by one step and consumed by two different downstream steps),
  with one leaf carrying no purchasability claim at all.

## Step 1: audit without a configured stock

```bash
renkin audit-route tests/fixtures/syntheseus/0.7.2/linear_two_leaf_route.json \
  --format syntheseus \
  --output human
```

```text
1 routes audited — 0 pass, 0 fail, 1 partial
route 1/1: PARTIAL
  - ForwardValidationNotEvaluable
  - stock: StockNotProvided
  - forward: MissingAtomMapping
```

`stock: StockNotProvided` is the same honest "we didn't check" result every
adapter reports without `--stock` — see the
[AiZynthFinder demo](aizynthfinder-audit-demo.md) for why that's correct,
not weaker. `forward: MissingAtomMapping` is specific to this adapter, and
worth understanding rather than dismissing as noise (see Step 3).

## Step 2: audit with a configured stock

```bash
renkin audit-route tests/fixtures/syntheseus/0.7.2/linear_two_leaf_route.json \
  --format syntheseus \
  --stock data/building_blocks.smi \
  --output human
```

```text
1 routes audited — 0 pass, 0 fail, 1 partial
route 1/1: PARTIAL
  - ForwardValidationNotEvaluable
  - forward: MissingAtomMapping
```

`stock: StockNotProvided` is gone — both leaves (ethanol, benzoic acid) are
in `data/building_blocks.smi`, so stock validation now passes outright. The
route still comes back `PARTIAL`, not `PASS`, purely because of forward
validation. That's Step 3.

## Step 3: why forward validation stays `not_evaluable`

RENKIN's declared-reaction-replay check needs an atom-mapped SMIRKS to know
which atom in the product came from which atom in the reactants.
AiZynthFinder's route metadata *optionally* carries one
(`mapped_reaction_smiles`); Syntheseus's `reaction_smiles` is a computed
property — a plain `reactants>>product` string, generated from canonical
SMILES, with no atom mapping at all, on every real Syntheseus route today.
`renkin audit-route` doesn't invent mapping that isn't there: it reports
`MissingAtomMapping`, the same honest `not_evaluable` result you'd get from
an AiZynthFinder route whose metadata omits `mapped_reaction_smiles` too
(see `missing_reaction_evidence_is_not_evaluable_never_silently_resolved_on_either_tool`
in `tests/cross_tool_audit.rs`). A future exporter enhancement could add
atom mapping if a real need for it shows up — nothing here rules it out —
but nothing today fabricates it to force a `PASS`.

## The convergent (non-tree) fixture

```bash
renkin audit-route tests/fixtures/syntheseus/0.7.2/convergent_route.json \
  --format syntheseus \
  --output human
```

```text
1 routes audited — 0 pass, 1 fail, 0 partial
route 1/1: FAIL
  - AmbiguousLeafStatus
  - AmbiguousLeafStatus
```

This fixture's one true leaf (`CC`, ethane) genuinely carries no
purchasability claim — `AmbiguousLeafStatus` is the correct, honest result,
never silently guessed at. It appears twice, not once: the same molecule is
reachable via two different reaction paths in this convergent route, and
RENKIN's route representation has no way to express a shared node with two
parents, so it's expanded independently under each parent — the same
duplication-on-flatten behavior RENKIN-native routes already have for this
case, not something new invented for Syntheseus (see
`docs/design/syntheseus-bridge-v0.md` §7.1).

## What `--format auto` does here

Every command above passes `--format syntheseus` explicitly. Omit it and
`--format auto` (the default) detects the same result on its own, by
looking for a top-level object with `"source_tool": "syntheseus"` — checked
ahead of RENKIN-native's own `{"target": ..., "routes": [...]}` shape,
since both are top-level objects and this is the more specific signal.

## Compatibility

| Source | Verified version | Input |
|---|---|---|
| Syntheseus | `0.7.2` (via `renkin.syntheseus_exporter`'s `syntheseus-route-v1` JSON) | single-route document only — no batch format exists for this adapter |

"Verified against" is deliberate phrasing, not "supported" — see the
[Audit Reproducibility and Compatibility Contract](audit-reproducibility-contract.md)
for the full rule set every adapter follows, and the
[AiZynthFinder demo](aizynthfinder-audit-demo.md) for that adapter's own
compatibility row. `0.7.2` was the named target version at the time this
adapter was built; PyPI's own latest at that time was already `0.8.0` — the
exporter records the real installed version via
`importlib.metadata.version("syntheseus")` on every export, so this isn't a
silent claim either way.

## Hit a compatibility problem?

If `audit-route --format syntheseus` rejects real
`renkin.syntheseus_exporter` output, or a real route gets flagged
`not_evaluable`/`fail` in a way that looks wrong, please
[open an adapter-compatibility issue](https://github.com/kent-tokyo/renkin/issues/new?template=adapter_compatibility.yml) —
it's set up to capture exactly what turns a real report into the next test
fixture.
