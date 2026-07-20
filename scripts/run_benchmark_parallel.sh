#!/bin/bash
# Run scripts/run_benchmark_chunks.sh across N parallel shards, each pinned
# to a fixed rayon thread count. Round-robin shard assignment spreads any
# ordering-correlated difficulty evenly, and isolates per-molecule tail
# latency (a handful of pathological targets can take 100+s) so one slow
# molecule doesn't starve the other shards' cores.
set -e

INPUT="${1:-data/uspto50k_test.smi}"
TEMPLATES="${2:-data/templates_extracted_5000.smi}"
OUT_DIR="${3:-data/bench_chunks_corrected_baseline}"
DEPTH="${4:-5}"
BEAM="${5:-100}"
SHARDS="${6:-5}"
THREADS_PER_SHARD="${7:-2}"
BUILDING_BLOCKS="${8:-}"  # optional: path to building blocks .smi file (default: built-in ~160 BBs)

mkdir -p "$OUT_DIR"

echo "=== RENKIN parallel benchmark: $SHARDS shards x $THREADS_PER_SHARD threads ==="
echo "    input: $INPUT"
echo "    templates: $TEMPLATES"
echo "    depth=$DEPTH  beam=$BEAM"
echo "    building-blocks: ${BUILDING_BLOCKS:-<built-in default>}"
echo ""

grep -v "^#" "$INPUT" | awk -v n="$SHARDS" -v dir="$OUT_DIR" '{ print > (dir "/shard_" (NR % n) ".smi") }'

# A handful of chunks occasionally fail transiently under 5-way concurrent
# load (observed but not root-caused — not reproducible standalone). Each
# 100-mol chunk is checkpointed by run_benchmark_chunks.sh, so a rerun only
# redoes chunks whose output file is missing/empty. Repair passes exploit
# that instead of chasing the heisenbug.
#
# The launch loop backgrounds jobs directly in THIS shell (not inside a
# function called via command substitution) — command substitution forks a
# subshell, which would make the backgrounded PID a child of that subshell
# instead of this one, and `wait "$pid"` on it fails immediately with
# "not a child of this shell" instead of actually waiting.
for ATTEMPT in 1 2 3; do
    echo "=== pass $ATTEMPT: launching ${SHARDS} shards ==="
    PIDS=()
    for i in $(seq 0 $((SHARDS - 1))); do
        SHARD_DIR="$OUT_DIR/shard_${i}"
        mkdir -p "$SHARD_DIR"
        LOG="$OUT_DIR/shard_${i}.log"
        RSS_LOG="$OUT_DIR/shard_${i}.rss.txt"
        (
            export RAYON_NUM_THREADS="$THREADS_PER_SHARD"
            /usr/bin/time -l bash scripts/run_benchmark_chunks.sh \
                "$OUT_DIR/shard_${i}.smi" "$TEMPLATES" "$SHARD_DIR" "$DEPTH" "$BEAM" "" "$BUILDING_BLOCKS" 1 \
                > "$LOG" 2> "$RSS_LOG"
        ) &
        LAST_PID=$!
        PIDS+=("$LAST_PID")
        echo "shard $i started, pid $LAST_PID"
    done
    for pid in "${PIDS[@]}"; do
        wait "$pid" || true
    done

    WARNINGS=$(grep -l "WARN: renkin-bench failed" "$OUT_DIR"/shard_*.log 2>/dev/null | wc -l | tr -d ' ')
    if [ "$WARNINGS" -eq 0 ]; then
        echo "=== all shards complete, no failed chunks ==="
        exit 0
    fi
    echo "=== pass $ATTEMPT left $WARNINGS shard(s) with failed chunks — repairing ==="
done

echo "=== gave up after 3 passes — still failing, check $OUT_DIR/shard_*.log ==="
exit 1
