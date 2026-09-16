"""Regression tests for explicitly selected AiZynthFinder formal YAML files."""

from __future__ import annotations

import sys
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


SCRIPTS_DIR = Path(__file__).resolve().parents[1]
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import compare_run
import compare_manifest


def renkin_recovery_args(**overrides):
    values = {
        "scorer": None,
        "template_policy_manifest": None,
        "template_policy_artifact": None,
        "comparison_mode": "shared_stock",
        "shared_stock_smi": "stock.smi",
        "building_blocks": "building_blocks.smi",
        "renkin_binary": "renkin",
        "templates": "templates.smi",
        "depth": 5,
        "beam_width": 100,
        "bond_index": False,
        "max_routes": 1,
        "route_selection": "rank1",
        "timeout_s": 31,
        "grace_s": 10,
        "resource_cpus": 8,
        "resource_memory_gib": 6,
        "renkin_container_image": "renkin@sha256:" + "a" * 64,
        "repo_root": ".",
        "ring_context_policy": "disabled",
        "ring_context_sidecar": None,
        "spectator_bond_policy": "off",
        "element_accounting_policy": "off",
        "beam_diversity_policy": "off",
        "beam_diversity_slots": 0,
        "retro_generator_manifest": None,
        "retro_generator_artifact": None,
        "retro_generator_slots": 0,
        "scorer_ordering_blend": 1.0,
        "speed_profile": False,
        "search_profile": None,
        "reranker_model": None,
        "reranker_freq_table": None,
        "downstream_selector": False,
        "search_mode": "recovery",
        "coverage_templates": None,
        "recovery_coverage_tier": [],
        "coverage_timeout_secs": None,
        "coverage_beam_width": None,
        "recovery_depth": 6,
        "recovery_beam_width": 200,
        "recovery_timeout_secs": 30,
        "recovery_stage_policy": "native",
    }
    values.update(overrides)
    return SimpleNamespace(**values)


def test_explicit_aizynthfinder_config_filename_is_preserved_in_run_config():
    args = SimpleNamespace(
        comparison_mode="shared_stock",
        aizynthfinder_config_filename="config_phase55_31s_rank1_shared_stock.yml",
        aizynthfinder_image="example@sha256:" + "a" * 64,
        public_data_dir="/tmp/public-data",
        timeout_s=31,
        grace_s=10,
        resource_cpus=8,
        resource_memory_gib=6,
        aizynthfinder_config_template="/tmp/formal-template.yml",
    )
    provenance = {
        "config_sha256": "b" * 64,
        "resolved_search": {
            "time_limit": "31",
            "iteration_limit": "100",
            "max_transforms": "5",
            "return_first": "false",
        },
        "resolved_post_processing": {"min_routes": "1", "max_routes": "1"},
    }

    with patch("compare_aizynthfinder_adapter.public_data_provenance", return_value=provenance):
        config, configuration_id, resolved = compare_run.aizynth_config_and_id(args)

    assert config.config_filename == "config_phase55_31s_rank1_shared_stock.yml"
    assert config.time_limit_s == 31.0
    assert config.max_routes == 1
    assert configuration_id == "aizynthfinder-shared_stock-cfgbbbbbbbbbbbb-rc8-rm6g"
    assert resolved is provenance


def test_recovery_budget_is_part_of_renkin_configuration_identity():
    _, _, baseline = compare_run.renkin_config_and_id(renkin_recovery_args())
    _, _, changed_beam = compare_run.renkin_config_and_id(
        renkin_recovery_args(recovery_beam_width=300)
    )
    _, _, changed_timeout = compare_run.renkin_config_and_id(
        renkin_recovery_args(recovery_timeout_secs=29)
    )

    assert "-rd6-rb200-rt30-" in baseline
    assert baseline != changed_beam
    assert baseline != changed_timeout


def test_resume_rejects_a_manifest_from_a_different_recovery_budget():
    _, _, baseline = compare_run.renkin_config_and_id(renkin_recovery_args())
    _, _, changed = compare_run.renkin_config_and_id(
        renkin_recovery_args(recovery_timeout_secs=29)
    )
    manifest = {
        "tool": "renkin",
        "comparison_mode": "shared_stock",
        "configuration_id": baseline,
    }

    try:
        compare_manifest.validate_run_identity(
            manifest,
            tool="renkin",
            comparison_mode="shared_stock",
            configuration_id=changed,
        )
    except ValueError as exc:
        assert "configuration_id" in str(exc)
    else:
        raise AssertionError("resume accepted a different recovery budget")
