# Implementation-compatibility check: native `--search-mode coverage` CLI vs. the VAL gate's two-phase harness

Required by the user before `RELEASE_CANDIDATE_SHA` may be frozen (see
`protocol.md` §3's flagged implementation decision): the formal-TEST
protocol runs Arm C through the actual shipped `--search-mode coverage`
CLI (Phase 41.18B, PR #120), not the earlier VAL reranker-compatibility
gate's `scripts/compare_run.py` two-phase Stage-1-then-filter-then-Stage-2
orchestration script. This check confirms the two are behaviorally
equivalent before that decision is relied on for the one-shot formal-TEST
run.

**This is a non-TEST, implementation-equivalence check — it introduces
no new efficacy threshold and touches no formal-TEST target**, per the
user's explicit pre-registration. It reuses 9 already-committed VAL
targets from `data/phase_b1_frontier/phase_b2/reranker_gate/`
(`armA_500_rows.jsonl` / `armC_stage2_2000_rows.jsonl`), hand-picked for
fast completion (a handful of each of: Stage-1-solved, Stage-2-solved,
Stage-2-unsolved, all `run_status == "completed"` i.e. no timeout in the
original run) — not a statistical sample, since the question here is
"does the code path change the answer," not "how big is the effect."

## Method

`scripts/verify_coverage_mode_cli_matches_val_gate.py` invokes
`target/release/renkin --search-mode coverage` directly (same
`--depth 5 --max-routes 1 --beam-width 100`, same
`data/comparison/shared_stock/shared_stock.smi`, same
`templates_500.smi`/`templates_2000.smi`, same frozen reranker
model/frequency-table as the original VAL gate) for each of the 9
targets, and checks against the original committed rows:

- **Stage-1 solved/unsolved partition**: the 3 `stage1_solved` targets
  must come back with `selected_stage == "stage1"`; all 6 Stage-2
  targets must come back with `selected_stage == "stage2"`.
- **Semantic selected-route projection**: for every target with a route
  (3 `stage1_solved` + 3 `stage2_solved`), `normalized_route_sha256`
  (`scripts/compare_route_graph.py` — the same tool-agnostic
  canonical-route hash the original rows were scored with) must match
  the original row's stored value exactly.
- **Stage-2 invocation behavior**: `stage2_invoked` must be `false` for
  `stage1_solved` targets, `true` for the other 6.
- **Coverage outcome for non-timeout targets**: `routes_found > 0` must
  match the original row's `route_found` for all 9 (all drawn from
  `run_status == "completed"` rows, i.e. none hit the original run's
  600s Stage-2 timeout).

## Result: PASS, 9/9

Run twice (independently, to rule out a fluke): both runs, all 9/9
targets matched on every one of the four checks above, byte-for-byte on
the route-hash check. Raw console output:
`implementation_compatibility_check_output.txt` (second run; the first
run's result was identical, confirmed by eye before being superseded by
this saved copy).

Checked against branch `feat/v0.24.0-release-candidate`, commit
`3f752ec7d839376f190858660417ce890a3e4b3f` (the commit immediately
before this check's own commit).

**Conclusion**: the native `--search-mode coverage` CLI is confirmed
behaviorally equivalent to the VAL gate's original two-phase
orchestration, for both the Stage-1/Stage-2 partition decision and the
actual route found. The formal-TEST protocol's decision to run Arm C
through the native CLI (`protocol.md` §3) is verified, not just
asserted.
