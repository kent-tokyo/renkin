"""Public-API surface introspection for the syntheseus classes
`renkin.syntheseus_exporter` depends on: `Molecule`, `Bag`,
`SingleProductReaction`/`Reaction`, `SynthesisGraph`, `AndOrGraph`,
`MolSetGraph`. Prints JSON to stdout; diff two runs (each from a clean,
artifact-pinned venv for a different syntheseus version) to get a real,
observed API-compatibility report rather than a guess from release notes.

Only public (non-underscore) names, real constructor signatures via
`inspect`, and real dataclass field lists -- never a class-name-only
compatibility judgment. See
docs/design/syntheseus-0.8-compatibility-spike.md for how this was used
to verify 0.7.2 vs 0.8.0.

Usage (from within a venv that has the target syntheseus version and a
built `renkin` wheel installed):

    python3 scripts/syntheseus_compat_introspect.py > api_<version>.json
"""

import dataclasses
import importlib.metadata
import inspect
import json

from syntheseus.interface.bag import Bag
from syntheseus.interface.molecule import Molecule
from syntheseus.interface.reaction import Reaction, SingleProductReaction
from syntheseus.search.graph.and_or import AndOrGraph
from syntheseus.search.graph.molset import MolSetGraph
from syntheseus.search.graph.route import SynthesisGraph

out = {"syntheseus_version": importlib.metadata.version("syntheseus")}


def public_members(cls):
    return sorted(n for n in dir(cls) if not n.startswith("_"))


def signature_str(callable_obj):
    try:
        return str(inspect.signature(callable_obj))
    except (TypeError, ValueError) as e:
        return f"<unavailable: {e}>"


def dataclass_fields(cls):
    if dataclasses.is_dataclass(cls):
        return [{"name": f.name, "type": str(f.type)} for f in dataclasses.fields(cls)]
    return None


for name, cls in [
    ("Molecule", Molecule),
    ("Bag", Bag),
    ("SingleProductReaction", SingleProductReaction),
    ("Reaction", Reaction),
    ("SynthesisGraph", SynthesisGraph),
    ("AndOrGraph", AndOrGraph),
    ("MolSetGraph", MolSetGraph),
]:
    out[name] = {
        "module": cls.__module__,
        "is_dataclass": dataclasses.is_dataclass(cls),
        "dataclass_fields": dataclass_fields(cls),
        "init_signature": signature_str(cls.__init__),
        "public_members": public_members(cls),
        "mro": [c.__name__ for c in cls.__mro__],
    }

# Real objects, public-API-only construction (mirrors the exporter's own
# construction pattern) -- inspect actual instance state, not just class
# metadata.
ethanol = Molecule("CCO", metadata={"is_purchasable": True})
benzoic_acid = Molecule(
    "OC(=O)c1ccccc1",
    metadata={"is_purchasable": True, "cost": 12.5, "supplier": "TestSupplierCo"},
)
ethyl_benzoate = Molecule("CCOC(=O)c1ccccc1")
step1 = SingleProductReaction(
    product=ethyl_benzoate,
    reactants=Bag([ethanol, benzoic_acid]),
    identifier="step1",
    metadata={"template": "esterification_retro", "source": "compat-spike"},
)
graph = SynthesisGraph(step1)
graph.assert_validity()

out["real_object_instance_check"] = {
    "molecule_public_members_instance": public_members(ethanol),
    "molecule_smiles_attr": ethanol.smiles,
    "molecule_metadata_attr_type": type(ethanol.metadata).__name__,
    "molecule_equality_same_smiles": Molecule("CCO") == Molecule("CCO"),
    "bag_type": type(step1.reactants).__name__,
    "reaction_public_members_instance": public_members(step1),
    "reaction_smiles_attr": step1.reaction_smiles,
    "reaction_identifier_attr": step1.identifier,
    "reaction_metadata_attr": dict(step1.metadata),
    "graph_public_members_instance": public_members(graph),
    "graph_root_node_is_reaction": type(graph.root_node).__name__,
    "graph_root_mol": graph.root_mol.smiles,
    "graph_successors_of_root": [r.product.smiles for r in graph.successors(graph.root_node)],
    "graph_get_starting_molecules": sorted(m.smiles for m in graph.get_starting_molecules()),
    "graph_is_tree": graph.is_tree(),
    "graph_is_minimal": graph.is_minimal(),
}

try:
    out["molecule_hash_stability"] = hash(Molecule("CCO")) == hash(Molecule("CCO"))
except TypeError as e:
    out["molecule_hash_stability"] = f"unhashable: {e}"

print(json.dumps(out, indent=2, default=str, sort_keys=True))
