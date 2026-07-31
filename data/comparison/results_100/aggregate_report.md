# 100-target feasibility results — Issue #66

Sample: first 100 rows of `data/comparison/sample_full_sorted.jsonl`
(`sample_key_sha256` domain-separated, deterministic; see
`data/comparison/sample_manifest.json`). This is the 100-target prefix of
the **4,903-target cross-tool corpus** (canonically deduped from the raw
4,907-row `data/uspto50k_test.smi`) — a different denominator from RENKIN's
historical 986/756/43-out-of-**4,907** "corrected baseline"; the two must
never be compared directly (see "Known gaps" /
"500/full run gate" in the comparison guide).

Hardware: macOS arm64, 10 CPU / 16 GB host, Docker Desktop VM ~7.65 GiB / 10
vCPU, shared (non-dedicated) machine. Sequential execution (one target in
flight at a time, one tool's full sweep to completion before the other
starts) for every arm reported here.

All numbers below are **descriptive only** (n=100) — no statistical
significance claim is made. Every arm below is a **single run per tool**;
AiZynthFinder's repeat-run variance (no documented seed control) has not
been characterized this round — see "Outstanding: repeatability (item D)"
at the end of this report. See
`docs/guides/open-source-retrosynthesis-comparison.md` for the full
semantic-firewall rules governing how to read every field here.

## RENKIN — native (Arm A / Arm C vendored-500 configuration)

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
| `target_elements_accounted_route_rate` (common, directional per-element check — NOT exact mass conservation) | 17/100 = 0.17 (17/21 of solved routes) | all_sampled |
| `common_structural_warning_rate` | 7/100 = 0.07 | all_sampled |
| `timeout_rate` / `crash_rate` / `setup_error_rate` / `invalid_input_rate` | 0.0 / 0.0 / 0.0 / 0.0 | all_sampled |
| `total_elapsed_ms` p50 / p95 / max | 4,313 / 26,997 / 31,824 ms | measured_runs (all 100 — no native timeout exists, so every run contributes real elapsed time even when unsolved) |
| `solved_only_total_elapsed_ms` p50 / p95 / max | 496 / 2,903 / 6,840 ms | route_found_runs — deployment-cost figure only, see "Latency firewall" note below |
| `peak_rss_bytes` (`usr_bin_time_v`, exact per-target) p50 / p95 / max | 9.9 MB / 17.1 MB / 18.5 MB | measured_runs |
| `best_route_depth` p50 / max | 3 / 5 | route_found_runs (n=21) |
| `best_route_step_count` p50 / max | 3 / 5 | route_found_runs (n=21) |
| `best_route_leaf_count` p50 / max | 3 / 6 | parseable_routes (n=21) |
| Total sweep wall-clock | 695.75 s (~11.6 min) for 100 sequential targets | — |

This same native RENKIN configuration is also this round's Arm C
measurement (vendored-500-template configuration on current `master`). It
is **not** a refresh of the historical "corrected baseline" (986/756/43 out
of 4,907, frozen to commit `e20dc8c`, a larger, differently-sourced
5,000-template configuration) — the two are separate measurements under
different template corpora and must never be compared as if one refreshed
the other.

Reproduction: `data/comparison/results_100/renkin_native.jsonl` +
`renkin_native_aggregate.json`. Note `target_elements_accounted_route_rate`
(17/100) is meaningfully lower than `route_found_rate` (21/100) — 4 of
RENKIN's own 21 solved routes fail the harness's stricter, directional,
per-element accounting check even though RENKIN itself reports them
solved. This is expected and by design (see `target_element_accounting_status`
in the comparison guide) — it is **not**, by itself, evidence of a RENKIN
defect: the harness's check is stricter than RENKIN's own internal
MW-based inequality. A per-target audit of these exact 4 failures (plus the
3 stock-check failures) with named root causes is in
`data/comparison/results_100/per_target_audit.md` — 3 of the 4 trace to a
disclosed, systemic property of RENKIN's handcrafted protecting-group
templates (they don't model the second reagent), and 1 to a specific
extracted-template quality issue flagged for follow-up.

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
| `target_elements_accounted_route_rate` | 66/100 = 0.66 (66/66 of solved routes) | all_sampled |
| `common_structural_warning_rate` | 1/100 = 0.01 | all_sampled |
| `timeout_rate` / `crash_rate` / `setup_error_rate` / `invalid_input_rate` | 0.0 / 0.0 / 0.0 / 0.0 | all_sampled |
| `total_elapsed_ms` p50 / p95 / max | 11,876 / 15,635 / 22,314 ms | measured_runs (all 100 — cold-start: includes per-target container start + ~650MB ZINC stock/model reload every target, see "AiZynthFinder-specific notes") |
| `solved_only_total_elapsed_ms` p50 / p95 / max | 10,994 / 13,755 / 19,309 ms | route_found_runs — deployment-cost figure only, see "Latency firewall" note below |
| `peak_rss_bytes` (`docker_stats_sampled`, coarse) p50 / p95 / max | 3.95 GB / 4.38 GB / 4.51 GB | measured_runs |
| `best_route_depth` (harness-derived, see "AiZynthFinder-specific notes") p50 / max | 1 / 6 | route_found_runs (n=66) |
| `best_route_step_count` p50 / max | 1 / 9 | route_found_runs (n=66) |
| `best_route_leaf_count` p50 / max | 2 / 6 | parseable_routes (n=66) |
| Total sweep wall-clock | 1,282.78 s (~21.4 min) for 100 sequential targets | — |

Two real adapter bugs were found and fixed during this arm's smoke gate
before this run — see "Two real bugs this smoke gate caught" in
`data/comparison/smoke_gate_report.md` for the full account (wrong output
envelope shape silently zeroing every result; native-mode stock check
wrongly configured against RENKIN's 402 compounds instead of ZINC). A
stratified sample of 10 of these 66 `accounted` routes, plus manual
reconstruction of 3 of the deepest ones, confirmed the step-extraction
logic genuinely evaluates every step, not just the first/last — see
`per_target_audit.md`, Part 2.

## Shared-stock arm (Arm B) — RENKIN and AiZynthFinder on an identical, zero-diff stock

**This supersedes the earlier "matched-stock" arm.** The prior conversion
(via `smiles2stock`) left `roundtrip_identity_confirmed=false` (9 conversion
failures, 1 stereo-layer mismatch) and was only accepted with a "modulo
stereo layer" exception — not an acceptable basis for a shared-stock claim.
The current construction (`scripts/compare_shared_stock.py`) parses
`data/building_blocks.smi` directly with RDKit and writes the resulting
InChIKey table straight to HDF5, bypassing `smiles2stock` entirely.
**`roundtrip_identity_confirmed=true`, zero missing/extra keys, no
exception needed** — see the comparison guide's "Provenance" section and
`data/comparison/shared_stock/shared_stock_manifest.json`. 393 unique
compounds (402 raw minus 9 RDKit-unparseable entries — a file-content
defect, not a chemistry limitation).

**Framing: this arm does not isolate search-engine quality.** Policy
calibration, search budgets, template/model sources, and internal
stock-matching semantics (RENKIN's VF2 subgraph-isomorphism fallback vs.
AiZynthFinder's exact InChIKey lookup — see `per_target_audit.md`) remain
different between the two tools even on an identical stock. The **primary
metric** for this arm is the common, independently-verified
`route_to_shared_stock` rate (`route_found AND route_tree_parseable AND
all_leaves_in_configured_stock`, checked identically for both tools) — a
tool-native "solved" route that fails this independent check never counts
toward the primary numerator. Tool-native `route_found` is reported
alongside as a secondary/informational field only.

**This arm is a single run per tool.** AiZynthFinder's repeat-run variance
has not been characterized (item D, outstanding — see the end of this
report), so this single-run result must not be treated as a stable,
repeatable finding; it is reported as exactly what it is, a descriptive
result from one run each.

Config: identical to native for both tools except the stock (`renkin`:
`data/comparison/shared_stock/shared_stock.smi` via `--building-blocks`;
`aizynthcli`: `config_shared_stock.yml`, `stock: shared:
/public/shared_stock.hdf5`). Expansion/filter policy **unchanged** from
native for AiZynthFinder; templates **unchanged** for RENKIN.
`renkin_shared_stock.jsonl` / `aizynthfinder_shared_stock.jsonl` (100 rows
each).

| Metric | RENKIN shared-stock | AiZynthFinder shared-stock | Denominator |
|---|---|---|---|
| `route_found_rate` (tool-native, secondary) | 21/100 = 0.21 | 4/100 = 0.04 | all_sampled |
| **`route_to_shared_stock` rate (primary)** | **18/100 = 0.18** | **4/100 = 0.04** | all_sampled — independently re-verified against the real 393-compound shared stock for both tools |
| `route_tree_parseable_rate` | 21/21 = 1.00 | 4/4 = 1.00 | route_found_runs |
| `reaction_steps_parseable_rate` | 21/21 = 1.00 | 4/4 = 1.00 | parseable_routes |
| `target_elements_accounted_route_rate` | 17/100 = 0.17 | 4/100 = 0.04 | all_sampled |
| `common_structural_warning_rate` | 7/100 = 0.07 | 0/100 = 0.00 | all_sampled |
| `timeout_rate` / `crash_rate` / `setup_error_rate` / `invalid_input_rate` | all 0.0 | all 0.0 | all_sampled |
| `total_elapsed_ms` p50 / p95 / max | 6,552 / 44,817 / 58,306 ms | 14,486 / 20,431 / 22,880 ms | measured_runs — deployment-cost figures only, not a cross-tool comparison, see "Latency firewall" |
| `peak_rss_bytes` p50 / p95 / max | 9.5 MB / 16.3 MB / 17.6 MB | 562 MB / 641 MB / 831 MB | measured_runs (AiZynthFinder's is dramatically lower than its native arm's ~4 GB — the 393-compound stock is a few hundred KB vs. ZINC's ~650 MB, tracking stock size, not a leak) |
| Total sweep wall-clock | 1,143.09 s (~19.1 min) | 1,584.62 s (~26.4 min) | shared, non-dedicated hardware — see note below |

RENKIN's shared-stock numbers (21/18/17) are **identical** to its native-arm
numbers — expected, since the shared stock (393 compounds) differs from
RENKIN's native stock (402 compounds) only by the 9 entries RDKit itself
cannot parse, none of which this 100-target sample's solved routes needed
as a leaf. This is a useful consistency check: switching RENKIN onto the
shared stock did not silently change its behavior.

RENKIN's shared-stock total sweep wall-clock (1,143s) is noticeably higher
than its native arm's (696s) despite an uncontended, sequential run; this
machine is explicitly shared/non-dedicated hardware (see "Hardware and run
conditions" in the comparison guide), and an earlier attempt at this same
run was contaminated by a real methodological mistake — RENKIN and
AiZynthFinder were briefly launched concurrently, violating the frozen
protocol's sequential-execution condition, and were re-run cleanly,
sequentially, after the mistake was caught. The route-found/accounting
results themselves are unaffected (RENKIN's search budget is combinatorial,
not wall-clock-bound), but the absolute latency numbers above should be
read as order-of-magnitude descriptive figures on shared hardware, not a
precision instrument.

### Paired comparison (RENKIN shared-stock vs. AiZynthFinder shared-stock)

Both tools now draw from the *identical, zero-diff* 393-compound stock.
`paired_stats_shared_stock.json` (10,000 bootstrap iterations, fixed seed
1066):

| Comparison | Observed (RENKIN − AiZynthFinder) | 95% CI | McNemar (RENKIN-only / AiZynthFinder-only) |
|---|---|---|---|
| **`route_to_shared_stock` rate difference (primary)** | **+0.14** | [+0.07, +0.22] | 15 / 1; p ≈ 5.2×10⁻⁴ |
| Tool-native `route_found` rate difference (secondary, informational) | +0.17 | [+0.10, +0.25] | 17 / 0; p ≈ 1.5×10⁻⁵ |

AiZynthFinder's result on this sample was highly sensitive to the
configured stock: its native-mode `route_found_rate` (66%, ~17.4M-compound
ZINC) collapses to 4% on the shared 393-compound stock, while its expansion
policy and filter model are unchanged. This shared-stock arm does not
isolate search-engine quality (see "Framing" above) — policy calibration,
search budgets, and internal stock-matching semantics still differ between
the two tools, so this is not a controlled test of either tool's search
algorithm. It is a **descriptive, single-run, n=100 finding** about two
specific tool configurations on one target sample and one stock, pending
the repeat-run characterization in item D.

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
AiZynthFinder's result on this sample was highly sensitive to the
configured stock (see the shared-stock arm above): restricting it to the
same 393-compound stock RENKIN uses collapses its solve rate from 66% to
4%, the reverse direction from this native-mode comparison. The 18 targets
both tools solved show RENKIN with substantially lower wall-clock latency
per target — but this is reported only as a disclosed deployment-cost
figure, not a licensed inference-latency comparison (see "Latency
firewall" below).

## Latency firewall

No cross-tool inference-latency comparison is made anywhere in this
report. Every AiZynthFinder measurement is a **cold-start** per-target
Docker container invocation (container startup plus policy-model/stock
load on every single target) — there is no persistent-worker/warm-latency
arm this round (item C, outstanding — see below) to separate
`initialization_ms` from per-target `planning_ms`. All `total_elapsed_ms`
and `solved_only_total_elapsed_ms` figures in this report are disclosed,
per-tool **deployment-cost** numbers only — never a comparative
inference-latency claim, and never described as licensed for direct
comparison.

## Interpretation notes

- `total_elapsed_ms` (all-target) reflects fundamentally different search
  budget *kinds* for the two tools (RENKIN: combinatorial depth×beam;
  AiZynthFinder: temporal `time_limit`/`iteration_limit`) — do not read the
  all-target numbers as a speed comparison.
- `peak_rss_bytes` methods differ (`usr_bin_time_v` for RENKIN, exact;
  `docker_stats_sampled` for AiZynthFinder, coarser and inclusive of
  container overhead + stock/model load) — never compared as
  equivalent-precision numbers.
- `route_found_rate` (tool-native) and `target_elements_accounted_route_rate`/
  `route_to_configured_stock_rate` (harness post-hoc) are reported
  side-by-side, never merged, per the semantic firewall.
- The 4,903-target cross-tool corpus this 100-target sample is drawn from
  is never compared directly against RENKIN's historical 4,907-row
  "corrected baseline" — see the header note above.

## Outstanding: warm-latency arm (item C)

Not attempted this round. AiZynthFinder's measurements above are cold-start
only; a genuine warm-latency arm (persistent worker separating
`initialization_ms`, `planning_ms`, `cold_start_ms`, and throughput, for
both tools) is deferred to a future round. The fallback taken instead: the
cross-tool inference-latency comparison is removed from every section
above, replaced by disclosed per-tool deployment-cost figures (see
"Latency firewall").

## Outstanding: repeatability (item D)

Not attempted this round. AiZynthFinder has no documented seed-fixing
mechanism, and both the native and shared-stock AiZynthFinder arms above
are **single runs**. Per-target solved frequency across repeated runs,
union/intersection route-found rate, target flip rate, route-hash
stability, and latency distribution across repetitions have not been
characterized. RENKIN's own repeatability (expected deterministic, given no
RNG/seed) was confirmed once during the original 5-target smoke gate
(byte-identical `run_status`/`normalized_route_sha256` across two runs of
the same target — see `smoke_gate_report.md`) but has not been reconfirmed
at n=100 this round. Before any 500- or 4,903-target round, or before
treating the shared-stock arm as a formally established, stable
comparison, this repeatability characterization (at least 3 independent
AiZynthFinder runs per mode, at least 2 RENKIN runs per mode) must be
completed — see the comparison guide's "500/full run gate".
