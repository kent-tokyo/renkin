# Per-target audit: Issue #66 500-target comparison

Base commit: `e479b27` (tag `issue66-500-base-e479b27`). Harness: `feat/issue66-500-target-harness` @ `d1df6d6`.
Covers all 6 arms: RENKIN {Conservative, Disabled} × {shared_stock, native}, AiZynthFinder × {shared_stock, native}.

## Cross-arm integrity (all 6 arms, 3000 rows total)

- Total rows across all 6 arms: 3000 (500 × 6), confirmed.
- Every arm has exactly 500 unique `target_id`s — 0 duplicates within any arm.
- All 6 arms reference the **identical 500-target_id set**, and that set matches
  `sample_full_sorted.jsonl[:500]` exactly (frozen sampling protocol, `compare_sampling.load_sample`) —
  confirmed by set-equality check across all 6 arms plus the source sample file.
- Per-arm `compare_verify_arm.py` PASS with 0 problems for all 6 arms (see each arm's manifest/verify
  output for the individual runs). No malformed JSON, no schema nullability violations, no missing
  `normalized_route_sha256` on any `route_found=true` row, in any arm.
- Per-arm status accounting (`completed + timeout + error == 500`) holds for all 6 arms:

| Arm | completed | timeout | error |
|---|---|---|---|
| RENKIN Conservative × shared_stock | 500 | 0 | 0 |
| AiZynthFinder × shared_stock | 500 | 0 | 0 |
| RENKIN Conservative × native | 499 | 1 | 0 |
| AiZynthFinder × native | 500 | 0 | 0 |
| RENKIN Disabled × shared_stock | 499 | 1 | 0 |
| RENKIN Disabled × native | 499 | 1 | 0 |

- 0 real crashes/adapter failures in any arm. AiZynthFinder native mode carries 316 informational
  `native_stock_trusted_not_independently_verified` warnings — documented, expected behavior (native
  mode trusts AiZynthFinder's own per-leaf stock claim rather than re-verifying against its ~17.4M-compound
  ZINC stock), not a defect.
- Binary SHA-256 (`4dd50872...`) and git commit (`d1df6d6f...`) unchanged for the full duration of every
  RENKIN arm (4 arms). Docker image digest (`sha256:7ead07a1...`) unchanged for the full duration of both
  AiZynthFinder arms. Confirmed via each arm's manifest cross-check against the tree state after completion.
- Shared stock file hash (`9046b2e2...`) identical between the RENKIN and AiZynthFinder shared_stock arms
  — both tools ran against the byte-identical 393-compound stock in Phase 1.

## Route sanity spot-check (RENKIN Conservative, both modes)

- shared_stock: 73 solved, 73/73 `reaction_steps_parseable=true`, 73/73 `all_leaves_in_configured_stock=true`
  (expected by construction). `target_element_accounting_status`: 62 `accounted`, 11
  `unaccounted_target_element` (the known, documented diagnostic category from Issue #79's fix).
- `common_validation_warnings` seen across solved RENKIN routes: only `unaccounted_target_element` and
  `stereo_center_count_mismatch`, both pre-existing documented categories — no novel/unexplained warning
  type surfaced anywhere in the 500-target run.
- Native vs shared_stock stock-sensitivity: RENKIN Conservative's native stock (449 unique compounds,
  `data/building_blocks.smi`) and shared stock (393 compounds) differ substantially in membership, yet
  `route_found` differs on only 1/500 targets between the two modes — RENKIN's search behavior on this
  sample is largely stock-difference-insensitive within this stock-size range.

## Conservative vs Disabled ablation cross-check

- The same 4 target_ids flip (Disabled solves, Conservative doesn't) in **both** shared_stock and native
  modes: `uspto50k_test#L1167`, `uspto50k_test#L308`, `uspto50k_test#L4279`, `uspto50k_test#L984`. This
  mode-independence is consistent with the ring-context guard acting at template-match time rather than
  stock-lookup time. Full detail: `conservative_vs_disabled.md`, `conservative_vs_disabled_stats.json`.

## Paired-join integrity (headline comparisons)

- `compare_paired_report.py` join on `target_id` succeeded without a set-mismatch error for both
  shared_stock (RENKIN Conservative vs AiZynthFinder) and native mode pairs — confirms identical
  500-target_id sets between paired tool runs, independently of the cross-arm check above.

## Caveats carried into the final report

- Wall-clock/latency figures are single-machine, sequential-run observations (run order: shared_stock
  RENKIN → shared_stock AiZynthFinder → native RENKIN → native AiZynthFinder → disabled shared_stock →
  disabled native, over ~2026-08-04 14:00 to 2026-08-05 ~00:00 JST). Not a controlled cross-tool
  performance benchmark — thermal/cache/background-load conditions varied across arms.
- No selective re-run-and-patch was performed anywhere in this study: every headline number above is
  each arm's single (run 1) measurement, unmodified.
