import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import phase_b2_orchestrator as orch  # noqa: E402


def row(target_id, route_found, run_status="completed", **kw):
    r = {
        "target_id": target_id,
        "target_smiles": target_id,
        "route_found": route_found,
        "run_status": run_status,
    }
    r.update(kw)
    return r


class TestUnsolvedSampleList(unittest.TestCase):
    def test_only_unsolved_included(self):
        stage1 = [row("A", True), row("B", False), row("C", None)]
        sl = orch.unsolved_sample_list(stage1)
        self.assertEqual({r["target_id"] for r in sl}, {"B", "C"})

    def test_duplicate_target_id_fails_loud(self):
        stage1 = [row("A", True), row("A", False)]
        with self.assertRaises(orch.OrchestrationError):
            orch.unsolved_sample_list(stage1)


class TestMergeArm(unittest.TestCase):
    def test_stage1_solved_result_is_never_overwritten(self):
        stage1 = [row("A", True), row("B", False)]
        stage2 = [row("B", True)]
        merged = orch.merge_arm(stage1, stage2)
        by_id = {e["target_id"]: e for e in merged}
        self.assertEqual(by_id["A"]["selected_stage"], "stage1")
        self.assertIs(by_id["A"]["row"], stage1[0])

    def test_stage2_input_set_equals_stage1_unsolved_set_extra_fails_loud(self):
        stage1 = [row("A", True), row("B", False)]
        stage2 = [row("B", False), row("C", True)]  # C was never unsolved in stage1
        with self.assertRaises(orch.OrchestrationError):
            orch.merge_arm(stage1, stage2)

    def test_missing_stage2_result_fails_loud(self):
        stage1 = [row("A", True), row("B", False)]
        stage2 = []  # B has no stage2 result at all
        with self.assertRaises(orch.OrchestrationError):
            orch.merge_arm(stage1, stage2)

    def test_duplicate_target_id_in_stage1_fails_loud(self):
        stage1 = [row("A", True), row("A", False)]
        with self.assertRaises(orch.OrchestrationError):
            orch.merge_arm(stage1, [])

    def test_duplicate_target_id_in_stage2_fails_loud(self):
        stage1 = [row("A", False)]
        stage2 = [row("A", True), row("A", False)]
        with self.assertRaises(orch.OrchestrationError):
            orch.merge_arm(stage1, stage2)

    def test_merge_order_is_stable_and_matches_stage1_order(self):
        stage1 = [row("C", False), row("A", True), row("B", False)]
        stage2 = [row("C", True), row("B", True)]
        merged = orch.merge_arm(stage1, stage2)
        self.assertEqual([e["target_id"] for e in merged], ["C", "A", "B"])

    def test_merge_order_is_stable_across_repeated_calls(self):
        stage1 = [row("C", False), row("A", True), row("B", False)]
        stage2 = [row("C", True), row("B", True)]
        merged1 = orch.merge_arm(stage1, stage2)
        merged2 = orch.merge_arm(stage1, stage2)
        self.assertEqual(
            [e["target_id"] for e in merged1], [e["target_id"] for e in merged2]
        )


class TestManifestConsistency(unittest.TestCase):
    def test_binary_hash_mismatch_fails_loud(self):
        manifests = [{"binary_sha256": "aaa"}, {"binary_sha256": "bbb"}]
        with self.assertRaises(orch.OrchestrationError):
            orch.verify_consistent_binary(manifests)

    def test_binary_hash_match_is_ok(self):
        manifests = [{"binary_sha256": "aaa"}, {"binary_sha256": "aaa"}]
        orch.verify_consistent_binary(manifests)  # must not raise

    def test_missing_binary_hash_field_is_ignored_not_a_false_mismatch(self):
        manifests = [{"binary_sha256": "aaa"}, {}]
        orch.verify_consistent_binary(manifests)  # must not raise


class TestSemanticProjection(unittest.TestCase):
    def test_excludes_nondeterministic_fields(self):
        r = row("A", True, total_elapsed_ms=12345.6, peak_rss_bytes=999)
        entry = {"target_id": "A", "selected_stage": "stage1", "row": r}
        proj = orch.semantic_projection(entry)
        self.assertNotIn("total_elapsed_ms", proj)
        self.assertNotIn("peak_rss_bytes", proj)

    def test_projection_hash_is_independent_of_input_list_order(self):
        stage1 = [row("A", True), row("B", False)]
        stage2 = [row("B", True)]
        merged_forward = orch.merge_arm(stage1, stage2)
        merged_reversed = orch.merge_arm(list(reversed(stage1)), stage2)
        self.assertEqual(
            orch.projection_sha256(merged_forward),
            orch.projection_sha256(merged_reversed),
        )

    def test_projection_hash_changes_when_canonical_route_differs(self):
        stage1_x = [row("A", True, normalized_route_sha256="X")]
        stage1_y = [row("A", True, normalized_route_sha256="Y")]
        h_x = orch.projection_sha256(orch.merge_arm(stage1_x, []))
        h_y = orch.projection_sha256(orch.merge_arm(stage1_y, []))
        self.assertNotEqual(h_x, h_y)

    def test_projection_hash_is_stable_when_only_timing_differs(self):
        stage1_run1 = [row("A", True, normalized_route_sha256="X", total_elapsed_ms=100.0)]
        stage1_run2 = [row("A", True, normalized_route_sha256="X", total_elapsed_ms=99999.0)]
        h1 = orch.projection_sha256(orch.merge_arm(stage1_run1, []))
        h2 = orch.projection_sha256(orch.merge_arm(stage1_run2, []))
        self.assertEqual(h1, h2)

    def test_is_invalid_and_is_timeout_derived_correctly(self):
        r_invalid = row("A", None, run_status="invalid_input")
        r_timeout = row("B", None, run_status="timeout")
        proj_invalid = orch.semantic_projection(
            {"target_id": "A", "selected_stage": "stage1", "row": r_invalid}
        )
        proj_timeout = orch.semantic_projection(
            {"target_id": "B", "selected_stage": "stage1", "row": r_timeout}
        )
        self.assertTrue(proj_invalid["is_invalid"])
        self.assertFalse(proj_invalid["is_timeout"])
        self.assertTrue(proj_timeout["is_timeout"])
        self.assertFalse(proj_timeout["is_invalid"])


if __name__ == "__main__":
    unittest.main()
