#!/usr/bin/env python3
"""Per-target screening runner (Phase 32, `perf/per-target-screening-timeout`).

Replaces the "N targets per subprocess" sharding approach for A/B screening
arms. Each target runs as its own isolated child process with its own
timeout, so one pathological target can never stall or corrupt an entire
batch's worth of results.

Concrete motivation: during Track D's Screening-500 baseline run, one shard
(100 targets, single `renkin-bench` process) took ~95 minutes because a
single target -- a large TIPS-protected macrolide, row 324 of screening_500
-- alone took 2,110,713.6 ms (~35 min). A bounded diagnostic (see
tasks/todo.md Phase 32 hard-tail entry) showed the cost is dominated by a
large FIXED per-node cost (19.3s for a single root node at depth=1/beam=20,
still 5.8s with zero extracted templates -- 28 hand-crafted rules only),
not primarily template-match count. Root cause not yet isolated; this
runner exists so that kind of outlier costs only its own timeout budget,
never an entire batch, and is never silently dropped from the denominator.

Reuses `renkin-bench`'s existing BenchResult/BenchReport JSON schema by
invoking it on a single-line .smi file per target, rather than reimplementing
the stats it already computes (matched_templates, stock_hits,
beam_limit_hit, max_depth_reached, atom_balance_ok, route_validation_status,
retro_cache_hits/misses, etc.).

Usage:
    python3 scripts/per_target_screening_runner.py \\
        --corpus data/corpora/screening_500.json --out-dir /tmp/run1 \\
        --binary target/release/renkin-bench --depth 5 --beam-width 100 \\
        --templates data/templates_extracted_5000.smi \\
        [--scorer data/template_scorer.onnx] [--bond-index] \\
        [--soft-timeout 180] [--hard-timeout 600] \\
        [--only-indices 12,47,324] [--building-blocks data/building_blocks.smi]

Output: one JSON file per target under <out-dir>/target_<index>.json (never
overwritten silently -- reruns must use a fresh --out-dir or --only-indices
to target specific rows), plus a normalize step
(normalize_per_target_results) that folds them into one target-level JSONL
matching aggregate_bench_results.py's expected `results[]` record shape,
with the extra per-target fields appended.
"""
import argparse
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import time

TIME_L_RSS_RE = re.compile(r"(\d+)\s+maximum resident set size")


def sha256_file(path):
    if not path or not os.path.exists(path):
        return None
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def git_commit(repo_dir):
    try:
        out = subprocess.run(
            ["git", "-C", repo_dir, "rev-parse", "HEAD"],
            capture_output=True, text=True, timeout=10, check=True,
        )
        return out.stdout.strip()
    except Exception:
        return None


def chematic_version(repo_dir):
    lock = os.path.join(repo_dir, "Cargo.lock")
    if not os.path.exists(lock):
        return None
    with open(lock) as f:
        lines = f.readlines()
    for i, line in enumerate(lines):
        if line.strip() == 'name = "chematic"':
            for follow in lines[i + 1 : i + 4]:
                if follow.strip().startswith("version"):
                    return follow.split("=", 1)[1].strip().strip('"')
    return None


def canonicalize_batch(repo_dir, smiles_list):
    """One `canonicalize_smiles` process for the whole batch -- this is
    metadata-only pre-processing, not part of any timed search arm, so
    batching it is fine (no per-target isolation needed here)."""
    binary = os.path.join(repo_dir, "target/release/examples/canonicalize_smiles")
    if not os.path.exists(binary):
        return {s: None for s in smiles_list}
    proc = subprocess.run(
        [binary], input="\n".join(smiles_list), capture_output=True, text=True, timeout=300
    )
    out = {}
    for line in proc.stdout.splitlines():
        parts = line.split("\t", 1)
        if len(parts) == 2:
            out[parts[0]] = parts[1] if parts[1] != "PARSE_ERROR" else None
    return out


def run_one_target(args, repo_dir, index, smiles, canonical, target_hash):
    smi_path = os.path.join(args.out_dir, f"_input_{index}.smi")
    with open(smi_path, "w") as f:
        f.write(smiles + "\n")

    cmd = [
        args.binary,
        "--input", smi_path,
        "--depth", str(args.depth),
        "--beam-width", str(args.beam_width),
    ]
    if args.templates:
        cmd += ["--templates", args.templates]
    if args.building_blocks:
        cmd += ["--building-blocks", args.building_blocks]
    if args.scorer:
        cmd += ["--scorer", args.scorer]
    if args.bond_index:
        cmd += ["--bond-index"]
    if args.plausibility:
        cmd += ["--plausibility"]

    # /usr/bin/time -l (BSD/macOS) reports peak RSS on stderr even for a
    # process this script's own timeout later kills, AS LONG AS the wrapper
    # itself is what receives the kill signal and forwards it -- Popen +
    # explicit terminate (not subprocess.run(timeout=)) so we control that.
    wrapped_cmd = ["/usr/bin/time", "-l"] + cmd

    stdout_path = os.path.join(args.out_dir, f"target_{index}.stdout.json")
    stderr_path = os.path.join(args.out_dir, f"target_{index}.stderr.txt")

    start_wall = time.time()
    start_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(start_wall))
    proc = subprocess.Popen(
        wrapped_cmd, stdout=open(stdout_path, "w"), stderr=open(stderr_path, "w"),
        preexec_fn=os.setsid,  # own process group -> can kill the whole /usr/bin/time + child tree
    )

    timed_out = False
    soft_tail_exceeded = False
    signal_name = None
    try:
        proc.wait(timeout=args.hard_timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait()

    end_wall = time.time()
    end_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(end_wall))
    elapsed_s = end_wall - start_wall
    if elapsed_s > args.soft_timeout:
        soft_tail_exceeded = True

    returncode = proc.returncode
    if returncode is not None and returncode < 0:
        signal_name = signal.Signals(-returncode).name

    os.remove(smi_path)

    peak_rss_bytes = None
    stderr_text = ""
    if os.path.exists(stderr_path):
        with open(stderr_path) as f:
            stderr_text = f.read()
        m = TIME_L_RSS_RE.search(stderr_text)
        if m:
            peak_rss_bytes = int(m.group(1))

    bench_result = None
    parse_error = False
    solved = False
    if not timed_out and os.path.exists(stdout_path):
        try:
            with open(stdout_path) as f:
                report = json.load(f)
            results = report.get("results", [])
            if results:
                bench_result = results[0]
                solved = bool(bench_result.get("solved"))
        except (json.JSONDecodeError, OSError):
            parse_error = True

    termination_reason = (
        "timeout" if timed_out
        else "signal" if signal_name
        else "parse_error" if parse_error
        else "ok" if returncode == 0
        else "nonzero_exit"
    )

    record = {
        "corpus_row_index": index,
        "original_smiles": smiles,
        "canonical_smiles": canonical,
        "target_hash": target_hash,
        "start_time": start_iso,
        "end_time": end_iso,
        "elapsed_s": elapsed_s,
        "exit_status": returncode,
        "solved": solved,
        "timeout": timed_out,
        "soft_tail_exceeded": soft_tail_exceeded,
        "signal_termination": signal_name,
        "parse_error": parse_error,
        "termination_reason": termination_reason,
        "peak_rss_bytes": peak_rss_bytes,
        "stdout_path": stdout_path,
        "stderr_path": stderr_path,
        "bench_result": bench_result,
        "config": {
            "depth": args.depth,
            "beam_width": args.beam_width,
            "templates": args.templates,
            "building_blocks": args.building_blocks,
            "scorer": args.scorer,
            "scorer_mode": "per-node" if args.scorer and args.per_node_label else (
                "root-only" if args.scorer else "none"
            ),
            "bond_index": args.bond_index,
            "plausibility": args.plausibility,
            "soft_timeout_s": args.soft_timeout,
            "hard_timeout_s": args.hard_timeout,
        },
        "provenance": {
            "binary": os.path.abspath(args.binary),
            "binary_commit": git_commit(repo_dir),
            "chematic_version": chematic_version(repo_dir),
            "templates_sha256": sha256_file(args.templates),
            "building_blocks_sha256": sha256_file(args.building_blocks),
            "scorer_sha256": sha256_file(args.scorer),
        },
    }
    return record


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--corpus", required=True, help="corpus JSON (e.g. data/corpora/screening_500.json)")
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--binary", required=True, help="path to renkin-bench")
    ap.add_argument("--depth", type=int, default=5)
    ap.add_argument("--beam-width", type=int, default=100)
    ap.add_argument("--templates")
    ap.add_argument("--building-blocks")
    ap.add_argument("--scorer")
    ap.add_argument("--bond-index", action="store_true")
    ap.add_argument("--plausibility", action="store_true")
    ap.add_argument("--per-node-label", action="store_true",
                     help="cosmetic only: label this run's scorer_mode as per-node vs root-only in output")
    ap.add_argument("--soft-timeout", type=float, default=180.0)
    ap.add_argument("--hard-timeout", type=float, default=600.0)
    ap.add_argument("--only-indices", help="comma-separated corpus row indices to (re)run; default = all")
    args = ap.parse_args()

    repo_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    os.makedirs(args.out_dir, exist_ok=True)

    with open(args.corpus) as f:
        corpus = json.load(f)
    targets = corpus["targets"]

    if args.only_indices:
        wanted = {int(x) for x in args.only_indices.split(",")}
        indices = [i for i in range(len(targets)) if i in wanted]
    else:
        indices = list(range(len(targets)))

    canon_map = canonicalize_batch(repo_dir, [targets[i]["smiles"] for i in indices])

    for n, i in enumerate(indices):
        smiles = targets[i]["smiles"]
        canonical = canon_map.get(smiles)
        target_hash = hashlib.sha256((canonical or smiles).encode()).hexdigest()
        out_path = os.path.join(args.out_dir, f"target_{i}.json")
        print(f"[{n + 1}/{len(indices)}] row={i} {smiles[:60]}...", file=sys.stderr, flush=True)
        record = run_one_target(args, repo_dir, i, smiles, canonical, target_hash)
        with open(out_path, "w") as f:
            json.dump(record, f, indent=2)
        tail_flag = " SOFT-TAIL" if record["soft_tail_exceeded"] else ""
        timeout_flag = " TIMEOUT" if record["timeout"] else ""
        print(
            f"    -> solved={record['solved']} elapsed={record['elapsed_s']:.1f}s"
            f"{tail_flag}{timeout_flag}",
            file=sys.stderr, flush=True,
        )


if __name__ == "__main__":
    main()
