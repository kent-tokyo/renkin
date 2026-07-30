# 100-target feasibility results — Issue #66

Sample: first 100 rows of `data/comparison/sample_full_sorted.jsonl`
(`sample_key_sha256` domain-separated, deterministic; see
`data/comparison/sample_manifest.json`). Hardware: macOS arm64, 10 CPU /
16 GB host, Docker Desktop VM ~7.65 GiB / 10 vCPU. Sequential execution
(one target in flight at a time), no concurrent tool runs.

All numbers below are **descriptive only** (n=100) — no statistical
significance claim is made. See
`docs/guides/open-source-retrosynthesis-comparison.md` for the full
semantic-firewall rules governing how to read every field here.

## RENKIN — native (Arm A / Arm C refreshed baseline)

Config: `renkin` CLI, depth=5, beam-width=100, max-routes=1,
`data/building_blocks.smi` (402 unique), `data/templates_extracted_500.smi`
(500 templates — **not** the historical baseline's 5,000-template file,
which is not vendored in this repo; see "Known gaps"). External per-target
timeout 150s (never triggered). `renkin_native.jsonl` (100 rows),
`renkin_native_aggregate.json`.

| Metric | Value | Denominator |
|---|---|---|
| `route_found_rate` | **21/100 = 0.21** | all_sampled |
| `route_to_configured_stock_rate` (harness-independent all-leaves-in-stock) | 18/100 = 0.18 | all_sampled |
| `route_tree_parseable_rate` | 21/21 = 1.00 | route_found_runs |
| `reaction_steps_parseable_rate` | 21/21 = 1.00 | parseable_routes |
| `mass_conserved_route_rate` (common, stricter per-element check) | 17/100 = 0.17 (17/21 of solved routes) | all_sampled |
| `common_structural_warning_rate` | 7/100 = 0.07 | all_sampled |
| `timeout_rate` / `crash_rate` / `setup_error_rate` / `invalid_input_rate` | 0.0 / 0.0 / 0.0 / 0.0 | all_sampled |
| `total_elapsed_ms` p50 / p95 / max | 4,313 / 26,997 / 31,824 ms | measured_runs (all 100 — no native timeout exists, so every run contributes real elapsed time even when unsolved) |
| `solved_only_total_elapsed_ms` p50 / p95 / max | 496 / 2,903 / 6,840 ms | route_found_runs (the only latency number licensed for cross-tool comparison — see "Latency comparison firewall") |
| `peak_rss_bytes` (`usr_bin_time_v`, exact per-target) p50 / p95 / max | 9.9 MB / 17.1 MB / 18.5 MB | measured_runs |
| `best_route_depth` p50 / max | 3 / 5 | route_found_runs (n=21) |
| `best_route_step_count` p50 / max | 3 / 5 | route_found_runs (n=21) |
| `best_route_leaf_count` p50 / max | 3 / 6 | parseable_routes (n=21) |
| Total sweep wall-clock | 695.75 s (~11.6 min) for 100 sequential targets | — |

Reproduction: `data/comparison/results_100/renkin_native.jsonl` +
`renkin_native_aggregate.json`. Note `mass_conserved_route_rate`
(17/100) is meaningfully lower than `route_found_rate` (21/100) — 4 of
RENKIN's own 21 solved routes fail the harness's stricter, directional,
per-element mass-conservation check even though RENKIN itself reports them
solved. This is expected and by design (see "common_mass_conservation_status"
in the comparison guide) — it is **not** evidence of a RENKIN defect, it is
the harness's independent check being stricter than RENKIN's own internal
MW-based inequality.

## AiZynthFinder — native (Arm A)

Config: `aizynthcli`, official public config (`config.yml`: USPTO+ringbreaker
ONNX expansion policy, USPTO ONNX filter, ZINC HDF5 stock — all
default/undocumented-override search parameters: `iteration_limit=100`,
`time_limit=120`, `algorithm=mcts`, confirmed via direct
`Configuration()` introspection). One container invocation per target,
`--network none`, `--cpus 8 --memory 6g`. External per-target timeout 150s
(never triggered). `aizynthfinder_native.jsonl` (100 rows),
`aizynthfinder_native_aggregate.json`.

| Metric | Value | Denominator |
|---|---|---|
| `route_found_rate` (tool's own `is_solved`) | **66/100 = 0.66** | all_sampled |
| `route_to_configured_stock_rate` | 66/100 = 0.66 | all_sampled (native mode trusts the tool's own per-leaf `in_stock` claim — the ~17.4M-compound ZINC stock is not independently re-canonicalized this round; every row carries an explicit `native_stock_trusted_not_independently_verified` adapter_warning) |
| `route_tree_parseable_rate` | 66/66 = 1.00 | route_found_runs |
| `reaction_steps_parseable_rate` | 66/66 = 1.00 | parseable_routes |
| `mass_conserved_route_rate` | 66/100 = 0.66 (66/66 of solved routes) | all_sampled |
| `common_structural_warning_rate` | 1/100 = 0.01 | all_sampled |
| `timeout_rate` / `crash_rate` / `setup_error_rate` / `invalid_input_rate` | 0.0 / 0.0 / 0.0 / 0.0 | all_sampled |
| `total_elapsed_ms` p50 / p95 / max | 11,876 / 15,635 / 22,314 ms | measured_runs (all 100 — includes per-target container start + ~650MB ZINC stock/model reload every target, see "AiZynthFinder-specific notes") |
| `solved_only_total_elapsed_ms` p50 / p95 / max | 10,994 / 13,755 / 19,309 ms | route_found_runs |
| `peak_rss_bytes` (`docker_stats_sampled`, coarse) p50 / p95 / max | 3.95 GB / 4.38 GB / 4.51 GB | measured_runs |
| `best_route_depth` (harness-derived, see "AiZynthFinder-specific notes") p50 / max | 1 / 6 | route_found_runs (n=66) |
| `best_route_step_count` p50 / max | 1 / 9 | route_found_runs (n=66) |
| `best_route_leaf_count` p50 / max | 2 / 6 | parseable_routes (n=66) |
| Total sweep wall-clock | 1,282.78 s (~21.4 min) for 100 sequential targets | — |

Two real adapter bugs were found and fixed during this arm's smoke gate
before this run — see "Two real bugs this smoke gate caught" in
`data/comparison/smoke_gate_report.md` for the full account (wrong output
envelope shape silently zeroing every result; native-mode stock check
wrongly configured against RENKIN's 402 compounds instead of ZINC).

## AiZynthFinder — matched-stock (Arm B)

Config: identical to native except `stock:` points at
`data/comparison/renkin_bb_402.hdf5` (RENKIN's 402-compound building-block
list, converted via `smiles2stock`, InChIKey round-trip identity confirmed
modulo one disclosed stereo-notation edge case — see
`smoke_gate_report.md`). Expansion/filter policy **unchanged** from native
— per the frozen protocol, Arm B isolates stock coverage only.
`aizynthfinder_matched_stock.jsonl` (100 rows),
`aizynthfinder_matched_stock_aggregate.json`.

| Metric | Value | Denominator |
|---|---|---|
| `route_found_rate` | **4/100 = 0.04** | all_sampled |
| `route_to_configured_stock_rate` (independently re-verified against the real 402-compound set, unlike native mode) | 4/100 = 0.04 | all_sampled |
| `route_tree_parseable_rate` | 4/4 = 1.00 | route_found_runs |
| `reaction_steps_parseable_rate` | 4/4 = 1.00 | parseable_routes |
| `mass_conserved_route_rate` | 4/100 = 0.04 (4/4 of solved routes) | all_sampled |
| `common_structural_warning_rate` | 0/100 = 0.00 | all_sampled |
| `timeout_rate` / `crash_rate` / `setup_error_rate` / `invalid_input_rate` | 0.0 / 0.0 / 0.0 / 0.0 | all_sampled |
| `total_elapsed_ms` p50 / p95 / max | 7,059 / 9,934 / 11,769 ms | measured_runs |
| `solved_only_total_elapsed_ms` p50 / max | 7,100 / 11,769 ms (n=4) | route_found_runs |
| `peak_rss_bytes` p50 / p95 / max | 568 MB / 648 MB / 774 MB | measured_runs (dramatically lower than native's ~4 GB — the 402-compound stock is a few hundred KB vs. ZINC's ~650 MB, confirming the memory difference tracks stock size, not a leak) |
| Total sweep wall-clock | 816.72 s (~13.6 min) for 100 sequential targets | — |

**This is the headline finding of Arm B.** Restricting AiZynthFinder to
*exactly the same* 402-compound stock RENKIN uses natively — while leaving
its USPTO expansion policy and filter model unchanged — collapses its
route-found rate from 66% (native, ~17.4M-compound ZINC) to 4%. This
isolates stock coverage as the dominant factor behind Arm A's native-mode
gap: AiZynthFinder's neural policy proposes disconnections assuming access
to a large purchasable-compound stock, and a small curated stock like
RENKIN's starves it almost entirely, independent of anything about either
tool's underlying search algorithm.

### Paired comparison (RENKIN native vs. AiZynthFinder matched-stock)

Both tools now draw from the *identical* 402-compound stock.
`paired_stats_matched_stock.json`:

| Comparison | Observed (RENKIN − AiZynthFinder matched-stock) | 95% CI |
|---|---|---|
| `route_found_rate` difference | **+0.17** | [+0.10, +0.25] |
| McNemar (discordant pairs) | RENKIN-only solved: 17; AiZynthFinder-only solved: 0; p ≈ 1.5×10⁻⁵ | reference statistic only |

With the stock variable held constant, RENKIN's route-found rate (21%) is
higher than AiZynthFinder's matched-stock rate (4%) on this sample — the
reverse of Arm A's direction. Read together, Arms A and B tell a coherent,
non-contradictory story: AiZynthFinder's native-mode advantage (Arm A) is
attributable in large part to its much larger public stock, not to a
general search-engine advantage — when that variable is removed (Arm B),
RENKIN's own templates/search on this stock outperform AiZynthFinder's
neural policy applied to the same small stock, on this specific sample.
This is still a **descriptive, n=100 finding** about two specific tool
configurations on one target sample, not a general algorithmic claim about
either tool's search engine — see the semantic firewall.

## Paired comparison (RENKIN native vs. AiZynthFinder native)

**Descriptive only (n=100) — this is a comparison of full public
distributions (Arm A), not an engine-only comparison: RENKIN native draws
from a 402-compound curated stock and 500 hand/extracted templates;
AiZynthFinder native draws from a ~17.4-million-compound public ZINC stock
and a trained USPTO neural expansion policy. No claim that either tool's
*search engine* is better is made or supported by this comparison — see the
frozen protocol's Arm A definition and semantic firewall.**

From `scripts/compare_stats.py` (`data/comparison/results_100/paired_stats_native.json`,
`paired_table_native.json`; 10,000 bootstrap iterations, fixed seed 1066):

| Comparison | Observed (RENKIN − AiZynthFinder) | 95% CI |
|---|---|---|
| `route_found_rate` difference | −0.45 | [−0.56, −0.34] |
| McNemar (discordant pairs) | RENKIN-only solved: 3; AiZynthFinder-only solved: 48; p ≈ 2×10⁻¹¹ | reference statistic only, not a substitute for the CI above |
| `total_elapsed_ms` difference, **both-solved pairs only** (n=18) | −9,971 ms | [−10,770, −9,102] |

**Reading this honestly:** under each project's own recommended public
configuration, AiZynthFinder's `is_solved` rate is higher than RENKIN's
`route_found` rate on this 100-target sample, and the gap is large relative
to the width of its bootstrap CI at this sample size. This is a real,
paired, descriptive finding — not fabricated, and not narrated away — but
it is a finding about **two full public distributions**, each combining a
search engine with a very differently-scoped stock and template/policy
source, not a controlled test of either tool's underlying search algorithm.
The 18 targets both tools solved show RENKIN with substantially lower
wall-clock latency per target (its own combinatorial depth×beam search
exhausts quickly relative to AiZynthFinder's ~10-20s per-target,
container-inclusive MCTS budget) — but per the latency comparison firewall,
this solved-only figure is the only latency number licensed for comparison,
and it does not extend to an "N× faster overall" claim. See Arm B
(matched-stock) below for the isolated-stock-coverage view of the same
target sample.

## Interpretation notes

- `total_elapsed_ms` (all-target) reflects fundamentally different search
  budget *kinds* for the two tools (RENKIN: combinatorial depth×beam;
  AiZynthFinder: temporal `time_limit`/`iteration_limit`) — do not read the
  all-target numbers as a speed comparison. `solved_only_total_elapsed_ms`
  is the only comparative latency figure this report licenses.
- `peak_rss_bytes` methods differ (`usr_bin_time_v` for RENKIN, exact;
  `docker_stats_sampled` for AiZynthFinder, coarser and inclusive of
  container overhead + the ~650MB ZINC stock/model load) — never compared
  as equivalent-precision numbers.
- `route_found_rate` (tool-native) and `mass_conserved_route_rate`/
  `route_to_configured_stock_rate` (harness post-hoc) are reported
  side-by-side, never merged, per the semantic firewall.
