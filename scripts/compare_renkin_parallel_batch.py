#!/usr/bin/env python3
"""Parallel throughput arm for the shared-process RENKIN batch adapter.

Each worker owns one renkin-bench process, so stock/template memory is
replicated. This is intentionally separate from the single-process batch arm
and must be evaluated with an explicit worker count and resource budget.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import subprocess
import tempfile
from pathlib import Path


def load_samples(path: str, size: int) -> list[dict]:
    return [json.loads(line) for line in open(path, encoding="utf-8") if line.strip()][:size]


def run_worker(samples: list[dict], args: argparse.Namespace, index: int) -> list[str]:
    with tempfile.NamedTemporaryFile(mode="w", suffix=f"-{index}.jsonl", delete=False) as handle:
        sample_path = handle.name
        for sample in samples:
            handle.write(json.dumps(sample, ensure_ascii=False) + "\n")
    output_path = Path(sample_path).with_suffix(".out.jsonl")
    command = [
        "python3",
        "scripts/compare_renkin_batch.py",
        "--sample-list",
        sample_path,
        "--sample-size",
        str(len(samples)),
        "--renkin-bench",
        args.renkin_bench,
        "--stock",
        args.stock,
        "--templates",
        args.templates,
        "--depth",
        str(args.depth),
        "--beam-width",
        str(args.beam_width),
        "--max-routes",
        str(args.max_routes),
        "--process-timeout-s",
        str(args.process_timeout_s),
        "--output-rows",
        str(output_path),
    ]
    if args.bond_index:
        command.append("--bond-index")
    try:
        subprocess.run(command, check=True, capture_output=True, text=True)
        return output_path.read_text(encoding="utf-8").splitlines()
    finally:
        Path(sample_path).unlink(missing_ok=True)
        output_path.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sample-list", required=True)
    parser.add_argument("--sample-size", type=int, default=100)
    parser.add_argument("--workers", type=int, default=2)
    parser.add_argument("--renkin-bench", default="target/release/renkin-bench")
    parser.add_argument("--stock", required=True)
    parser.add_argument("--templates", required=True)
    parser.add_argument("--depth", type=int, default=5)
    parser.add_argument("--beam-width", type=int, default=100)
    parser.add_argument("--max-routes", type=int, default=1)
    parser.add_argument("--bond-index", action="store_true")
    parser.add_argument("--process-timeout-s", type=int, default=900)
    parser.add_argument("--output-rows", required=True)
    args = parser.parse_args()
    if args.workers <= 0:
        parser.error("--workers must be positive")
    samples = load_samples(args.sample_list, args.sample_size)
    if not samples:
        parser.error("sample list is empty")
    chunks = [samples[i::args.workers] for i in range(args.workers)]
    chunks = [chunk for chunk in chunks if chunk]
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(chunks)) as pool:
        futures = [pool.submit(run_worker, chunk, args, i) for i, chunk in enumerate(chunks)]
        lines = [line for future in futures for line in future.result()]
    rows = [json.loads(line) for line in lines]
    rows.sort(key=lambda row: row["sample_rank"])
    output = Path(args.output_rows)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("".join(json.dumps(row, sort_keys=True) + "\n" for row in rows), encoding="utf-8")
    print(json.dumps({"rows": len(rows), "workers": len(chunks), "route_found": sum(r["route_found"] for r in rows)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
