# Phase 1 report: shared_stock arms (500 targets)

Base commit: `e479b27` (tag `issue66-500-base-e479b27`). Harness: `feat/issue66-500-target-harness` @ `d1df6d6`.
RENKIN config: `--ring-context-policy conservative` (the official RENKIN configuration for this comparison).

## Arm results

| Arm | n | route_found_rate | wall_clock_total_sweep_s | timeouts | errors |
|---|---|---|---|---|---|
| RENKIN Conservative × shared_stock | 500 | 0.146 (73/500) | 5148.0 | 0 | 0 |
| AiZynthFinder × shared_stock | 500 | 0.050 (25/500) | 4151.3 | 0 | 0 |

Wall-clock observations on this machine and run order; not a controlled cross-tool performance benchmark.

## Integrity verification (`compare_verify_arm.py`, both arms)

- PASS: true, 0 problems, for both arms.
- Exact 500/500 rows, no duplicate/missing target_ids, `completed==500` (`timeout+error==0`).
- 0 malformed JSON lines, 0 schema nullability violations, 0 crash/adapter failures.
- `route_found=true` ⇒ `normalized_route_sha256` present: verified for all rows, both arms.
- Manifest cross-check: `input_files_unchanged_during_run: true` for both arms.
- RENKIN binary SHA-256 and git commit unchanged start-to-end (manually cross-checked against current tree).
- AiZynthFinder Docker image digest unchanged start-to-end (manually cross-checked).
- Shared stock file hash identical across both arms' manifests (`9046b2e2...`) — confirms both tools ran against the byte-identical 393-compound stock.

## Paired stats (`compare_paired_report.py --mode shared_stock`)

- `route_to_shared_stock` rate diff (RENKIN − AiZynthFinder): observed 0.098, 95% CI [0.070, 0.128], McNemar p ≈ 1.9e-11 (RENKIN-only 54, AiZynthFinder-only 5).
- Secondary tool-native `route_found` diff: observed 0.096, 95% CI [0.068, 0.126], McNemar p ≈ 9.7e-11 (RENKIN-only 54, AiZynthFinder-only 6).
- Full detail: `paired_stats_shared_stock.json`, per-target join: `paired_table_shared_stock.json`.

## RENKIN route sanity spot-check

- 73 solved: 73/73 `reaction_steps_parseable=true`, 73/73 `all_leaves_in_configured_stock=true` (expected by shared_stock construction).
- `target_element_accounting_status`: 62 `accounted`, 11 `unaccounted_target_element` — the known, documented diagnostic category from Issue #79's fix (not a new/unexplained defect).
- `common_validation_warnings` seen: only `unaccounted_target_element` and `stereo_center_count_mismatch`, both pre-existing documented categories. No novel warning types.

## Stop-condition evaluation (per user's Phase 1 gate)

| Condition | Result |
|---|---|
| ≥1 adapter/schema error | None — 0 problems both arms |
| `route_found=true` with missing hash | None — 0/0 |
| Missing/duplicate targets | None — exact 500/500, identical target_id sets both arms |
| Stock-identity manifest mismatch | None — stock hash identical across both arms |
| New, unexplained-incorrect RENKIN Conservative route | None found in spot-check — only known diagnostic categories |
| Mass AiZynthFinder crashes/environment errors | None — 0 crashes, 500/500 completed |
| Results wildly divergent from 100-target baseline | No — RENKIN 16%→14.6%, AiZynthFinder 4%→5% (100→500 baseline), within normal scaling fluctuation (explicitly not a stop reason per protocol) |

**Decision: no stop condition triggered. Proceeding to Phase 2 (native arms).**
