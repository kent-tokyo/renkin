# Conservative vs Disabled ablation report (500 targets, RENKIN only)

Base commit: `e479b27` (tag `issue66-500-base-e479b27`). Harness: `feat/issue66-500-target-harness` @ `d1df6d6`.

Phase 3 checks whether the ring-context safety guard (Issue #72/#242) has a measurable cost at 500-target
scale. **RENKIN Disabled is not a headline arm of this comparison** — it carries the legacy ring-context
mis-application issue from #72 (a known correctness gap, not a performance baseline). Conservative
(`--ring-context-policy conservative`) is the official RENKIN configuration for the Issue #66
headline comparison (Phase 1/Phase 2). This report is a sensitivity/ablation analysis only.

## Arm results

| Arm | n | route_found_rate | timeout_rate |
|---|---|---|---|
| RENKIN Conservative × shared_stock | 500 | 0.146 (73/500) | 0.000 |
| RENKIN Disabled × shared_stock | 500 | 0.154 (77/500) | 0.002 (1/500) |
| RENKIN Conservative × native | 500 | 0.146 (73/500) | 0.002 (1/500) |
| RENKIN Disabled × native | 500 | 0.154 (77/500) | 0.002 (1/500) |

All 4 arms verified PASS (`compare_verify_arm.py`): exact 500/500 coverage, 0 problems, binary SHA-256
and git commit unchanged for the full duration of each arm.

## Paired ablation stats (Conservative vs Disabled, same 500 targets)

| Mode | metric | n_pairs | conservative_only | disabled_only | observed diff (Cons − Dis) | 95% CI | McNemar p |
|---|---|---|---|---|---|---|---|
| shared_stock | route_to_shared_stock | 500 | 0 | 4 | −0.008 | [−0.016, −0.002] | 0.125 |
| native | route_found | 500 | 0 | 4 | −0.008 | [−0.016, −0.002] | 0.125 |

Both modes: Disabled solves exactly 4 more targets than Conservative, Conservative solves 0 targets that
Disabled misses — and it is the **same 4 target_ids** in both modes:
`uspto50k_test#L1167`, `uspto50k_test#L308`, `uspto50k_test#L4279`, `uspto50k_test#L984`.

This is consistent with the guard acting at template-match time (independent of which stock is loaded)
rather than at stock-lookup time — the same templates get blocked regardless of comparison_mode, so the
same targets flip in both arms. McNemar p=0.125 (n=4 discordant pairs) does not reach conventional
significance at 500 targets.

Because only four discordant pairs were observed, the percentile-bootstrap interval is descriptive; the
exact McNemar test is the primary inferential result for this sparse ablation, and it did not reach
statistical significance.

## Interpretation

**In this sample, no statistically significant difference was detected between Conservative and Disabled**
(observed −0.8 percentage points, 95% CI [−1.6, −0.2], exact McNemar p=0.125, n=4 discordant pairs out of
500). This is not the same claim as "the guard has zero cost": the observed direction (Disabled solving 4
targets Conservative doesn't, Conservative solving 0 that Disabled doesn't) is consistent with Conservative
blocking a small number of correct routes, and it points the same direction as the 100-target measurement
already reported in #242's draft PR. At n=500 with only 4 discordant pairs, this study cannot confirm or
rule out a real effect of this size — it is directionally consistent with, but does not statistically
confirm, the #242 tradeoff. Full stats: `conservative_vs_disabled_stats.json`.
