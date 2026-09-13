import importlib.util
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "phase55_failure_atlas.py"
SPEC = importlib.util.spec_from_file_location("phase55_failure_atlas", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def row(*, route_found=False, termination="completed", depth=False, beam=False, validator=None):
    return {
        "target_id": "t",
        "run_status": "completed",
        "route_found": route_found,
        "validator_confirmed_route_found": validator,
        "tool_specific": {
            "renkin": {
                "recovery": {
                    "attempts": [
                        {
                            "termination": termination,
                            "max_depth_reached": depth,
                            "beam_limit_hit": beam,
                            "direct_generator_proposals": 0,
                        }
                    ]
                }
            }
        },
    }


def test_classification_preserves_ambiguous_signals():
    primary, signals = MODULE.classify(row(depth=True, beam=True))
    assert primary == "depth_and_beam_limit"
    assert signals == ["max_depth_reached", "beam_limit_hit"]


def test_deadline_takes_precedence_over_search_signals():
    primary, signals = MODULE.classify(row(termination="deadline_exceeded", depth=True))
    assert primary == "budget_exhausted"
    assert "deadline_exceeded" in signals


def test_validator_null_is_not_a_success():
    primary, _ = MODULE.classify(row(route_found=True, validator=None))
    assert primary == "validator_unknown"


def test_build_atlas_keeps_competitor_relation_separate():
    atlas = MODULE.build_atlas(
        [row(route_found=False)],
        {"t": {"target_id": "t", "renkin_route_found": False, "aizynthfinder_route_found": True}},
    )
    assert atlas["competitor_relation_counts"] == {"aizynthfinder_only": 1}
    assert atlas["records"][0]["diagnostic_scope"] == "observed_row_signals_only"


def test_attempt_ledger_preserves_observed_stage_fields_only():
    atlas = MODULE.build_atlas(
        [row(depth=True, beam=True)],
        {"t": {"target_id": "t", "renkin_route_found": False, "aizynthfinder_route_found": False}},
    )
    ledger = atlas["records"][0]["attempt_ledger"]
    assert len(ledger) == 1
    assert ledger[0]["termination"] == "completed"
    assert ledger[0]["max_depth_reached"] is True
    assert ledger[0]["beam_limit_hit"] is True
    assert "route_edges" not in ledger[0]
