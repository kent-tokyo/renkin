"""Export a Syntheseus ``SynthesisGraph`` to the ``syntheseus-route-v1`` JSON
interchange format consumed by RENKIN's route-audit pipeline.

Optional dependency -- requires ``pip install renkin[syntheseus]``. This
module is never imported by ``renkin/__init__.py``, so plain ``import renkin``
stays free of the ``syntheseus`` dependency; importing this module directly
without it installed raises Python's own ``ImportError``.

Only syntheseus's public interface is used (``Molecule``/``Bag``/
``SingleProductReaction``/``SynthesisGraph`` and their public methods --
``root_node``, ``root_mol``, ``successors()``, ``get_starting_molecules()``,
``assert_validity()``). No leading-underscore attribute is touched, so this
module carries no private-API compatibility risk (see
``docs/design/syntheseus-bridge-v0.md`` sec 7.2 -- a real test suite may
still need ``._graph.add_edge`` to *construct* multi-step graphs for testing,
since syntheseus exposes no public multi-step constructor, but that is a
test-only concern, not something this exporter itself relies on).

Design grounded in ``docs/design/syntheseus-bridge-v0.md`` sec 3: the flat
``steps`` list mirrors RENKIN's own native route shape so the future Rust
normalizer (Phase 2) can reuse ``normalize_renkin_route``'s existing
tree-flattening algorithm. ``molecule_metadata`` carries purchasability only
for leaves (``starting_molecules``) -- a synthesized intermediate has no
meaningful "is this buyable" question, matching the AiZynthFinder adapter's
identical leaf-only convention.
"""

import importlib.metadata
import json

try:
    from syntheseus.search.graph.route import SynthesisGraph
except ImportError as exc:  # pragma: no cover - exercised only without the extra installed
    raise ImportError(
        "renkin.syntheseus_exporter requires the 'syntheseus' package -- "
        "install with: pip install renkin[syntheseus]"
    ) from exc

SCHEMA_VERSION = 1


def _canon(mol) -> str:
    return mol.smiles  # Molecule.__post_init__ already canonicalizes via rdkit


def export_syntheseus_route_v1(graph: SynthesisGraph) -> dict:
    """Export a validated ``SynthesisGraph`` to a ``syntheseus-route-v1`` dict.

    Fail-loud: raises ``TypeError`` for a non-``SynthesisGraph`` argument,
    and propagates whatever ``graph.assert_validity()`` raises for a
    structurally broken graph (syntheseus's own validator, not reinvented
    here) -- never silently exports a malformed or best-effort route.
    """
    if not isinstance(graph, SynthesisGraph):
        raise TypeError(f"expected a syntheseus SynthesisGraph, got {type(graph).__name__}")
    graph.assert_validity()

    # Deterministic traversal: BFS from the root, each level's children
    # sorted by product SMILES -- never rely on networkx's own internal
    # node-iteration order for the byte-stable-output guarantee.
    visited_order = []
    seen = set()
    frontier = [graph.root_node]
    while frontier:
        frontier.sort(key=lambda r: _canon(r.product))
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
        reactant_smiles = sorted(_canon(m) for m in rxn.reactants)
        reaction_metadata = {"reaction_smiles": rxn.reaction_smiles}
        if rxn.identifier is not None:
            reaction_metadata["identifier"] = rxn.identifier
        for key in ("template", "source", "reaction_id"):
            if key in rxn.metadata:
                reaction_metadata[key] = rxn.metadata[key]

        steps.append(
            {
                "product": _canon(rxn.product),
                "reactants": reactant_smiles,
                "reaction_metadata": reaction_metadata,
            }
        )
        for m in list(rxn.reactants) + [rxn.product]:
            all_molecules_by_smiles.setdefault(_canon(m), m)

    starting_molecules = sorted(_canon(m) for m in graph.get_starting_molecules())

    # A leaf present here with is_purchasable: None means the source tool
    # genuinely didn't say -- never guessed, never silently defaulted.
    molecule_metadata = {}
    for smi in starting_molecules:
        mol = all_molecules_by_smiles[smi]
        entry = {
            "is_purchasable": bool(mol.metadata["is_purchasable"])
            if "is_purchasable" in mol.metadata
            else None
        }
        if "cost" in mol.metadata:
            entry["cost"] = mol.metadata["cost"]
        if "supplier" in mol.metadata:
            entry["supplier"] = mol.metadata["supplier"]
        molecule_metadata[smi] = entry

    return {
        "schema_version": SCHEMA_VERSION,
        "source_tool": "syntheseus",
        "source_version": importlib.metadata.version("syntheseus"),
        "target": _canon(graph.root_mol),
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


def dumps_syntheseus_route_v1(graph: SynthesisGraph) -> str:
    """Byte-stable JSON string for ``export_syntheseus_route_v1(graph)``."""
    return json.dumps(export_syntheseus_route_v1(graph), indent=2, sort_keys=True, ensure_ascii=False)
