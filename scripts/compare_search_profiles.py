"""Run the RENKIN named search profiles on one frozen cohort.

This is an orchestration layer over ``compare_run.py``. It deliberately does
not implement search or recompute comparison metrics: each profile keeps the
existing per-target rows, aggregate, and manifest contract. The portfolio
manifest only binds those three arms to the same input files and records their
artifact hashes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

import compare_manifest


PROFILE_NAMES = ("fast", "balanced", "deep")
PORTFOLIO_SCHEMA_VERSION = 1


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _repo_path(repo_root: Path, value: str) -> str:
    path = Path(value)
    return str(path if path.is_absolute() else repo_root / path)


def parse_profiles(values: list[str]) -> list[str]:
    profiles: list[str] = []
    for value in values:
        for profile in value.split(","):
            if profile not in PROFILE_NAMES:
                raise ValueError(
                    f"invalid profile {profile!r}; expected one of {', '.join(PROFILE_NAMES)}"
                )
            if profile in profiles:
                raise ValueError(f"duplicate profile {profile!r}")
            profiles.append(profile)
    if not profiles:
        raise ValueError("at least one profile is required")
    return profiles


def build_compare_command(
    *,
    compare_run: Path,
    repo_root: Path,
    sample_list: str,
    sample_size: int,
    comparison_mode: str,
    renkin_binary: str,
    building_blocks: str,
    shared_stock_smi: str,
    templates: str,
    timeout_s: float,
    grace_s: float,
    max_routes: int,
    route_selection: str,
    output_rows: Path,
    output_aggregate: Path,
    output_manifest: Path,
    profile: str,
) -> list[str]:
    return [
        sys.executable,
        str(compare_run),
        "--sample-list",
        sample_list,
        "--sample-size",
        str(sample_size),
        "--tool",
        "renkin",
        "--comparison-mode",
        comparison_mode,
        "--output-rows",
        str(output_rows),
        "--output-aggregate",
        str(output_aggregate),
        "--manifest-path",
        str(output_manifest),
        "--repo-root",
        str(repo_root),
        "--renkin-binary",
        renkin_binary,
        "--building-blocks",
        building_blocks,
        "--shared-stock-smi",
        shared_stock_smi,
        "--templates",
        templates,
        "--timeout-s",
        str(timeout_s),
        "--grace-s",
        str(grace_s),
        "--max-routes",
        str(max_routes),
        "--route-selection",
        route_selection,
        "--search-profile",
        profile,
    ]


def _write_json_atomic(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def run(args: argparse.Namespace) -> int:
    repo_root = Path(args.repo_root).resolve()
    compare_run = repo_root / "scripts" / "compare_run.py"
    output_dir = Path(args.output_dir).resolve()
    profiles = parse_profiles(args.profiles)
    if output_dir.exists():
        raise ValueError(f"output directory already exists; refusing to overwrite: {output_dir}")
    if not compare_run.is_file():
        raise ValueError(f"compare_run.py not found under repo root: {compare_run}")

    commands: dict[str, list[str]] = {}
    for profile in profiles:
        commands[profile] = build_compare_command(
            compare_run=compare_run,
            repo_root=repo_root,
            sample_list=args.sample_list,
            sample_size=args.sample_size,
            comparison_mode=args.comparison_mode,
            renkin_binary=args.renkin_binary,
            building_blocks=args.building_blocks,
            shared_stock_smi=args.shared_stock_smi,
            templates=args.templates,
            timeout_s=args.timeout_s,
            grace_s=args.grace_s,
            max_routes=args.max_routes,
            route_selection=args.route_selection,
            output_rows=output_dir / f"renkin_{profile}.jsonl",
            output_aggregate=output_dir / f"renkin_{profile}_aggregate.json",
            output_manifest=output_dir / f"renkin_{profile}_manifest.json",
            profile=profile,
        )

    if args.dry_run:
        for profile in profiles:
            print(json.dumps({"profile": profile, "argv": commands[profile]}))
        return 0

    output_dir.mkdir(parents=True)
    summaries = []
    for profile in profiles:
        completed = subprocess.run(commands[profile], cwd=repo_root, check=False)
        if completed.returncode != 0:
            raise RuntimeError(f"profile {profile!r} failed with exit code {completed.returncode}")
        aggregate_path = output_dir / f"renkin_{profile}_aggregate.json"
        manifest_path = output_dir / f"renkin_{profile}_manifest.json"
        aggregate = json.loads(aggregate_path.read_text(encoding="utf-8"))
        if aggregate.get("search_profile") != profile:
            raise RuntimeError(f"profile {profile!r} aggregate has inconsistent identity")
        summaries.append(
            {
                "name": profile,
                "configuration_id": aggregate.get("configuration_id"),
                "n_rows": aggregate.get("total_rows_in_file"),
                "rows_sha256": _sha256(output_dir / f"renkin_{profile}.jsonl"),
                "aggregate_sha256": _sha256(aggregate_path),
                "manifest_sha256": _sha256(manifest_path),
            }
        )

    portfolio = {
        "schema_version": PORTFOLIO_SCHEMA_VERSION,
        "kind": "renkin-search-profile-comparison",
        "tool": "renkin",
        "comparison_mode": args.comparison_mode,
        "sample_list": args.sample_list,
        "sample_size": args.sample_size,
        "profiles": summaries,
        "shared_budget": {
            "timeout_s": args.timeout_s,
            "grace_s": args.grace_s,
            "max_routes": args.max_routes,
            "route_selection": args.route_selection,
        },
        "input_sha256": {
            "sample_list": compare_manifest.sha256_file(_repo_path(repo_root, args.sample_list)),
            "templates": compare_manifest.sha256_file(_repo_path(repo_root, args.templates)),
            "building_blocks": compare_manifest.sha256_file(
                _repo_path(
                    repo_root,
                    args.shared_stock_smi
                    if args.comparison_mode == "shared_stock"
                    else args.building_blocks,
                )
            ),
        },
    }
    _write_json_atomic(output_dir / "profile_comparison_manifest.json", portfolio)
    print(json.dumps(portfolio, indent=2, sort_keys=True))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sample-list", default="data/comparison/sample_full_sorted.jsonl")
    parser.add_argument("--sample-size", type=int, default=200)
    parser.add_argument("--profiles", nargs="+", default=list(PROFILE_NAMES))
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--comparison-mode", choices=["native", "shared_stock"], default="native")
    parser.add_argument("--renkin-binary", default="target/release/renkin")
    parser.add_argument("--building-blocks", default="data/building_blocks.smi")
    parser.add_argument("--shared-stock-smi", default="data/comparison/shared_stock/shared_stock.smi")
    parser.add_argument("--templates", default="data/templates_extracted_500.smi")
    parser.add_argument("--timeout-s", type=float, default=150.0)
    parser.add_argument("--grace-s", type=float, default=10.0)
    parser.add_argument("--max-routes", type=int, default=1)
    parser.add_argument("--route-selection", choices=["rank1", "strict_validated", "strict_on_rank1_failure"], default="rank1")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    try:
        return run(args)
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
