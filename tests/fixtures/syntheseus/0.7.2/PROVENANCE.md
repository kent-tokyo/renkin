# Syntheseus 0.7.2 fixture provenance

Different provenance story from `tests/fixtures/aizynthfinder/v4.4.1/`
(real tool output, captured once) on purpose: producing a real *searched*
route from Syntheseus requires a backward-reaction model (checkpoint
download, in some cases GPU) -- explicitly out of scope for this Phase 0
feasibility spike (`docs/design/syntheseus-bridge-v0.md`). These fixtures
are **not** hand-typed JSON either. They are the real, byte-for-byte JSON
output of a Python exporter run against genuine
`syntheseus.search.graph.route.SynthesisGraph` objects, constructed
entirely from Syntheseus's own public interface classes (`Molecule`,
`Bag`, `SingleProductReaction`) using the identical object-construction
pattern Syntheseus's own official, public
`AndOrGraph.to_synthesis_graph()` / `MolSetGraph.to_synthesis_graph()`
methods use internally (confirmed by reading
`syntheseus/search/graph/and_or.py`'s and `.../molset.py`'s own source --
both build a `SynthesisGraph` via `new_graph._graph.add_node(...)`/
`.add_edge(...)`, the exact same calls this spike's exporter script
uses). No search algorithm, reaction-prediction model, checkpoint
download, or GPU was used anywhere in producing these fixtures --
confirmed by inspecting `pip show syntheseus`'s own dependency list
(`more_itertools`, `networkx`, `numpy`, `omegaconf`, `rdkit`, `tqdm` --
no `torch`, no model-backend package at all in the base install used
here) and by never importing any of
`syntheseus.reaction_prediction.inference.*`.

## Software

- **syntheseus**: `0.7.2` (PyPI, `pip install syntheseus==0.7.2` into a
  clean Python 3.13 venv). Note: `syntheseus.__version__` does not exist
  on this package -- the version was confirmed via
  `importlib.metadata.version("syntheseus")`, which matches `pip show`.
  At spike time (2026-08-22), `0.8.0` is the latest release on PyPI;
  `0.7.2` was used because it's what the user's own instruction named as
  the target version, not because it's current-latest.
- **rdkit**: `2026.3.5` (pulled in as `syntheseus`'s own dependency, used
  only for SMILES canonicalization inside `Molecule.__post_init__`).
- **Capture date**: 2026-08-22.

## Exporter script (exact, as run)

```python
"""syntheseus-route-v1 exporter -- Phase 0 feasibility spike."""

import json
from syntheseus.search.graph.route import SynthesisGraph


def canon(mol):
    return mol.smiles  # Molecule.__post_init__ already canonicalizes via rdkit


def export_syntheseus_route_v1(graph: SynthesisGraph, source_version: str) -> dict:
    # Deterministic traversal: BFS from the root, each level's children
    # sorted by product SMILES -- do not rely on networkx's own internal
    # node-iteration order as a stability guarantee.
    visited_order = []
    seen = set()
    frontier = [graph.root_node]
    while frontier:
        frontier.sort(key=lambda r: canon(r.product))
        next_frontier = []
        for rxn in frontier:
            if rxn in seen:
                continue
            seen.add(rxn)
            visited_order.append(rxn)
            next_frontier.extend(graph.successors(rxn))
        frontier = next_frontier

    steps = []
    all_molecules_by_smiles = {}

    for rxn in visited_order:
        reactant_smiles = sorted(canon(m) for m in rxn.reactants)
        reaction_metadata = {"reaction_smiles": rxn.reaction_smiles}
        if rxn.identifier is not None:
            reaction_metadata["identifier"] = rxn.identifier
        for key in ("template", "source", "reaction_id"):
            if key in rxn.metadata:
                reaction_metadata[key] = rxn.metadata[key]

        steps.append({
            "product": canon(rxn.product),
            "reactants": reactant_smiles,
            "reaction_metadata": reaction_metadata,
        })
        for m in list(rxn.reactants) + [rxn.product]:
            all_molecules_by_smiles.setdefault(canon(m), m)

    starting_molecules = sorted(canon(m) for m in graph.get_starting_molecules())

    molecule_metadata = {}
    for smi in starting_molecules:
        mol = all_molecules_by_smiles[smi]
        entry = {"is_purchasable": bool(mol.metadata["is_purchasable"]) if "is_purchasable" in mol.metadata else None}
        if "cost" in mol.metadata:
            entry["cost"] = mol.metadata["cost"]
        if "supplier" in mol.metadata:
            entry["supplier"] = mol.metadata["supplier"]
        molecule_metadata[smi] = entry

    return {
        "schema_version": 1,
        "source_tool": "syntheseus",
        "source_version": source_version,
        "target": canon(graph.root_mol),
        "steps": steps,
        "starting_molecules": starting_molecules,
        "molecule_metadata": molecule_metadata,
        "source_metadata": {
            "exporter_schema": "syntheseus-route-v1",
            "note": (
                "Generated from a real syntheseus.search.graph.route.SynthesisGraph "
                "object via the public interface (Molecule/Bag/SingleProductReaction/"
                "SynthesisGraph) -- no model inference, search algorithm, checkpoint, "
                "or GPU was used to construct this route."
            ),
        },
    }
```

This is a **Phase 0 spike script**, not the Phase 1 production exporter
(which will live as a real, packaged, fail-loud-on-unsupported-shapes
module per `docs/design/syntheseus-bridge-v0.md` §Phase 1). Kept here
verbatim for exact reproducibility of these two fixtures, not as a
committed CLI tool.

## Fixture A: `linear_two_leaf_route.json`

A single-step route, both leaves carrying real `Molecule.metadata`
(`is_purchasable`, and one with `cost`/`supplier` too) -- exercises the
"everything present" case.

- **Construction** (exact, as run):
  ```python
  from syntheseus.interface.bag import Bag
  from syntheseus.interface.molecule import Molecule
  from syntheseus.interface.reaction import SingleProductReaction
  from syntheseus.search.graph.route import SynthesisGraph

  ethanol_purchasable = Molecule("CCO", metadata={"is_purchasable": True})
  benzoic_acid = Molecule(
      "OC(=O)c1ccccc1",
      metadata={"is_purchasable": True, "cost": 12.5, "supplier": "TestSupplierCo"},
  )
  ethyl_benzoate = Molecule("CCOC(=O)c1ccccc1")

  step1 = SingleProductReaction(
      product=ethyl_benzoate,
      reactants=Bag([ethanol_purchasable, benzoic_acid]),
      identifier="step1",
      metadata={"template": "esterification_retro", "source": "renkin-syntheseus-spike-fixture"},
  )
  graph = SynthesisGraph(step1)
  graph.assert_validity()  # Syntheseus's own structural validator -- passed
  ```
- **Syntheseus's own reported properties**: `is_tree() == True`,
  `is_minimal() == True`,
  `get_starting_molecules() == {Molecule("CCO"), Molecule("OC(=O)c1ccccc1")}`.
- **Determinism**: the exporter was run twice against the identical
  in-memory object; output was byte-identical both times (confirmed
  before writing the committed file).
- **Output SHA-256**: `1f263e7d4ae32d7c7864d2239bdf7cea4bb305dea86413e0a65a6395bbe083df`

## Fixture B: `convergent_route.json`

A 2-step **convergent** (non-tree) route -- `CO` (methanol) is produced
by one reaction and consumed as a reactant in two different places
(directly by the root reaction, and indirectly as the input to producing
`CS`). Exercises: (1) a genuinely non-tree Syntheseus route structure,
which RENKIN's future Rust normalizer will need to flatten (a real,
non-hypothetical design question for Phase 2 -- see the design doc's own
open-questions section), and (2) a leaf (`CC`, ethane) with **no**
`is_purchasable` metadata at all, i.e. the genuinely-ambiguous case.

- **Construction** (exact, as run) -- mirrors the identical pattern
  Syntheseus's own test suite uses for this same kind of fixture
  (`syntheseus/tests/search/conftest.py`'s `minimal_synthesis_graph`
  fixture):
  ```python
  cc = Molecule("CC")  # no is_purchasable metadata at all
  co_from_cc = SingleProductReaction(product=Molecule("CO"), reactants=Bag([cc]), identifier="co_from_cc")
  cs_from_co = SingleProductReaction(product=Molecule("CS"), reactants=Bag([Molecule("CO")]), identifier="cs_from_co")
  cocs_from_co_cs = SingleProductReaction(
      product=Molecule("COCS"), reactants=Bag([Molecule("CO"), Molecule("CS")]), identifier="cocs_from_co_cs",
  )
  graph = SynthesisGraph(cocs_from_co_cs)
  graph._graph.add_edge(cocs_from_co_cs, co_from_cc)
  graph._graph.add_edge(cocs_from_co_cs, cs_from_co)
  graph._graph.add_edge(cs_from_co, co_from_cc)
  graph.assert_validity()  # Syntheseus's own structural validator -- passed
  ```
- **Syntheseus's own reported properties**: `is_tree() == False` (confirms
  Syntheseus itself recognizes this as non-tree/convergent),
  `is_minimal() == True`, `get_starting_molecules() == {Molecule("CC")}`
  only (correctly excludes `CO`/`CS` -- both are reactants somewhere in
  the graph, but neither is a "starting molecule" by Syntheseus's own
  definition, since each is also some reaction's product elsewhere in
  the same graph).
- **`molecule_metadata["CC"]`**: `{"is_purchasable": null}` -- the
  genuinely-ambiguous case, never guessed or defaulted to `false`.
- **Output SHA-256**: `dc204e3dab9831350a679778ccf0434216911a3002c7b44c9605d798117381c5`
