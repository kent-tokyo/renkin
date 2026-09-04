# v0.24 Coverage Mode — Formal-TEST Post-Anomaly Replacement Confirmation (v2)

**Status: PRE-REGISTERED, FROZEN. Not yet executed.** User-authored
decision, verbatim, 2026-08-16, in direct response to v1's execution
anomaly (below).

**This is NOT a clean pre-registered formal TEST.** v1's Arm A results
have already been observed (102/500 solved, 2/500 external-wrapper
timeouts, p50 7.5s / p95 43.8s / p99 112.0s / max 150.0s — see
`data/coverage_mode_formal_test/results/arm_a_rows.jsonl`,
`arm_a_aggregate.json`, `arm_a_manifest.json`, committed at `341b3f8`).
v2 is explicitly labeled a **post-anomaly replacement confirmation**,
not an independent first run, precisely because that observation
already happened. Everything in this document that is not called out
below as changed is unchanged from `protocol.md` (v1) — cohort, arms,
common configuration (binary, depth/max-routes/beam-width, stock,
reranker, one-run-per-target/no-retry discipline), and every pass/fail
criterion in v1 §4.

## 1. What v1 actually found (and did not find)

v1's Arm A completed all 500 targets in one attempt against frozen
candidate `441c603`. 2 of 500 rows (0.4%) hit
`scripts/compare_run.py`'s **external** wrapper timeout
(`run_status == "timeout"`) at exactly the `--timeout-s 150` boundary:
`uspto50k_test#L4504` (150007.45ms) and `uspto50k_test#L4181`
(150007.96ms). Per v1 protocol.md §5, this is an execution anomaly,
not a result — no PASS/FAIL verdict was computed or is claimed from
that data. Arm C was deliberately never started.

**Important correction, verified against the committed v1
`protocol.md`**: `--timeout-s 150` (and `--grace-s`) does **not**
appear anywhere in `protocol.md`. `protocol.md` §3 fixes depth,
max-routes, beam-width, stock, reranker, and Stage 2's *internal*
cooperative-cancellation deadline (`--coverage-timeout-secs 600`) — it
does not fix an external harness wrapper timeout. `150` first appeared
in the v1 run's own invocation, carried over unreviewed from the
example command in `scripts/coverage_mode_formal_test_gate.py`'s
docstring. This is therefore not "loosening a pre-registered threshold
after seeing results" (which v1 §5 and this whole program's discipline
forbid) — it is correcting an unregistered harness safety cap that
censored a normal Arm A search before any product-level pass/fail
criterion was ever evaluated. v2 changes only that unregistered
parameter.

## 2. What changes for v2

**Unchanged from v1** (re-stated here for a reviewer's convenience,
not re-registered as new values): cohort (same 500 targets,
`cohort_targets_sha256: bf68169e6a1eccfc...`), Arm A/Arm C definitions,
`--depth 5 --max-routes 1 --beam-width 100`, `shared_stock.smi` stock,
reranker `model.txt`/`frequency_table.json`, Stage 2
`--coverage-timeout-secs 600`, one run per arm per target with no
retry/no failed-row rerun, and every §4 pass/fail criterion (coverage
delta ≥ +3.0pp / net gain ≥ +15, regressions = 0, invalid = 0,
reranker_failures = 0, Arm-A-solved ⇒ Arm-C-Stage-1-exact, Stage 2
never invoked when Stage 1 solved, Stage 2 timeout rate ≤ 5%).

**Changed — external harness safety caps only** (`scripts/compare_run.py`
`--timeout-s`/`--grace-s`, the OS-level `/usr/bin/time` wrapper's
kill deadline — an infrastructure boundary, not a product search-mode
timeout):
- Arm A: `--timeout-s 1200 --grace-s 10` (was 150/10 in v1).
- Arm C: `--timeout-s 2400 --grace-s 10` (was undetermined — v1's Arm C
  never ran. `--coverage-timeout-secs 600` s cooperative-cancellation is
  Stage 2's real product deadline; 2400s external gives headroom for a
  potentially long Stage 1 plus the full 600s Stage 2 window plus
  soft-deadline overshoot, without which the same censoring risk that
  hit Arm A could recur, worse, on Arm C given Arm A's own p99 was
  already 112s on a search with no internal deadline at all).
- Reaching either new external cap is **still** an execution anomaly
  under v1 §5, unchanged. This is a larger safety margin, not a removal
  of the safety mechanism.

**Result paths**: `data/coverage_mode_formal_test/results_v2/` (v1's
`results/` directory is untouched, permanently preserved as the
anomaly record).

**Execution discipline, explicit**:
- No `--resume` against v1's results (different directory entirely, so
  this is structural, not just a flag choice).
- All 500 Arm A rows run from scratch; then, strictly after Arm A
  completes, all 500 Arm C rows run from scratch.
- v1's 2 anomalous targets are not treated specially — not excluded,
  not run in isolation, not given a different timeout from the other
  498. All 500 targets in both arms run under the identical v2
  configuration.
- v1's 498 non-anomalous Arm A rows are not reused or merged into v2 —
  v2's Arm A is a complete independent re-run under the new caps, so
  that every row in the eventual gate evaluation ran under one
  consistent set of conditions, not a mix of v1-150s-capped and
  v2-1200s-capped rows.
- No cohort change, no target exclusion.
- No threshold or configuration change after either arm begins.

## 3. Environment preflight (checked immediately before Arm A v2 starts,
   and re-checked immediately before Arm C v2 starts)

- AC power connected (not battery).
- `caffeinate` active (prevents system/display sleep during the run).
- No other `renkin` / `cargo` (build, test, bench) / benchmark /
  training process running.
- 1-minute load average ≤ 2.0.
- ≥ 15 GiB free disk on the volume the repository lives on.
- Environment snapshot captured via the existing harness's own
  `--manifest-path` mechanism (`start_environment`/`end_environment`,
  already implemented in `scripts/compare_run.py` — no new capture
  code needed).
- If any condition is not met, the run does not start; the blocking
  condition is reported and waited on, not worked around.

## 4. Outcome handling

- **v2 external wrapper timeout, or any other infrastructure
  anomaly**: STOP permanently. Do not create a v3 protocol. Do not
  release v0.24.0. Preserve partial results under `results_v2/` and
  report.
- **v2 completes but fails a §4 hard gate**: record FAIL, as-is. Do
  not release. STOP.
- **v2 passes every hard gate**: continue the already-authorized
  v0.24.0 release pipeline without a further confirmation round —
  results commit → release metadata (CHANGELOG/CITATION.cff/README) →
  final CI → merge PR #122 → tag `v0.24.0` → publish
  crates.io/PyPI/npm → GitHub Release + coverage-template asset →
  public post-release verification.

## 5. Out of scope for this recovery

Source/search implementation, the formal cohort, v1's results
(`data/coverage_mode_formal_test/results/`), `fetch_reranker_model.py`
or any existing GitHub Release, and any unrelated worktree/stash/PR
(including PR #104) are not touched by this document or by v2's
execution.

## 6. Last attempt

If v2 also produces an external wrapper timeout despite the enlarged
caps and a clean preflight environment, that implicates RENKIN's
standard search-mode tail latency generally, not coverage mode
specifically — a pre-release investigation item, not something to
route around with a v3 protocol.
