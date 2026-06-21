#!/bin/bash
# Run USPTO-50k benchmark in 100-molecule chunks.
# Results of each chunk are saved to data/bench_chunks/<chunk>.json
# A running total is printed after each chunk.

set -e

INPUT="${1:-data/uspto50k_test.smi}"
TEMPLATES="${2:-data/templates_extracted.smi}"
CHUNK_DIR="${3:-data/bench_chunks}"
DEPTH="${4:-3}"
BEAM="${5:-50}"

mkdir -p "$CHUNK_DIR"

# Split input into 100-line chunks (skip comment lines)
TMP_SPLIT="/tmp/bench_split_$$"
mkdir -p "$TMP_SPLIT"
grep -v "^#" "$INPUT" | split -l 100 - "$TMP_SPLIT/chunk_"

CHUNKS=$(ls "$TMP_SPLIT"/chunk_* | sort)
TOTAL_CHUNKS=$(echo "$CHUNKS" | wc -l | tr -d ' ')

echo "=== RENKIN benchmark: $TOTAL_CHUNKS chunks × 100 mol ==="
echo "    templates: $TEMPLATES"
echo "    depth=$DEPTH  beam=$BEAM"
echo "    results dir: $CHUNK_DIR"
echo ""

TOTAL_SOLVED=0
TOTAL_MOLS=0
CHUNK_NUM=0

# json_field <file> <field>: safely extract a numeric JSON field using jq or python3.
# Uses file path as a positional arg — never interpolated into code strings.
json_field() {
    local file="$1" field="$2"
    if command -v jq >/dev/null 2>&1; then
        jq ".$field" "$file" 2>/dev/null || echo 0
    else
        python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(d['$field'])" "$file" 2>/dev/null || echo 0
    fi
}

for CHUNK in $CHUNKS; do
    CHUNK_NUM=$((CHUNK_NUM + 1))
    CHUNK_NAME=$(basename "$CHUNK")
    OUT="$CHUNK_DIR/${CHUNK_NAME}.json"

    # Skip already-completed chunks
    if [ -s "$OUT" ]; then
        SOLVED=$(json_field "$OUT" solved)
        MOLS=$(json_field "$OUT" total)
        TOTAL_SOLVED=$((TOTAL_SOLVED + SOLVED))
        TOTAL_MOLS=$((TOTAL_MOLS + MOLS))
        echo "[${CHUNK_NUM}/${TOTAL_CHUNKS}] $CHUNK_NAME — already done ($SOLVED/$MOLS), skipping"
        continue
    fi

    echo -n "[${CHUNK_NUM}/${TOTAL_CHUNKS}] $CHUNK_NAME ... "
    START=$(date +%s)

    ./target/release/renkin-bench \
        --input "$CHUNK" \
        --depth $DEPTH \
        --beam-width $BEAM \
        --max-routes 1 \
        --templates "$TEMPLATES" \
        2>/dev/null > "$OUT"

    END=$(date +%s)
    ELAPSED=$((END - START))

    SOLVED=$(json_field "$OUT" solved)
    MOLS=$(json_field "$OUT" total)
    TOTAL_SOLVED=$((TOTAL_SOLVED + SOLVED))
    TOTAL_MOLS=$((TOTAL_MOLS + MOLS))

    RATE=$(python3 -c "print(f'{$TOTAL_SOLVED/$TOTAL_MOLS*100:.1f}%')" 2>/dev/null || echo "?")
    echo "solved $SOLVED/$MOLS in ${ELAPSED}s  |  cumulative: $TOTAL_SOLVED/$TOTAL_MOLS ($RATE)"
done

rm -rf "$TMP_SPLIT"

echo ""
echo "=== FINAL: $TOTAL_SOLVED / $TOTAL_MOLS ($(python3 -c "print(f'{$TOTAL_SOLVED/$TOTAL_MOLS*100:.1f}%')")) ==="
