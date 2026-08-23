import json
import re
import unittest
from collections import Counter
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
FIXTURE_DIR = REPO_ROOT / "tests" / "fixtures" / "synplanner" / "v1.6.0"

# Mirrors src/bridge/forward.rs's has_atom_mapping: a `:` immediately
# followed by an ASCII digit.
MAP_TOKEN_RE = re.compile(r":[0-9]")


def _mapped_atoms(smirks_side):
    return Counter(int(m) for m in re.findall(r":(\d+)\]", smirks_side))


def _iter_reaction_nodes(node):
    if node.get("type") == "mol":
        for child in node.get("children", []):
            yield from _iter_reaction_nodes(child)
    elif node.get("type") == "reaction":
        yield node
        for child in node.get("children", []):
            yield from _iter_reaction_nodes(child)


def _iter_mol_nodes(node):
    if node.get("type") == "mol":
        yield node
        for child in node.get("children", []):
            yield from _iter_mol_nodes(child)
    elif node.get("type") == "reaction":
        for child in node.get("children", []):
            yield from _iter_mol_nodes(child)


class TestRealPlanningRouteAtomMapping(unittest.TestCase):
    """Verifies properties of *real* SynPlanner 1.6.0 MCTS-searched routes
    (aspirin, synplanner-gps preset, CPU-only), not hand-constructed
    fixtures. See PROVENANCE.md for exact capture provenance.

    These properties resolve docs/design/synplanner-adapter-v1.md's §7
    open questions 1 and 2: real planning output's top-level shape and
    whether its reaction smiles retain usable atom maps.
    """

    def setUp(self):
        self.route_1step = json.loads(
            (FIXTURE_DIR / "real_planning_route_1step.json").read_text()
        )
        self.route_2step = json.loads(
            (FIXTURE_DIR / "real_planning_route_2step.json").read_text()
        )

    def test_top_level_shape_is_route_id_keyed_object(self):
        for fixture in (self.route_1step, self.route_2step):
            self.assertIsInstance(fixture, dict)
            (root,) = fixture.values()
            self.assertEqual(root["type"], "mol")

    def test_every_reaction_smiles_has_atom_mapping(self):
        for fixture in (self.route_1step, self.route_2step):
            (root,) = fixture.values()
            reactions = list(_iter_reaction_nodes(root))
            self.assertGreater(len(reactions), 0)
            for rxn in reactions:
                self.assertRegex(rxn["smiles"], MAP_TOKEN_RE)

    def test_atom_maps_are_internally_valid_per_reaction(self):
        """No duplicate map numbers on either side; every product-side map
        number traces back to a reactant-side map number (conservation)."""
        for fixture in (self.route_1step, self.route_2step):
            (root,) = fixture.values()
            for rxn in _iter_reaction_nodes(root):
                lhs, rhs = rxn["smiles"].split(">>")
                lhs_map = _mapped_atoms(lhs)
                rhs_map = _mapped_atoms(rhs)
                dup_lhs = {k: v for k, v in lhs_map.items() if v > 1}
                dup_rhs = {k: v for k, v in rhs_map.items() if v > 1}
                self.assertEqual(dup_lhs, {}, f"duplicate reactant map in {rxn['smiles']!r}")
                self.assertEqual(dup_rhs, {}, f"duplicate product map in {rxn['smiles']!r}")
                orphans = set(rhs_map) - set(lhs_map)
                self.assertEqual(
                    orphans, set(), f"product map(s) {orphans} absent from reactants"
                )

    def test_cross_step_atom_numbering_is_consistent(self):
        """The 2-step fixture's shared intermediate molecule must carry the
        *same* atom-map numbering (byte-identical mapped SMILES) whether it
        appears as the outer step's reactant or the inner step's product --
        even though SynPlanner's default export path documents itself as
        using 'per-step-local' numbering (see route_cgr.py's
        extract_reactions docstring), i.e. NOT guaranteed reconciled.
        This test's fixture is real evidence that, for tree.synthesis_route
        -derived routes, per-step-local numbering happens to remain
        globally consistent in practice."""
        (route_id, root) = next(iter(self.route_2step.items()))
        outer_rxn = root["children"][0]
        outer_lhs = outer_rxn["smiles"].split(">>")[0]
        intermediate_mol = next(
            c for c in outer_rxn["children"] if c.get("children")
        )
        inner_rxn = intermediate_mol["children"][0]
        inner_rhs = inner_rxn["smiles"].split(">>")[1]
        self.assertIn(
            inner_rhs,
            outer_lhs.split("."),
            "inner reaction's mapped product must appear byte-identical "
            "among the outer reaction's mapped reactants",
        )

    def test_in_stock_is_always_a_definite_bool(self):
        for fixture in (self.route_1step, self.route_2step):
            (root,) = fixture.values()
            for mol in _iter_mol_nodes(root):
                self.assertIn("in_stock", mol)
                self.assertIsInstance(mol["in_stock"], bool)

    def test_route_metadata_fields_are_absent_from_real_cli_output(self):
        """The shipped `synplan planning` CLI never passes a route_metadata
        argument to write_routes_json, so rule_id/rule_source/rule_key/
        meta/step_id/tree_node_id -- all opt-in per Phase 0's source
        reading -- never actually appear in standard CLI usage."""
        never_present = {
            "rule_id",
            "rule_source",
            "rule_key",
            "meta",
            "step_id",
            "tree_node_id",
        }
        for fixture in (self.route_1step, self.route_2step):
            (root,) = fixture.values()
            for rxn in _iter_reaction_nodes(root):
                self.assertEqual(never_present & set(rxn.keys()), set())


class TestRealPlanningExportPublicContract(unittest.TestCase):
    """The --export_routes artifact (manifest.json + results.json.gz) is a
    separate, explicitly versioned 'public contract'
    (ROUTE_EXPORT_SCHEMA_VERSION) with its own wrapper shape and an
    unambiguous format-detection signal RENKIN's future adapter should
    prefer over field-heuristic sniffing of the bare RouteNode shape."""

    def setUp(self):
        self.manifest = json.loads(
            (FIXTURE_DIR / "real_planning_export.manifest.json").read_text()
        )
        self.export = json.loads(
            (FIXTURE_DIR / "real_planning_export.results.json").read_text()
        )

    def test_manifest_declares_synplanner_adapter_directive(self):
        self.assertEqual(self.manifest["directives"]["adapter"], "synplanner")
        self.assertIn("schema_version", self.manifest)

    def test_export_is_target_smiles_keyed_list_of_route_nodes(self):
        self.assertIsInstance(self.export, dict)
        (routes,) = self.export.values()
        self.assertIsInstance(routes, list)
        self.assertGreaterEqual(len(routes), 1)
        for route in routes:
            self.assertEqual(route["type"], "mol")


if __name__ == "__main__":
    unittest.main()
