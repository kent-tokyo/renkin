#!/usr/bin/env bash
# Run the RENKIN benchmark and print a human-readable summary.
#
# Usage:
#   ./scripts/run_benchmark.sh [--input <file>] [--depth N] [--beam-width N]
#
# Defaults:
#   --input data/benchmark_targets.smi
#   --depth 5
#   --beam-width 0  (unlimited A*)
set -euo pipefail

INPUT="data/benchmark_targets.smi"
DEPTH=5
BEAM=0
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --input|-i)  INPUT="$2"; shift 2 ;;
    --depth|-d)  DEPTH="$2"; shift 2 ;;
    --beam-width|-w) BEAM="$2"; shift 2 ;;
    *) EXTRA_ARGS+=("$1"); shift ;;
  esac
done

# Build release binary if needed
if [[ ! -f target/release/renkin-bench ]]; then
  echo "[build] cargo build --bin renkin-bench --release"
  cargo build --bin renkin-bench --release 2>&1
fi

echo ""
echo "=== RENKIN Benchmark ==="
echo "Input : $INPUT"
echo "Depth : $DEPTH  |  Beam: $BEAM"
echo "========================"
echo ""

REPORT=$(./target/release/renkin-bench \
  --input "$INPUT" \
  --depth "$DEPTH" \
  --beam-width "$BEAM" \
  "${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}" \
  2>/dev/null)

# Summary using Python (stdlib only)
python3 - "$REPORT" <<'PYEOF'
import json, sys

data = json.loads(sys.argv[1])

total   = data["total"]
solved  = data["solved"]
rate    = data["success_rate"] * 100
adepth  = data["avg_depth"]
atime   = data["avg_time_ms"]

print(f"  Total targets : {total}")
print(f"  Solved        : {solved}  ({rate:.1f}%)")
print(f"  Avg depth     : {adepth:.2f}")
print(f"  Avg time      : {atime:.1f} ms/target")
print()

# Failed targets
failed = [r for r in data["results"] if not r["solved"]]
if failed:
    print(f"  Unsolved ({len(failed)}):")
    for r in failed:
        name = r.get("name","") or r["smiles"]
        print(f"    - {r['smiles']}  ({name})")
else:
    print("  All targets solved!")

# Slowest targets
slowest = sorted(data["results"], key=lambda r: r["time_ms"], reverse=True)[:5]
print()
print("  Slowest 5:")
for r in slowest:
    name = r.get("name","") or r["smiles"]
    print(f"    {r['time_ms']:7.1f} ms  {name}")
PYEOF

echo ""
echo "Full JSON report: run without the script to get raw output:"
echo "  ./target/release/renkin-bench --input $INPUT --depth $DEPTH"
