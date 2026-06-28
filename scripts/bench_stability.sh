#!/usr/bin/env bash
# Collect quietset observations across multiple beam widths, then score stability.
#
# Usage:
#   bash scripts/bench_stability.sh --input <smiles_file> [options]
#
# Options:
#   --input|-i <file>         SMILES input file (required)
#   --beams <w1,w2,...>       Comma-separated beam widths (default: 50,100,200)
#   --depth|-d <N>            Search depth (default: 5)
#   --templates <path>        Template file (optional)
#   --building-blocks|-b <path>  Building blocks file (optional)
#   --out-dir <dir>           Output directory (default: bench_stability_out)
#   --min-observations <N>    quietset --min-observations-keep (default: 2)
#
# Outputs (in --out-dir):
#   observations.jsonl        Raw quietset observations (one line per molecule per run)
#   stability.jsonl           quietset score output
#   stable_targets.jsonl      Targets with decision=keep
#
# Requires quietset: cargo install quietset-cli
set -euo pipefail

INPUT=""
BEAMS="50,100,200"
DEPTH=5
TEMPLATES_ARG=()
BB_ARG=()
OUT_DIR="bench_stability_out"
MIN_OBS=2
EXTRA_BENCH_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --input|-i)       INPUT="$2"; shift 2 ;;
    --beams)          BEAMS="$2"; shift 2 ;;
    --depth|-d)       DEPTH="$2"; shift 2 ;;
    --templates)      TEMPLATES_ARG=(--templates "$2"); shift 2 ;;
    --building-blocks|-b) BB_ARG=(--building-blocks "$2"); shift 2 ;;
    --out-dir)        OUT_DIR="$2"; shift 2 ;;
    --min-observations) MIN_OBS="$2"; shift 2 ;;
    *) EXTRA_BENCH_ARGS+=("$1"); shift ;;
  esac
done

if [[ -z "$INPUT" ]]; then
  echo "Usage: $0 --input <smiles_file> [--beams 50,100,200] [--depth 5] [--out-dir <dir>]"
  exit 1
fi

if [[ ! -f target/release/renkin-bench ]]; then
  echo "[build] cargo build --bin renkin-bench --release"
  cargo build --bin renkin-bench --release 2>&1
fi

mkdir -p "$OUT_DIR"
OBS="$OUT_DIR/observations.jsonl"
rm -f "$OBS"

IFS=',' read -ra BEAM_LIST <<< "$BEAMS"

echo "=== RENKIN bench_stability ==="
echo "Input  : $INPUT"
echo "Depth  : $DEPTH  |  Beams: $BEAMS"
echo "Output : $OUT_DIR"
echo ""

for beam in "${BEAM_LIST[@]}"; do
  echo "[run] depth=$DEPTH beam=$beam"
  ./target/release/renkin-bench \
    --input "$INPUT" \
    --depth "$DEPTH" \
    --beam-width "$beam" \
    "${TEMPLATES_ARG[@]+"${TEMPLATES_ARG[@]}"}" \
    "${BB_ARG[@]+"${BB_ARG[@]}"}" \
    "${EXTRA_BENCH_ARGS[@]+"${EXTRA_BENCH_ARGS[@]}"}" \
    --quietset-out "$OBS" \
    > /dev/null
done

N_OBS=$(wc -l < "$OBS")
echo ""
echo "Observations: $N_OBS lines → $OBS"

if ! command -v quietset &>/dev/null; then
  echo ""
  echo "quietset not found. Install with:"
  echo "  cargo install quietset-cli"
  echo ""
  echo "Then run:"
  echo "  quietset score $OBS --use-adjusted-score --min-observations-keep $MIN_OBS > $OUT_DIR/stability.jsonl"
  echo "  quietset filter $OUT_DIR/stability.jsonl --decision keep > $OUT_DIR/stable_targets.jsonl"
  exit 0
fi

echo ""
echo "[quietset] scoring..."
quietset score "$OBS" \
  --use-adjusted-score \
  --min-observations-keep "$MIN_OBS" \
  > "$OUT_DIR/stability.jsonl"

quietset filter "$OUT_DIR/stability.jsonl" --decision keep \
  > "$OUT_DIR/stable_targets.jsonl"

N_STABLE=$(wc -l < "$OUT_DIR/stable_targets.jsonl")
N_TOTAL=$(wc -l < "$OUT_DIR/stability.jsonl")

echo ""
echo "=== Results ==="
quietset summary "$OUT_DIR/stability.jsonl" 2>/dev/null || true
echo ""
echo "Stable (keep): $N_STABLE / $N_TOTAL targets"
echo "Files:"
echo "  $OBS"
echo "  $OUT_DIR/stability.jsonl"
echo "  $OUT_DIR/stable_targets.jsonl"
