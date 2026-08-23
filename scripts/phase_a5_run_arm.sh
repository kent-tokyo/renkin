#!/usr/bin/env bash
# Phase A.5: run one (stage, template-size) arm -- renkin-pool-gen over the
# full VAL groups file (candidate generation only, no route search/reranker),
# capture its own stdout summary, then compute coverage/dedup/recall metrics.
#
# Usage: scripts/phase_a5_run_arm.sh <stage_dir> <arm_label> <limit-or-empty>
#   scripts/phase_a5_run_arm.sh smoke_100 500 100
#   scripts/phase_a5_run_arm.sh full_val 10000 ""
set -euo pipefail

STAGE_DIR="$1"
ARM="$2"
LIMIT="${3:-}"

OUT="data/phase_a5_template_scaling/${STAGE_DIR}"
mkdir -p "$OUT"

if [ -n "$LIMIT" ]; then
  ./target/release/renkin-pool-gen \
      --groups data/reranker_groups_uspto50k_val.jsonl \
      --templates "data/phase_a5_template_scaling/templates/templates_${ARM}.smi" \
      --pool-output "${OUT}/${ARM}_pool.jsonl" \
      --groups-output "${OUT}/${ARM}_groups.jsonl" \
      --manifest-output "${OUT}/${ARM}_manifest.json" \
      --limit "$LIMIT" \
      2> "${OUT}/${ARM}_pool_gen.log"
else
  ./target/release/renkin-pool-gen \
      --groups data/reranker_groups_uspto50k_val.jsonl \
      --templates "data/phase_a5_template_scaling/templates/templates_${ARM}.smi" \
      --pool-output "${OUT}/${ARM}_pool.jsonl" \
      --groups-output "${OUT}/${ARM}_groups.jsonl" \
      --manifest-output "${OUT}/${ARM}_manifest.json" \
      2> "${OUT}/${ARM}_pool_gen.log"
fi

# renkin-pool-gen's feasibility summary is the trailing pretty-printed JSON
# block on stderr (progress lines precede it, also on stderr) -- pull out
# just that block, from the last top-level "{" to end of file.
python3 -c "
import json
lines = open('${OUT}/${ARM}_pool_gen.log').readlines()
start = max(i for i, l in enumerate(lines) if l.strip() == '{')
json.loads(''.join(lines[start:]))
open('${OUT}/${ARM}_pool_gen_summary.json', 'w').write(''.join(lines[start:]))
"

python3 scripts/phase_a5_pool_metrics.py \
    --pool "${OUT}/${ARM}_pool.jsonl" \
    --groups "${OUT}/${ARM}_groups.jsonl" \
    --labels data/reranker_labels_uspto50k_val.jsonl \
    --pool-gen-summary "${OUT}/${ARM}_pool_gen_summary.json" \
    --arm-label "$ARM" \
    --output "${OUT}/${ARM}_metrics.json" \
    > /dev/null

echo "done: ${STAGE_DIR}/${ARM}"
