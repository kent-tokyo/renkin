#!/usr/bin/env python3
"""Run Syntheseus 0.8 RetroStar for one frozen target and emit one row."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import time
from pathlib import Path

try:
    from .four_tool_record import FourToolRecord
    from .four_tool_resources import apply_resource_limits, enforcement_label, resource_environment
except ImportError:  # direct script execution
    from four_tool_record import FourToolRecord
    from four_tool_resources import apply_resource_limits, enforcement_label, resource_environment


def load_stock(path: str) -> list[str]:
    return [
        line.split()[0]
        for line in Path(path).read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]


def run(args: argparse.Namespace) -> FourToolRecord:
    # Keep the optional dependency out of the normal benchmark tooling import
    # path. The exact public Syntheseus 0.8.0 classes are resolved at run time.
    exporter_path = Path(__file__).parents[1] / "python/renkin/syntheseus_exporter.py"
    exporter_spec = importlib.util.spec_from_file_location("renkin_syntheseus_exporter", exporter_path)
    if exporter_spec is None or exporter_spec.loader is None:
        raise ImportError(f"cannot load Syntheseus exporter: {exporter_path}")
    exporter = importlib.util.module_from_spec(exporter_spec)
    exporter_spec.loader.exec_module(exporter)
    from syntheseus.interface.molecule import Molecule
    from syntheseus.reaction_prediction.inference import LocalRetroModel
    from syntheseus.search.algorithms.best_first.retro_star import RetroStarSearch
    from syntheseus.search.analysis.route_extraction import iter_routes_cost_order
    from syntheseus.search.mol_inventory import SmilesListInventory
    from syntheseus.search.node_evaluation.common import ConstantNodeEvaluator, ReactionModelLogProbCost

    started = time.perf_counter()
    model = LocalRetroModel(model_dir=args.model_dir, device="cpu")
    inventory = SmilesListInventory(load_stock(args.stock))
    search = RetroStarSearch(
        reaction_model=model,
        mol_inventory=inventory,
        limit_iterations=args.limit_iterations,
        limit_reaction_model_calls=args.limit_reaction_model_calls,
        time_limit_s=args.timeout_s,
        value_function=ConstantNodeEvaluator(constant=0.0),
        and_node_cost_fn=ReactionModelLogProbCost(),
    )
    graph, _ = search.run_from_mol(Molecule(args.target_smiles))
    routes = list(iter_routes_cost_order(graph, max_routes=1))
    elapsed_ms = (time.perf_counter() - started) * 1000
    if not routes:
        return FourToolRecord(
            target_id=args.target_id, target_smiles=args.target_smiles,
            sample_rank=args.sample_rank, tool="syntheseus", arm_id=args.arm_id,
            run_status="completed", route_found=False, tool_reported_route_count=0,
            planning_elapsed_ms=elapsed_ms,
            tool_specific={"syntheseus": {"export_schema": "syntheseus-route-v1", "audit_rank": 1,
                                           "resource_enforcement": enforcement_label()}},
        )
    route_graph = graph.to_synthesis_graph(routes[0])
    route = exporter.export_syntheseus_route_v1(route_graph)
    artifact = Path(args.output).with_suffix(".route.json")
    artifact.write_text(exporter.dumps_syntheseus_route_v1(route_graph) + "\n", encoding="utf-8")
    # Common post-hoc audit is intentionally delegated to the same CLI used by
    # the Syntheseus artifact adapter, keeping this runner's native search path
    # independent from audit implementation details.
    from four_tool_syntheseus import audit_route
    audit = audit_route(route, renkin_binary=args.renkin, stock=args.stock,
                        work_dir=Path(args.output).with_suffix(""))
    status = audit["status"]
    return FourToolRecord(
        target_id=args.target_id, target_smiles=args.target_smiles,
        sample_rank=args.sample_rank, tool="syntheseus", arm_id=args.arm_id,
        run_status="completed", route_found=True, tool_reported_route_count=1,
        common_route_parseable=True, strict_route_to_shared_stock=status == "pass",
        common_audit_status=status, planning_elapsed_ms=elapsed_ms,
        raw_output_sha256=hashlib.sha256(artifact.read_bytes()).hexdigest(),
        route_artifact_sha256=hashlib.sha256(
            json.dumps(route, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest(),
        tool_specific={"syntheseus": {"export_schema": "syntheseus-route-v1", "audit_rank": 1,
                                       "resource_enforcement": enforcement_label()}},
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--stock", required=True)
    parser.add_argument("--renkin", required=True)
    parser.add_argument("--target-id", required=True)
    parser.add_argument("--target-smiles", required=True)
    parser.add_argument("--sample-rank", type=int, required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--arm-id", default="syntheseus-0.8.0-localretro-retrostar-shared-stock")
    parser.add_argument("--limit-iterations", type=int, default=30)
    parser.add_argument("--limit-reaction-model-calls", type=int, default=1000)
    parser.add_argument("--timeout-s", type=float, default=120)
    parser.add_argument("--cpus", type=int, default=8)
    parser.add_argument("--memory-bytes", type=int, default=6 * 1024**3)
    args = parser.parse_args()
    import os
    os.environ.update(resource_environment(args.cpus))
    apply_resource_limits(args.memory_bytes, int(args.timeout_s))
    record = run(args)
    Path(args.output).write_text(record.to_json_line() + "\n", encoding="utf-8")
    print(record.to_json_line())
    return 0 if record.run_status == "completed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
