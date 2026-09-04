# Current-master benchmark snapshot (2026-08-31)

This is a 50-target, current-master smoke benchmark for the Phase 0
reproducibility contract. It is intentionally separate from the historical
500-target comparison and is not a superiority claim or a replacement for the
formal 4,903-target measurement.

## Configuration

- Commit: `65e158a10cd95ba2b1cf0a083ba55e128e5ccf2b`
- Binary SHA-256: `sha256:8e63f922b536fee76f13ae95c3e74da2387fd3c32953c07f9fa11f4c68630f9b`
- Tool: RENKIN native binary
- Comparison mode: `shared_stock`
- Sample: first 50 rows of `data/comparison/sample_full_sorted.jsonl`
- Search: depth 5, beam width 100, conservative ring-context policy
- Templates: `data/templates_extracted_500.smi`
- Stock: `data/comparison/shared_stock/shared_stock.smi`
- Ring sidecar: `data/ring_context_metadata_500.json`
- External timeout: 150 seconds, 30-second grace period

The complete input and runtime provenance is in
`current_master_2026-08-31_manifest.json`; per-target rows are in
`current_master_2026-08-31_rows.jsonl` and the machine-readable aggregate is
in `current_master_2026-08-31_aggregate.json`.

## Results

| Metric | Result |
|---|---:|
| Targets | 50 |
| Route found | 4/50 (8.0%) |
| Route to configured stock | 4/50 (8.0%) |
| Validator-confirmed route | 4/50 (8.0%) |
| Route-tree parseable among found | 4/4 (100%) |
| Timeout / crash / setup error | 0 / 0 / 0 |
| Target-element-accounted route | 4/50 (8.0%) |
| Total elapsed p50 / p95 | 5.37 s / 51.46 s |
| Peak RSS p50 / p95 | 13.47 MiB / 17.39 MiB |

These values describe this fixed 50-target snapshot only. They must not be
compared directly with the historical 500-target artifact, which was produced
from an older commit and configuration.
