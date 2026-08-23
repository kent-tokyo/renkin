#!/usr/bin/env bash
# Phase A.5: process exactly ONE VAL-groups chunk and append its results to
# the arm's cumulative pool/groups/chunk-summary files. Split out from
# phase_a5_run_arm_chunked.sh (which looped over all 10 chunks in one shell
# invocation) because that loop was killed by the environment twice around
# the 40-70 minute cumulative-runtime mark, even though each individual
# chunk completes in ~12-25 minutes -- keeping each Bash tool call to one
# chunk stays safely under whatever that threshold is. Idempotent to
# resume: re-running an already-processed chunk index would double-append,
# so the caller must track which chunks are done (see findings.md).
#
# Usage: scripts/phase_a5_run_one_chunk.sh <arm_label> <chunk_path>
set -euo pipefail

ARM="$1"
CHUNK="$2"
IDX=$(basename "$CHUNK" .jsonl)

OUT="data/phase_a5_template_scaling/full_val"
mkdir -p "$OUT"

./target/release/renkin-pool-gen \
    --groups "$CHUNK" \
    --templates "data/phase_a5_template_scaling/templates/templates_${ARM}.smi" \
    --pool-output "${OUT}/${ARM}_${IDX}_pool.jsonl" \
    --groups-output "${OUT}/${ARM}_${IDX}_groups.jsonl" \
    --manifest-output "${OUT}/${ARM}_${IDX}_manifest.json" \
    2> "${OUT}/${ARM}_${IDX}_pool_gen.log"

python3 -c "
import json
lines = open('${OUT}/${ARM}_${IDX}_pool_gen.log').readlines()
start = max(i for i, l in enumerate(lines) if l.strip() == '{')
json.loads(''.join(lines[start:]))
open('${OUT}/${ARM}_${IDX}_summary.json', 'w').write(''.join(lines[start:]))
"

cat "${OUT}/${ARM}_${IDX}_pool.jsonl" >> "${OUT}/${ARM}_pool.jsonl"
cat "${OUT}/${ARM}_${IDX}_groups.jsonl" >> "${OUT}/${ARM}_groups.jsonl"
cat "${OUT}/${ARM}_${IDX}_summary.json" >> "${OUT}/${ARM}_chunk_summaries.jsonl"
echo >> "${OUT}/${ARM}_chunk_summaries.jsonl"

rm -f "${OUT}/${ARM}_${IDX}_pool.jsonl" "${OUT}/${ARM}_${IDX}_groups.jsonl" \
      "${OUT}/${ARM}_${IDX}_manifest.json" "${OUT}/${ARM}_${IDX}_pool_gen.log" \
      "${OUT}/${ARM}_${IDX}_summary.json"

echo "done: ${ARM} ${IDX}"
