import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import coverage_mode_formal_test_gate as gate  # noqa: E402


def make_row(target_id, route_found, selected_stage=None, stage2_invoked=False,
             stage2_timeout=False, normalized_route_sha256=None, run_status="completed",
             route_tree_parseable=None, reranker_failures=0):
    if route_tree_parseable is None:
        route_tree_parseable = True if route_found else None
    return {
        "target_id": target_id,
        "run_status": run_status,
        "route_found": route_found,
        "route_tree_parseable": route_tree_parseable,
        "normalized_route_sha256": normalized_route_sha256,
        "tool_specific": {
            "renkin": {
                "selected_stage": selected_stage,
                "stage2_invoked": stage2_invoked,
                "stage2_timeout": stage2_timeout,
                "reranker_failures": reranker_failures,
            }
        },
    }


class ComputeGateTests(unittest.TestCase):
    def _base_scenario(self):
        """5 targets: 2 solved at Arm A (and Arm C's stage1, matching),
        3 unsolved at Arm A -> escalate to Arm C stage2, all 3 solved there.
        Clean PASS baseline other tests mutate one field of."""
        arm_a = [
            make_row("t1", True, normalized_route_sha256="hash1"),
            make_row("t2", True, normalized_route_sha256="hash2"),
            make_row("t3", False),
            make_row("t4", False),
            make_row("t5", False),
        ]
        arm_c = [
            make_row("t1", True, selected_stage="stage1", stage2_invoked=False, normalized_route_sha256="hash1"),
            make_row("t2", True, selected_stage="stage1", stage2_invoked=False, normalized_route_sha256="hash2"),
            make_row("t3", True, selected_stage="stage2", stage2_invoked=True, normalized_route_sha256="hash3"),
            make_row("t4", True, selected_stage="stage2", stage2_invoked=True, normalized_route_sha256="hash4"),
            make_row("t5", False, selected_stage="stage2", stage2_invoked=True),
        ]
        return arm_a, arm_c

    def test_clean_scenario_computes_expected_deltas(self):
        arm_a, arm_c = self._base_scenario()
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["arm_a_solved"], 2)
        self.assertEqual(v["arm_c_solved"], 4)
        self.assertEqual(v["net_gain"], 2)
        self.assertAlmostEqual(v["coverage_delta_pp"], 40.0)
        self.assertEqual(v["regressions"], [])
        self.assertEqual(v["invalid"], [])
        self.assertEqual(v["stage1_semantic_mismatches"], [])
        self.assertEqual(v["stage2_invoked_when_stage1_solved"], [])
        self.assertEqual(v["stage2_invocation_count"], 3)
        self.assertEqual(v["stage2_timeout_count"], 0)
        # coverage_delta_ge_3pp_and_net_gain_ge_15 fails here (n=5 too small
        # for +15 net) but every OTHER criterion should be clean -- proves
        # criteria are independent, not one bundled boolean.
        self.assertFalse(v["criteria"]["coverage_delta_ge_3pp_and_net_gain_ge_15"])
        self.assertTrue(v["criteria"]["regressions_zero"])
        self.assertTrue(v["criteria"]["invalid_zero"])
        self.assertTrue(v["criteria"]["reranker_failures_zero_both_arms"])
        self.assertTrue(v["criteria"]["arm_a_solved_exact_match_in_arm_c_stage1"])
        self.assertTrue(v["criteria"]["stage2_never_invoked_when_stage1_solved"])
        self.assertTrue(v["criteria"]["stage2_timeout_rate_le_5pct"])
        self.assertFalse(v["overall_pass"])

    def test_regression_detected(self):
        arm_a, arm_c = self._base_scenario()
        # t1 solved at Arm A but NOT at Arm C -- a structural violation that
        # shouldn't be possible given coverage mode's own guarantees, but
        # the gate must catch it if it somehow happened.
        arm_c[0] = make_row("t1", False, selected_stage="stage1", stage2_invoked=False)
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["regressions"], ["t1"])
        self.assertFalse(v["criteria"]["regressions_zero"])

    def test_invalid_route_tree_detected(self):
        arm_a, arm_c = self._base_scenario()
        arm_c[2] = make_row(
            "t3", True, selected_stage="stage2", stage2_invoked=True,
            normalized_route_sha256="hash3", route_tree_parseable=False,
        )
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["invalid"], ["t3"])
        self.assertFalse(v["criteria"]["invalid_zero"])

    def test_reranker_failures_detected_in_either_arm(self):
        arm_a, arm_c = self._base_scenario()
        arm_a[0]["tool_specific"]["renkin"]["reranker_failures"] = 1
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["reranker_failures_arm_a"], 1)
        self.assertFalse(v["criteria"]["reranker_failures_zero_both_arms"])

    def test_stage1_semantic_mismatch_detected(self):
        arm_a, arm_c = self._base_scenario()
        # Arm C says stage1 but the route hash differs from Arm A's --
        # same selected_stage, different actual route: still a mismatch.
        arm_c[0]["normalized_route_sha256"] = "a-different-hash"
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["stage1_semantic_mismatches"], ["t1"])
        self.assertFalse(v["criteria"]["arm_a_solved_exact_match_in_arm_c_stage1"])

    def test_stage1_selected_but_wrong_stage_label_is_also_a_mismatch(self):
        arm_a, arm_c = self._base_scenario()
        arm_c[0]["tool_specific"]["renkin"]["selected_stage"] = "stage2"
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["stage1_semantic_mismatches"], ["t1"])

    def test_stage2_invoked_when_stage1_already_solved_detected(self):
        arm_a, arm_c = self._base_scenario()
        arm_c[0]["tool_specific"]["renkin"]["stage2_invoked"] = True
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["stage2_invoked_when_stage1_solved"], ["t1"])
        self.assertFalse(v["criteria"]["stage2_never_invoked_when_stage1_solved"])

    def test_stage2_timeout_rate_over_threshold_fails(self):
        arm_a, arm_c = self._base_scenario()
        # 3 stage2 invocations (t3/t4/t5); mark 1 as timed out -> 33% > 5%.
        arm_c[2]["tool_specific"]["renkin"]["stage2_timeout"] = True
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["stage2_timeout_count"], 1)
        self.assertAlmostEqual(v["stage2_timeout_rate"], 1 / 3)
        self.assertFalse(v["criteria"]["stage2_timeout_rate_le_5pct"])

    def test_full_pass_scenario(self):
        # n=500-equivalent shrunk to a scale where +15/n>=3pp is checkable:
        # 500 targets, 100 solved at Arm A, 120 at Arm C (net +20, +4pp).
        n = 500
        arm_a = []
        arm_c = []
        for i in range(n):
            tid = f"t{i}"
            if i < 100:
                arm_a.append(make_row(tid, True, normalized_route_sha256=f"h{i}"))
                arm_c.append(
                    make_row(tid, True, selected_stage="stage1", normalized_route_sha256=f"h{i}")
                )
            elif i < 120:
                arm_a.append(make_row(tid, False))
                arm_c.append(
                    make_row(tid, True, selected_stage="stage2", stage2_invoked=True,
                             normalized_route_sha256=f"h{i}")
                )
            else:
                arm_a.append(make_row(tid, False))
                arm_c.append(make_row(tid, False, selected_stage="stage2", stage2_invoked=True))
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["net_gain"], 20)
        self.assertAlmostEqual(v["coverage_delta_pp"], 4.0)
        self.assertTrue(v["overall_pass"])

    def test_missing_target_in_one_arm_reported(self):
        arm_a, arm_c = self._base_scenario()
        del arm_c[-1]
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["cohort_mismatch"]["missing_in_arm_c"], ["t5"])
        self.assertEqual(v["n_common"], 4)

    def test_external_timeout_reported_as_anomaly_not_folded_into_stage2_rate(self):
        arm_a, arm_c = self._base_scenario()
        arm_c[2]["run_status"] = "timeout"
        v = gate.compute_gate(arm_a, arm_c)
        self.assertEqual(v["execution_anomalies"]["arm_c_external_timeout"], ["t3"])
        # t3 is still counted toward stage2_invocation_count/timeout_rate
        # via its own stage2_timeout field (unset here, so not counted as
        # a stage2 timeout) -- the anomaly list is a separate signal, not a
        # replacement for it.
        self.assertEqual(v["stage2_timeout_count"], 0)


if __name__ == "__main__":
    unittest.main()
