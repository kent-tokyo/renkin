import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_route_graph as rg  # noqa: E402

TARGET = "CCOC(=O)c1ccccc1"  # ethyl benzoate
ETHANOL = "CCO"
BENZOIC_ACID = "O=C(O)c1ccccc1"


def renkin_single_step_route(target=TARGET, precursors=(ETHANOL, BENZOIC_ACID)):
    return {
        "steps": [
            {
                "rule": "esterification",
                "template_id": "t1",
                "target": target,
                "precursors": list(precursors),
                "step_confidence": 1.0,
            }
        ],
        "depth": 1,
        "score": 1.0,
        "building_blocks": list(precursors),
        "confidence": 1.0,
        "convergency": 1.0,
        "success_probability": 1.0,
        "route_cost": 1.0,
    }


def aizynth_single_step_tree(target=TARGET, precursors=(ETHANOL, BENZOIC_ACID)):
    return {
        "type": "mol",
        "smiles": target,
        "in_stock": False,
        "children": [
            {
                "type": "reaction",
                "children": [
                    {"type": "mol", "smiles": p, "in_stock": True, "children": []}
                    for p in precursors
                ],
            }
        ],
    }


@unittest.skipUnless(rg.HAVE_RDKIT, "requires rdkit (see scripts/requirements-compare-66.txt)")
class TestNormalizeRenkinRoute(unittest.TestCase):
    def test_zero_step_direct_buy_route_is_a_parseable_stock_leaf(self):
        outcome = rg.normalize_renkin_route(
            {"steps": [], "building_blocks": []},
            "CC(=O)O",
        )
        self.assertTrue(outcome.parseable, outcome.defects)
        self.assertEqual(outcome.graph.step_count_collapsed_edges, 0)
        self.assertEqual(rg.count_leaves(outcome.graph.root), 1)
        self.assertTrue(outcome.graph.root.is_stock_leaf)
        self.assertIsNotNone(rg.normalized_route_sha256(outcome.graph))

    def test_single_step_route_parses(self):
        outcome = rg.normalize_renkin_route(renkin_single_step_route(), TARGET)
        self.assertTrue(outcome.parseable, outcome.defects)
        self.assertEqual(outcome.graph.step_count_collapsed_edges, 1)
        leaves = list(rg.iter_leaves(outcome.graph.root))
        self.assertEqual(len(leaves), 2)
        self.assertTrue(all(leaf.is_stock_leaf for leaf in leaves))

    def test_root_mismatch_detected(self):
        outcome = rg.normalize_renkin_route(renkin_single_step_route(), "CCN")
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.ROOT_MISMATCH, outcome.defects)

    def test_cycle_detected(self):
        route = {
            "steps": [
                {"target": "CCO", "precursors": ["CCN"]},
                {"target": "CCN", "precursors": ["CCO"]},
            ],
            "building_blocks": [],
        }
        outcome = rg.normalize_renkin_route(route, "CCO")
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.CYCLE_DETECTED, outcome.defects)

    def test_childless_non_leaf_detected(self):
        route = {
            "steps": [{"target": TARGET, "precursors": []}],
            "building_blocks": [],
        }
        outcome = rg.normalize_renkin_route(route, TARGET)
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.CHILDLESS_NON_LEAF, outcome.defects)

    def test_ambiguous_leaf_status_when_precursor_not_declared_building_block(self):
        route = {
            "steps": [{"target": TARGET, "precursors": [ETHANOL, BENZOIC_ACID]}],
            "building_blocks": [ETHANOL],  # BENZOIC_ACID missing from declared leaves
        }
        outcome = rg.normalize_renkin_route(route, TARGET)
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.AMBIGUOUS_LEAF_STATUS, outcome.defects)

    def test_unparseable_smiles_in_route_detected(self):
        route = renkin_single_step_route(precursors=(ETHANOL, "not_a_smiles((("))
        outcome = rg.normalize_renkin_route(route, TARGET)
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.UNPARSEABLE_SMILES_IN_ROUTE, outcome.defects)

    def test_degenerate_self_referential_step_detected(self):
        route = {
            "steps": [{"target": TARGET, "precursors": [TARGET]}],
            "building_blocks": [],
        }
        outcome = rg.normalize_renkin_route(route, TARGET)
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.DEGENERATE_SELF_REFERENTIAL_STEP, outcome.defects)

    def test_malformed_shape_is_raw_output_not_decodable(self):
        outcome = rg.normalize_renkin_route({"unexpected": "shape"}, TARGET)
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.RAW_OUTPUT_NOT_DECODABLE, outcome.defects)


@unittest.skipUnless(rg.HAVE_RDKIT, "requires rdkit (see scripts/requirements-compare-66.txt)")
class TestNormalizeAizynthfinderRoute(unittest.TestCase):
    def test_single_step_tree_parses(self):
        outcome = rg.normalize_aizynthfinder_route(aizynth_single_step_tree(), TARGET)
        self.assertTrue(outcome.parseable, outcome.defects)
        leaves = list(rg.iter_leaves(outcome.graph.root))
        self.assertEqual(len(leaves), 2)
        self.assertTrue(all(leaf.is_stock_leaf for leaf in leaves))

    def test_unresolved_leaf_recorded_not_a_parse_defect(self):
        tree = aizynth_single_step_tree()
        tree["children"][0]["children"][1]["in_stock"] = False
        outcome = rg.normalize_aizynthfinder_route(tree, TARGET)
        self.assertTrue(outcome.parseable, outcome.defects)
        leaves = list(rg.iter_leaves(outcome.graph.root))
        statuses = sorted(leaf.is_stock_leaf for leaf in leaves)
        self.assertEqual(statuses, [False, True])

    def test_missing_in_stock_key_is_ambiguous_leaf_status(self):
        tree = aizynth_single_step_tree()
        del tree["children"][0]["children"][0]["in_stock"]
        outcome = rg.normalize_aizynthfinder_route(tree, TARGET)
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.AMBIGUOUS_LEAF_STATUS, outcome.defects)

    def test_missing_type_discriminator_is_not_decodable(self):
        tree = {"smiles": TARGET, "in_stock": False, "children": []}
        outcome = rg.normalize_aizynthfinder_route(tree, TARGET)
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.RAW_OUTPUT_NOT_DECODABLE, outcome.defects)

    def test_root_mismatch_detected(self):
        outcome = rg.normalize_aizynthfinder_route(aizynth_single_step_tree(), "CCN")
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.ROOT_MISMATCH, outcome.defects)

    def test_non_dict_raw_output(self):
        outcome = rg.normalize_aizynthfinder_route(["not", "a", "dict"], TARGET)
        self.assertFalse(outcome.parseable)
        self.assertIn(rg.RAW_OUTPUT_NOT_DECODABLE, outcome.defects)


@unittest.skipUnless(rg.HAVE_RDKIT, "requires rdkit (see scripts/requirements-compare-66.txt)")
class TestNormalizedRouteHashCrossToolConsistency(unittest.TestCase):
    def test_same_disconnection_hashes_identically_across_tools(self):
        renkin_outcome = rg.normalize_renkin_route(renkin_single_step_route(), TARGET)
        aizynth_outcome = rg.normalize_aizynthfinder_route(aizynth_single_step_tree(), TARGET)
        self.assertTrue(renkin_outcome.parseable)
        self.assertTrue(aizynth_outcome.parseable)
        renkin_hash = rg.normalized_route_sha256(renkin_outcome.graph)
        aizynth_hash = rg.normalized_route_sha256(aizynth_outcome.graph)
        self.assertEqual(
            renkin_hash,
            aizynth_hash,
            "the same proposed disconnection must hash identically regardless of source tool",
        )

    def test_hash_stable_across_repeated_calls(self):
        outcome = rg.normalize_renkin_route(renkin_single_step_route(), TARGET)
        h1 = rg.normalized_route_sha256(outcome.graph)
        h2 = rg.normalized_route_sha256(outcome.graph)
        self.assertEqual(h1, h2)

    def test_hash_independent_of_input_precursor_order(self):
        route_a = renkin_single_step_route(precursors=(ETHANOL, BENZOIC_ACID))
        route_b = renkin_single_step_route(precursors=(BENZOIC_ACID, ETHANOL))
        outcome_a = rg.normalize_renkin_route(route_a, TARGET)
        outcome_b = rg.normalize_renkin_route(route_b, TARGET)
        self.assertEqual(
            rg.normalized_route_sha256(outcome_a.graph),
            rg.normalized_route_sha256(outcome_b.graph),
        )

    def test_hash_differs_for_different_disconnections(self):
        outcome_a = rg.normalize_renkin_route(renkin_single_step_route(), TARGET)
        other_route = renkin_single_step_route(precursors=(ETHANOL, "CC(=O)O"))
        outcome_b = rg.normalize_renkin_route(other_route, TARGET)
        self.assertNotEqual(
            rg.normalized_route_sha256(outcome_a.graph),
            rg.normalized_route_sha256(outcome_b.graph),
        )


@unittest.skipUnless(rg.HAVE_RDKIT, "requires rdkit (see scripts/requirements-compare-66.txt)")
class TestGraphDepth(unittest.TestCase):
    def test_single_step_route_has_depth_one(self):
        outcome = rg.normalize_renkin_route(renkin_single_step_route(), TARGET)
        self.assertEqual(rg.graph_depth(outcome.graph.root), 1)

    def test_leaf_only_has_depth_zero(self):
        leaf = rg.RouteNode(rg.canonicalize(ETHANOL), is_stock_leaf=True, children=[])
        self.assertEqual(rg.graph_depth(leaf), 0)

    def test_depth_is_longest_path_not_shortest(self):
        # root -> A -> B (depth 2), root -> C (depth 1) -- longest wins.
        leaf_b = rg.RouteNode("B", is_stock_leaf=True, children=[])
        node_a = rg.RouteNode("A", is_stock_leaf=False, children=[leaf_b])
        leaf_c = rg.RouteNode("C", is_stock_leaf=True, children=[])
        root = rg.RouteNode("root", is_stock_leaf=False, children=[node_a, leaf_c])
        self.assertEqual(rg.graph_depth(root), 2)


if __name__ == "__main__":
    unittest.main()
