#!/usr/bin/env bash
# Phase A.5: once every chunk for an arm has been processed via
# scripts/phase_a5_run_one_chunk.sh, aggregate the per-chunk
# renkin-pool-gen summaries into one equivalent summary and compute the
# arm's coverage/dedup/recall metrics. Fast (no pool-gen call) -- safe to
# run synchronously, not backgrounded.
#
# Usage: scripts/phase_a5_finalize_arm.sh <arm_label>
set -euo pipefail

ARM="$1"
OUT="data/phase_a5_template_scaling/full_val"

python3 -c "
import json
from pathlib import Path
from collections import Counter

out_dir = Path('${OUT}')
arm = '${ARM}'

chunks = [json.loads(l) for l in (out_dir / f'{arm}_chunk_summaries.jsonl').read_text().split(chr(10)+chr(10)) if l.strip()]
print(f'{len(chunks)} chunk summaries found')

per_group = Counter()
for row in (json.loads(l) for l in (out_dir / f'{arm}_pool.jsonl').read_text().splitlines() if l.strip()):
    per_group[row['group_id']] += 1
all_group_ids = {json.loads(l)['group_id'] for l in (out_dir / f'{arm}_groups.jsonl').read_text().splitlines() if l.strip()}
counts = sorted(per_group.get(g, 0) for g in all_group_ids)

def percentile(sorted_vals, p):
    if not sorted_vals:
        return 0
    idx = round((len(sorted_vals) - 1) * p)
    return sorted_vals[min(idx, len(sorted_vals) - 1)]

summary = {
    'n_groups_requested': sum(c['n_groups_requested'] for c in chunks),
    'n_groups_parse_failed': sum(c['n_groups_parse_failed'] for c in chunks),
    'n_groups_target_id_mismatch': sum(c['n_groups_target_id_mismatch'] for c in chunks),
    'n_groups_zero_candidates': sum(c['n_groups_zero_candidates'] for c in chunks),
    'n_candidate_rows': sum(c['n_candidate_rows'] for c in chunks),
    'candidates_per_group_p50': percentile(counts, 0.50),
    'candidates_per_group_p90': percentile(counts, 0.90),
    'candidates_per_group_p95': percentile(counts, 0.95),
    'candidates_per_group_max': counts[-1] if counts else 0,
    'wall_clock_seconds': sum(c['wall_clock_seconds'] for c in chunks),
    'chunked': True,
    'n_chunks': len(chunks),
}
(out_dir / f'{arm}_pool_gen_summary.json').write_text(json.dumps(summary, indent=2))
print(json.dumps(summary, indent=2))
"

python3 scripts/phase_a5_pool_metrics.py \
    --pool "${OUT}/${ARM}_pool.jsonl" \
    --groups "${OUT}/${ARM}_groups.jsonl" \
    --labels data/reranker_labels_uspto50k_val.jsonl \
    --pool-gen-summary "${OUT}/${ARM}_pool_gen_summary.json" \
    --arm-label "$ARM" \
    --output "${OUT}/${ARM}_metrics.json" \
    > /dev/null

echo "done: full_val/${ARM} (finalized)"
