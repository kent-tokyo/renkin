# Issue #66: RENKIN vs AiZynthFinder, 500-target comparison — final report

Base commit: `e479b27bedbc8849dd2f20692d67652b0858447c` (tag `issue66-500-base-e479b27`, post-PR-#83 master).
Harness: `feat/issue66-500-target-harness` @ `d1df6d6f300fb2410e191710dc952d39b458cf8a`.
Sample: `sample_full_sorted.jsonl[:500]` (frozen sampling protocol, `compare_sampling.load_sample`).
RENKIN config for the headline comparison: `--ring-context-policy conservative` (the official RENKIN
configuration — Disabled is an ablation-only arm, see below).

All 6 arms complete, all individually integrity-verified (`compare_verify_arm.py`, PASS, 0 problems each).
Cross-arm audit: `per_target_audit.md`. Run manifests (binary/commit/Docker/input-file hashes, host
environment at start and end of each arm): `run_manifest.json`.

## The 4 headline arms

| Arm | n | route_found (numerator/500) | route_to_shared_stock (numerator/500) | timeout_rate |
|---|---|---|---|---|
| RENKIN Conservative × shared_stock | 500 | 73/500 (0.146) | 73/500 (0.146) | 0/500 |
| AiZynthFinder × shared_stock | 500 | 25/500 (0.050) | 24/500 (0.048) | 0/500 |
| RENKIN Conservative × native | 500 | 73/500 (0.146) | n/a (native mode) | 1/500 |
| AiZynthFinder × native | 500 | 316/500 (0.632) | n/a (native mode) | 0/500 |

`route_to_shared_stock` (shared_stock mode only) is the primary, independently-re-verified metric used
for the headline shared_stock comparison below; `route_found` is each tool's own claim.

Wall-clock observations on this machine and run order (sequential, single machine, 2026-08-04 to
2026-08-05); not a controlled cross-tool performance benchmark — no cross-tool latency-superiority claim
is made anywhere in this report.

## shared_stock (identical 393-compound stock for both tools)

Primary metric: **`route_to_shared_stock`** (`route_found AND route_tree_parseable AND
all_leaves_in_configured_stock`, independently re-verified — not each tool's own claim).

- RENKIN Conservative: **73/500** (0.146).
- AiZynthFinder: **24/500** (0.048). (AiZynthFinder's tool-native `route_found` was 25/500; one of those
  25 routes had a leaf outside the shared stock and does not count toward `route_to_shared_stock`.)
- Paired McNemar discordant counts: RENKIN-only 54, AiZynthFinder-only 5 (n=500 pairs).
- Observed difference (RENKIN − AiZynthFinder): **+9.8 percentage points**, 95% CI [7.0, 12.8],
  exact McNemar p ≈ 1.9e-11.
- Full detail: `paired_stats_shared_stock.json`, `paired_table_shared_stock.json`.

**Interpretation, scoped to what this arm actually controls:** under this fixed 500-target sample, the
shared 393-compound stock, and each tool's configured policy and search budget for this run, RENKIN
Conservative's primary outcome was 9.8 percentage points higher than AiZynthFinder's, and the paired
outcome difference was statistically significant. This does **not** establish that RENKIN's search
capability is superior in general, and it does not prove that stock differences are the *only* thing
shared_stock controls for — template/model differences, policy calibration, search budget, and other
internal-behavior differences between the two tools remain unmatched. It isolates *stock choice*, not
search-engine quality in isolation (see `docs/guides/open-source-retrosynthesis-comparison.md` and the
100-target report's own caveat on this point).

## native (each tool's own stock)

Primary metric: **`route_found`** (tool-native claim; `route_to_shared_stock` does not apply — there is
no shared stock in this mode).

- RENKIN Conservative: **73/500** (0.146).
- AiZynthFinder: **316/500** (0.632). AiZynthFinder's native stock is its own ~17.4M-compound ZINC
  building-block set; RENKIN's native stock is 402 compounds (`data/building_blocks.smi`).
- Paired McNemar discordant counts: RENKIN-only 3, AiZynthFinder-only 246 (n=500 pairs).
- Observed difference (RENKIN − AiZynthFinder): **−48.6 percentage points**, 95% CI [−53.0, −44.2],
  exact McNemar p ≈ 5.7e-69.
- Both-solved (n=70) `total_elapsed_ms` diff (RENKIN − AiZynthFinder): observed −11,136ms
  (RENKIN faster on this jointly-solved subset) — wall-clock observation only, not a controlled benchmark.
- Full detail: `paired_stats_native.json`, `paired_table_native.json`.

**Interpretation, scoped to what this arm actually shows:** the native-configuration −48.6pt difference
reflects the *entire* set of unmatched conditions between the two arms — including, but not limited to, a
stock-size difference of roughly 402 vs 17.4 million compounds. That the direction of the difference
reverses under shared_stock is strong evidence that the result is sensitive to stock choice, but this does
not prove that the full magnitude of the native-mode gap is caused by stock size alone; template/model
differences, policy calibration, and search-budget differences between the two tools' native configurations
were not controlled for and could also contribute.

## Ablation: RENKIN Conservative vs Disabled (ring-context guard, Issue #72/#242)

**Not a headline arm.** Disabled carries the legacy ring-context mis-application issue from Issue #72;
Conservative is the official configuration. Included only as a sensitivity check at 500-target scale.

- Both shared_stock and native: Disabled solves 4 more targets than Conservative (the **same 4 targets**
  in both modes: `uspto50k_test#L1167`, `uspto50k_test#L308`, `uspto50k_test#L4279`, `uspto50k_test#L984`),
  Conservative solves 0 that Disabled misses. Conservative: 73/500 both modes. Disabled: 77/500 both modes.
- Paired McNemar discordant counts: Conservative-only 0, Disabled-only 4 (n=500 pairs, both modes).
- Observed difference (Conservative − Disabled): **−0.8 percentage points**, 95% CI [−1.6, −0.2],
  exact McNemar p = 0.125. Because only four discordant pairs were observed, the percentile-bootstrap
  interval is descriptive; the exact McNemar test is the primary inferential result for this sparse
  ablation, and it did not reach statistical significance.
- **In this sample, no statistically significant difference was detected between Conservative and
  Disabled.** This is not the same as "zero cost" — the observed direction is consistent with Conservative
  blocking a small number of correct routes, and the point estimate and 100-target measurement in #242
  both point the same direction — but n=4 discordant pairs at n=500 targets does not reach significance.
- Full detail: `conservative_vs_disabled.md`, `conservative_vs_disabled_stats.json`.

## Bottom line

- **shared_stock, under the fixed conditions of this run** (500-target sample, shared 393-compound stock,
  each tool's configured policy and search budget): RENKIN Conservative's primary outcome was 9.8
  percentage points higher than AiZynthFinder's, a statistically significant paired difference
  (p≈1.9e-11, n=500). This is not a general claim that RENKIN's search capability is superior, and it does
  not isolate search-engine quality in full — see the scoped interpretation above.
- **native-mode's −48.6pt difference reflects the full set of unmatched conditions** between the two
  arms, of which the ~402-vs-17.4M-compound stock difference is one contributor; the shared_stock reversal
  is strong evidence of stock sensitivity but does not prove stock size explains the entire native-mode gap.
- **The Conservative ring-context guard's cost was not statistically significant in this sample** (−0.8pt,
  p=0.125, n=4 discordant pairs) — directionally consistent with, but not confirming, the tradeoff reported
  from #242's 100-target measurement.

## Stop conditions

No stop condition was triggered at any phase gate (Phase 1 gate after the two shared_stock arms; Phase 2
re-check after all 4 headline arms). Every arm ran to completion once started, at the pinned commit, with
0 real crashes and 0 schema violations across all 3000 rows. Full stop-gate evaluations:
`phase1_shared_stock_report.md`, `phase2_interim_report.md`.

## Reproduction

All aggregates and paired reports below were independently regenerated from the raw per-arm `rows.jsonl`
files into a scratch directory and diffed against the checked-in files: aggregates matched exactly modulo
timing/provenance fields (`wall_clock_total_sweep_s`, `new_rows_this_invocation`, `total_rows_in_file`,
`tool`, `comparison_mode`, which are appended by `compare_run.py` after the aggregate computation itself);
`paired_stats_*.json`, `paired_table_*.json`, and `conservative_vs_disabled_stats.json` matched
byte-for-byte (deterministic bootstrap, fixed seed `1066`).

```
# per-arm aggregate (from data/comparison/results_500/<arm>/rows.jsonl)
.venv/bin/python -c "
import sys; sys.path.insert(0, 'scripts')
import compare_aggregate as aggregate
from compare_schema import load_rows
rows = load_rows('data/comparison/results_500/<arm>/rows.jsonl')
print(aggregate.compute_aggregate(rows))
"

# headline paired stats (shared_stock / native)
.venv/bin/python scripts/compare_paired_report.py \
  --renkin-rows data/comparison/results_500/renkin_conservative_shared_stock/rows.jsonl \
  --aizynthfinder-rows data/comparison/results_500/aizynthfinder_shared_stock/rows.jsonl \
  --mode shared_stock \
  --output-stats /tmp/paired_stats_shared_stock.json --output-table /tmp/paired_table_shared_stock.json

.venv/bin/python scripts/compare_paired_report.py \
  --renkin-rows data/comparison/results_500/renkin_conservative_native/rows.jsonl \
  --aizynthfinder-rows data/comparison/results_500/aizynthfinder_native/rows.jsonl \
  --mode native \
  --output-stats /tmp/paired_stats_native.json --output-table /tmp/paired_table_native.json

# per-arm integrity verification
.venv/bin/python scripts/compare_verify_arm.py \
  --rows data/comparison/results_500/<arm>/rows.jsonl \
  --manifest data/comparison/results_500/<arm>/manifest.json \
  --sample-list data/comparison/sample_full_sorted.jsonl --sample-size 500
```

Conservative-vs-Disabled ablation stats have no checked-in generator script (a one-off ad-hoc analysis
reusing `compare_stats.paired_bootstrap_diff`/`mcnemar_exact` directly); see `conservative_vs_disabled.md`
for the exact join and metric definitions used.

## Deliverables index

| File | Contents |
|---|---|
| `aggregate_report.md` | This file |
| `paired_stats_shared_stock.json` / `paired_table_shared_stock.json` | RENKIN vs AiZynthFinder, shared_stock |
| `paired_stats_native.json` / `paired_table_native.json` | RENKIN vs AiZynthFinder, native |
| `conservative_vs_disabled.md` / `conservative_vs_disabled_stats.json` | Ablation-only, RENKIN Conservative vs Disabled |
| `per_target_audit.md` | Cross-arm integrity/sanity audit |
| `run_manifest.json` | Combined per-arm run manifests (binary/commit/Docker/input hashes, host environment) |
| `phase1_shared_stock_report.md` | Phase 1 gate report (interim, superseded by this file for headline numbers) |
| `phase2_interim_report.md` | Phase 2 gate report (interim, superseded by this file for headline numbers) |
| `{arm_name}/rows.jsonl`, `aggregate.json`, `manifest.json` | Per-arm raw data (6 arms) |

Excluded from this PR: per-arm `run.log` (redundant with `rows.jsonl` + `aggregate.json` — stdout/stderr
progress logging only, no unique data), and `artifacts/` (pre-existing, unrelated to this comparison).
