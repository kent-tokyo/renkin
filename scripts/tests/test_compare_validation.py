import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_route_graph as rg  # noqa: E402
import compare_validation as v  # noqa: E402

TARGET = "CCOC(=O)c1ccccc1"
ETHANOL = "CCO"
BENZOIC_ACID = "O=C(O)c1ccccc1"


def renkin_single_step_route(target=TARGET, precursors=(ETHANOL, BENZOIC_ACID)):
    return {
        "steps": [{"target": target, "precursors": list(precursors)}],
        "building_blocks": list(precursors),
    }


@unittest.skipUnless(v.HAVE_RDKIT, "requires rdkit (see scripts/requirements-compare-66.txt)")
class TestStockLeafValidation(unittest.TestCase):
    def test_all_leaves_matched(self):
        outcome = rg.normalize_renkin_route(renkin_single_step_route(), TARGET)
        stock = v.build_stock_set([ETHANOL, BENZOIC_ACID, "CCN"])
        result = v.validate_stock_leaves(outcome.graph, stock)
        self.assertTrue(result.all_leaves_in_configured_stock)
        self.assertTrue(all(o.outcome == "matched" for o in result.leaf_breakdown))

    def test_leaf_claimed_stock_but_not_in_configured_stock(self):
        outcome = rg.normalize_renkin_route(renkin_single_step_route(), TARGET)
        # Configured stock (e.g. shared_stock mode) doesn't include benzoic acid.
        stock = v.build_stock_set([ETHANOL])
        result = v.validate_stock_leaves(outcome.graph, stock)
        self.assertFalse(result.all_leaves_in_configured_stock)
        outcomes = {o.canonical_smiles: o.outcome for o in result.leaf_breakdown}
        benzoic_canon = rg.canonicalize(BENZOIC_ACID)
        self.assertEqual(outcomes[benzoic_canon], v.LEAF_CLAIMED_STOCK_NOT_MATCHED)

    def test_unresolved_leaf_is_not_a_validator_defect(self):
        tree = {
            "type": "mol",
            "smiles": TARGET,
            "in_stock": False,
            "children": [
                {
                    "type": "reaction",
                    "children": [
                        {"type": "mol", "smiles": ETHANOL, "in_stock": True, "children": []},
                        {"type": "mol", "smiles": BENZOIC_ACID, "in_stock": False, "children": []},
                    ],
                }
            ],
        }
        outcome = rg.normalize_aizynthfinder_route(tree, TARGET)
        self.assertTrue(outcome.parseable)
        stock = v.build_stock_set([ETHANOL, BENZOIC_ACID])
        result = v.validate_stock_leaves(outcome.graph, stock)
        self.assertFalse(result.all_leaves_in_configured_stock)
        benzoic_canon = rg.canonicalize(BENZOIC_ACID)
        outcomes = {o.canonical_smiles: o.outcome for o in result.leaf_breakdown}
        self.assertEqual(outcomes[benzoic_canon], v.LEAF_UNRESOLVED)


@unittest.skipUnless(v.HAVE_RDKIT, "requires rdkit (see scripts/requirements-compare-66.txt)")
class TestTargetElementAccounting(unittest.TestCase):
    def test_esterification_is_accounted_despite_water_byproduct(self):
        # Product has FEWER heavy atoms than the precursor sum (water is lost) -- must pass.
        outcome = rg.normalize_renkin_route(renkin_single_step_route(), TARGET)
        status, warnings = v.check_target_element_accounting(outcome.graph)
        self.assertEqual(status, "accounted")
        self.assertNotIn(v.UNACCOUNTED_TARGET_ELEMENT, warnings)

    def test_atom_materializing_from_nowhere_is_unaccounted(self):
        # Chlorobenzene "from" bromobenzene: Cl appears in the product with
        # no precursor accounting for it. MW-based checks pass this
        # (bromobenzene is heavier); the per-element check must not.
        route = {
            "steps": [{"target": "Clc1ccccc1", "precursors": ["Brc1ccccc1"]}],
            "building_blocks": ["Brc1ccccc1"],
        }
        outcome = rg.normalize_renkin_route(route, "Clc1ccccc1")
        self.assertTrue(outcome.parseable, outcome.defects)
        status, warnings = v.check_target_element_accounting(outcome.graph)
        self.assertEqual(status, "unaccounted_target_element")
        self.assertIn(v.UNACCOUNTED_TARGET_ELEMENT, warnings)

    def test_leaf_only_route_is_not_evaluable(self):
        route = {"steps": [], "building_blocks": []}
        # A route consisting of the target itself as a stock leaf has no
        # steps at all -- normalize_renkin_route requires >=1 step, so
        # simulate directly via a single-node graph instead.
        leaf = rg.RouteNode(rg.canonicalize(TARGET), is_stock_leaf=True, children=[])
        graph = rg.RouteGraph(root=leaf, step_count_collapsed_edges=0)
        status, warnings = v.check_target_element_accounting(graph)
        self.assertEqual(status, "not_evaluable")
        self.assertEqual(warnings, [])

    def test_caveat_text_explicitly_bans_chemical_correctness_label(self):
        # The phrase legitimately appears -- as an explicit negation/ban, not
        # a claim. Check it's phrased as "must never be read as", not bare.
        self.assertIn('must never be read as "chemically correct"', v.CAVEAT_TEXT)
        self.assertIn("not validated against real reaction feasibility", v.CAVEAT_TEXT)


@unittest.skipUnless(v.HAVE_RDKIT, "requires rdkit (see scripts/requirements-compare-66.txt)")
class TestReactionStepsParseable(unittest.TestCase):
    def test_normal_route_is_parseable(self):
        outcome = rg.normalize_renkin_route(renkin_single_step_route(), TARGET)
        ok, warnings = v.check_reaction_steps_parseable(outcome.graph)
        self.assertTrue(ok)


if __name__ == "__main__":
    unittest.main()
