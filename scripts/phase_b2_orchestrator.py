"""Phase B.2 (Progressive Template Escalation) orchestration.

Benchmark/orchestration layer only, per Phase B.2's implementation
boundary (ROADMAP.md) -- no new RENKIN core API. Builds the Stage-2
unsolved sample-list from Stage-1 results (reusing compare_run.py's own
sample-list JSONL schema), merges Stage 1 + Stage 2 into one per-target
result, and computes a deterministic semantic projection (route
found/shape/validator outcome, excluding wall-clock/timestamps/temp
paths) for cross-run determinism checks.

Every invariant below fails loud (raises OrchestrationError) rather than
silently proceeding on inconsistent input -- see
data/phase_b1_frontier/findings.md's "Phase B.2" section and
scripts/tests/test_phase_b2_orchestrator.py for what each one guards
against.
"""

import hashlib
import json


class OrchestrationError(Exception):
    """Raised on any invariant violation. Never silently degrade."""


def load_rows(path):
    rows = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def unsolved_sample_list(stage1_rows):
    """Stage-2 input sample-list (compare_run.py's own schema) built from
    Stage 1's unsolved targets, in Stage 1's own row order."""
    seen = set()
    out = []
    for r in stage1_rows:
        tid = r["target_id"]
        if tid in seen:
            raise OrchestrationError(f"duplicate target_id in stage1_rows: {tid!r}")
        seen.add(tid)
        if r.get("route_found") is not True:
            out.append(
                {
                    "canonical_smiles": r["target_smiles"],
                    "sample_key": None,
                    "sample_rank": len(out),
                    "source_line_number": None,
                    "target_id": tid,
                }
            )
    return out


def merge_arm(stage1_rows, stage2_rows):
    """Merge Stage 1 + Stage 2 into one per-target result, preserving Stage
    1's row order. A Stage-1-solved result is never replaced by anything
    from Stage 2 -- Stage 2 is only ever consulted for targets Stage 1 left
    unsolved, and this function fails loudly if that invariant doesn't
    hold in the data it's given, rather than trusting the caller ran the
    two stages correctly.
    """
    stage1_by_id = {}
    for r in stage1_rows:
        tid = r["target_id"]
        if tid in stage1_by_id:
            raise OrchestrationError(f"duplicate target_id in stage1_rows: {tid!r}")
        stage1_by_id[tid] = r

    stage2_by_id = {}
    for r in stage2_rows:
        tid = r["target_id"]
        if tid in stage2_by_id:
            raise OrchestrationError(f"duplicate target_id in stage2_rows: {tid!r}")
        stage2_by_id[tid] = r

    stage1_unsolved_ids = {
        tid for tid, r in stage1_by_id.items() if r.get("route_found") is not True
    }
    extra = set(stage2_by_id) - stage1_unsolved_ids
    if extra:
        raise OrchestrationError(
            f"stage2_rows contains {len(extra)} target(s) not in stage1's "
            f"unsolved set (e.g. {sorted(extra)[:5]})"
        )
    missing = stage1_unsolved_ids - set(stage2_by_id)
    if missing:
        raise OrchestrationError(
            f"stage2_rows missing {len(missing)} target(s) from stage1's "
            f"unsolved set (e.g. {sorted(missing)[:5]})"
        )

    merged = []
    for r1 in stage1_rows:  # stable order = stage1's own order, always
        tid = r1["target_id"]
        if r1.get("route_found") is True:
            merged.append({"target_id": tid, "selected_stage": "stage1", "row": r1})
        else:
            merged.append(
                {"target_id": tid, "selected_stage": "stage2", "row": stage2_by_id[tid]}
            )
    return merged


def verify_consistent_binary(manifests):
    """All given manifest dicts must share the same binary_sha256 -- fails
    loud on any mismatch (e.g. a rebuild happened mid-run, as occurred
    earlier in this program with the 500@600s arm re-run)."""
    hashes = {m["binary_sha256"] for m in manifests if m.get("binary_sha256")}
    if len(hashes) > 1:
        raise OrchestrationError(f"binary_sha256 mismatch across manifests: {sorted(hashes)}")


def semantic_projection(merged_entry):
    """Deterministic fields only -- excludes elapsed time, timestamps,
    temp paths, peak_rss, etc."""
    row = merged_entry["row"]
    return {
        "target_id": merged_entry["target_id"],
        "selected_stage": merged_entry["selected_stage"],
        "route_found": row.get("route_found"),
        "canonical_route_sha256": row.get("normalized_route_sha256"),
        "all_leaves_in_configured_stock": row.get("all_leaves_in_configured_stock"),
        "route_tree_parseable": row.get("route_tree_parseable"),
        "reaction_steps_parseable": row.get("reaction_steps_parseable"),
        "target_element_accounting_status": row.get("target_element_accounting_status"),
        "common_validation_warnings": sorted(row.get("common_validation_warnings") or []),
        "run_status": row.get("run_status"),
        "is_invalid": row.get("run_status") == "invalid_input",
        "is_timeout": row.get("run_status") == "timeout",
        "reranker_failures": (row.get("tool_specific") or {}).get("renkin", {}).get("reranker_failures"),
    }


def projection_sha256(merged):
    """SHA-256 over the semantic projections of every entry, sorted by
    target_id -- independent of merge/list ordering."""
    projections = sorted(
        (semantic_projection(e) for e in merged), key=lambda p: p["target_id"]
    )
    canonical = json.dumps(projections, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()
