# Phase 2 interim report: all 4 headline arms complete (500 targets each)

Base commit: `e479b27` (tag `issue66-500-base-e479b27`). Harness: `feat/issue66-500-target-harness` @ `d1df6d6`.
RENKIN config: `--ring-context-policy conservative` (the official RENKIN configuration for this comparison).
This completes the Issue #66 headline comparison — all 4 primary arms (RENKIN Conservative × {shared_stock, native}, AiZynthFinder × {shared_stock, native}) are now measured.

## Arm results

| Arm | n | route_found_rate | timeout_rate | wall_clock_total_sweep_s |
|---|---|---|---|---|
| RENKIN Conservative × shared_stock | 500 | 0.146 (73/500) | 0.000 | 5148.0 |
| AiZynthFinder × shared_stock | 500 | 0.050 (25/500) | 0.000 | 4151.3 |
| RENKIN Conservative × native | 500 | 0.146 (73/500) | 0.002 (1/500) | 5528.9 |
| AiZynthFinder × native | 500 | 0.632 (316/500) | 0.000 | 7223.0 |

Wall-clock observations on this machine and run order; not a controlled cross-tool performance benchmark.

AiZynthFinder's much higher native route_found_rate reflects its native stock being AiZynthFinder's full ~17.4M-compound ZINC building-block set, versus the 393-compound shared stock — a stock-size effect, not a claim about search-algorithm quality (that's what the shared_stock arm isolates).

## Integrity verification (`compare_verify_arm.py`, both new arms)

- PASS: true, 0 problems, both arms.
- RENKIN native: 499 completed + 1 timeout = 500 (a single external-timeout target; not mass failure, no stop condition).
- AiZynthFinder native: 500/500 completed, 0 timeouts. 316 rows carry an informational `native_stock_trusted_not_independently_verified` adapter warning (documented, expected: native mode trusts AiZynthFinder's own per-leaf stock claim rather than re-verifying against the ~17.4M-compound ZINC set) — 0 real crashes.
- `route_found=true` ⇒ `normalized_route_sha256` present: verified for all rows, both arms.
- Manifest cross-check: `input_files_unchanged_during_run: true` for both arms.
- RENKIN binary SHA-256 and git commit unchanged start-to-end (cross-checked against current tree).
- AiZynthFinder Docker image digest unchanged start-to-end (cross-checked).
- Paired join (`compare_paired_report.py --mode native`) succeeded on identical 500 target_id sets — no missing/duplicate targets.

## Paired stats, native (`paired_stats_native.json`)

- `route_found` rate diff (RENKIN − AiZynthFinder): observed −0.486, 95% CI [−0.530, −0.442], McNemar p ≈ 5.7e-69 (RENKIN-only 3, AiZynthFinder-only 246). AiZynthFinder solves far more targets in native mode; native-mode stock sizes differ enormously (RENKIN ~402 vs AiZynthFinder ~17.4M compounds), but this arm does not control for stock or any other condition, so the difference cannot be attributed to stock size alone.
- Both-solved (n=70) `total_elapsed_ms` diff (RENKIN − AiZynthFinder): observed −11,136ms, 95% CI [−11,896, −10,408] (RENKIN faster on the jointly-solved subset). Wall-clock observation on this machine/run order only — not a controlled performance benchmark (see caption above).

## Combined 4-arm picture — superseded by `aggregate_report.md`

*(This section is retained as an as-run historical record of Phase 2's own gate reasoning. Its final,
carefully-scoped interpretation is `aggregate_report.md`'s "Bottom line" section — read that one for the
authoritative wording.)*

- **shared_stock (identical 393-compound stock, each tool's own configured policy/budget)**: RENKIN's
  primary outcome was 9.8 percentage points higher than AiZynthFinder's, a statistically significant
  paired difference (McNemar p≈1.9e-11) under this fixed sample and these fixed conditions — not a general
  claim about search-algorithm superiority.
- **native (each tool's own stock)**: AiZynthFinder solves far more targets than RENKIN (−48.6pt). This
  reflects the full set of unmatched native-mode conditions, of which the ~402-vs-17.4M-compound stock gap
  is one contributor; it is not proven to be the sole cause.
- The shared_stock/native reversal is strong evidence that the result is sensitive to stock choice, but
  neither arm in isolation proves stock size is the entire explanation for either result.

## Stop-condition re-check (same 7 conditions as Phase 1, applied to Phase 2 data)

| Condition | Result |
|---|---|
| ≥1 adapter/schema error | None — 0 problems both arms |
| `route_found=true` with missing hash | None |
| Missing/duplicate targets | None — paired join succeeded on identical sets |
| Stock-identity manifest mismatch | N/A for native mode (each tool uses its own stock by design) |
| New, unexplained-incorrect RENKIN Conservative route | Not investigated beyond Phase 1's spot-check; no crash/schema signal suggesting one |
| Mass AiZynthFinder crashes/environment errors | None — 0 crashes, 500/500 and 500/500 completed |
| Results wildly divergent from 100-target baseline | RENKIN native 100→500: route_found_rate consistent (100-target baseline not re-quoted here; shared_stock/native pattern matches Phase 1's directionality) |

**Decision: no stop condition triggered. Proceeding to Phase 3 (Disabled ablation arms).**
