#!/usr/bin/env bash
# Phase A.5: same as phase_a5_run_arm.sh but processes the VAL groups file
# in 500-group chunks (data/phase_a5_template_scaling/chunks/) and
# concatenates the results -- needed for the 5000/10000-template arms,
# whose single-shot full-VAL runtime (~2-4h) hit an environment-level kill
# twice in a row on the unchunked version. 500-group chunks are a size
# already proven to complete reliably (Stage 2 ran exactly this size in one
# shot for every arm, including 10000 templates: 1467s).
#
# Usage: scripts/phase_a5_run_arm_chunked.sh <arm_label>
set -euo pipefail

ARM="$1"
OUT="data/phase_a5_template_scaling/full_val"
CHUNK_DIR="data/phase_a5_template_scaling/chunks"
mkdir -p "$OUT"

: > "${OUT}/${ARM}_pool.jsonl"
: > "${OUT}/${ARM}_groups.jsonl"
: > "${OUT}/${ARM}_chunk_summaries.jsonl"

for chunk in "${CHUNK_DIR}"/val_groups_*.jsonl; do
  idx=$(basename "$chunk" .jsonl)
  echo "=== chunk ${idx} ==="
  ./target/release/renkin-pool-gen \
      --groups "$chunk" \
      --templates "data/phase_a5_template_scaling/templates/templates_${ARM}.smi" \
      --pool-output "${OUT}/${ARM}_${idx}_pool.jsonl" \
      --groups-output "${OUT}/${ARM}_${idx}_groups.jsonl" \
      --manifest-output "${OUT}/${ARM}_${idx}_manifest.json" \
      2> "${OUT}/${ARM}_${idx}_pool_gen.log"

  python3 -c "
import json
lines = open('${OUT}/${ARM}_${idx}_pool_gen.log').readlines()
start = max(i for i, l in enumerate(lines) if l.strip() == '{')
json.loads(''.join(lines[start:]))
open('${OUT}/${ARM}_${idx}_summary.json', 'w').write(''.join(lines[start:]))
"

  cat "${OUT}/${ARM}_${idx}_pool.jsonl" >> "${OUT}/${ARM}_pool.jsonl"
  cat "${OUT}/${ARM}_${idx}_groups.jsonl" >> "${OUT}/${ARM}_groups.jsonl"
  cat "${OUT}/${ARM}_${idx}_summary.json" >> "${OUT}/${ARM}_chunk_summaries.jsonl"
  echo >> "${OUT}/${ARM}_chunk_summaries.jsonl"

  rm -f "${OUT}/${ARM}_${idx}_pool.jsonl" "${OUT}/${ARM}_${idx}_groups.jsonl" \
        "${OUT}/${ARM}_${idx}_manifest.json" "${OUT}/${ARM}_${idx}_pool_gen.log" \
        "${OUT}/${ARM}_${idx}_summary.json"
done

# Aggregate the per-chunk renkin-pool-gen summaries into one equivalent
# summary -- sums are exact; candidates-per-group percentiles are
# recomputed from the concatenated pool (not averaged across chunks'
# own percentiles, which wouldn't combine validly).
python3 -c "
import json
from pathlib import Path
from collections import Counter

out_dir = Path('${OUT}')
arm = '${ARM}'

chunks = [json.loads(l) for l in (out_dir / f'{arm}_chunk_summaries.jsonl').read_text().split(chr(10)+chr(10)) if l.strip()]

per_group = Counter()
for row in (json.loads(l) for l in (out_dir / f'{arm}_pool.jsonl').read_text().splitlines() if l.strip()):
    per_group[row['group_id']] += 1
# Zero-candidate groups have no pool rows at all -- count them from the
# groups index too, so the percentile denominator matches renkin-pool-gen's
# own (which includes zero-candidate groups as a 0 in candidate_counts).
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

echo "done: full_val/${ARM} (chunked)"
