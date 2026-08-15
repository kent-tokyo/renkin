# RENKIN Coverage Mode — Design Doc

Status: **draft, design-only — no implementation yet.** Base commit:
`135f62c66b5dcbca7c080c7c959adf0f2178a694` (`research/phase-b1-b2-progressive-escalation`,
PR #118, not yet merged to `origin/master`).

**Gate**: product integration (anything beyond this document) does not
start until the reranker-arm extended determinism replay passes.
**Current status: the replay FAILED as specified** -- 36/37 targets
exact, 1 mismatch (`uspto50k_val#L2330`, a Stage-2 wall-clock timeout-
boundary classification flip, characterized in full in
`ROADMAP.md`/`findings.md`). This is not softened or waived. A
follow-up diagnostic (Phase B.2d, `data/phase_b1_frontier/findings.md`)
targets that one mismatch in isolation to distinguish two different
questions the original gate conflated -- see §4 below, which this
result directly motivated. The original 600s gate's FAIL stands
permanently regardless of the diagnostic's outcome; it is never
retroactively upgraded to a PASS. This document is itself Green/design
work and does not depend on the diagnostic's result.

## 0. What this is, in one paragraph

Phase B.2 (`data/phase_b1_frontier/findings.md`) proved that escalating
only Stage-1-unsolved targets to a larger-template Stage-2 search
converts real candidate-pool coverage gains (Phase A.5) into route
coverage with zero regressions, at an opt-in cost tier (`p95 5.72x`).
This document is the CLI/Python/core-boundary/observability/packaging
design for shipping that as an actual product feature — a `standard`
mode (today's unchanged behavior, still the default) and an opt-in
`coverage` mode (Stage 1 at the normal template set, Stage 2 at a
larger one, only for what Stage 1 didn't solve). No production code
changes are proposed or made in this document — it is a plan to review
before Phase 41.17's implementation begins.

## 1. Existing-code grounding (read before designing, not after)

- `renkin`'s CLI (`src/main.rs`) is hand-rolled `match` arg parsing, no
  `clap` — flags are added as new `"--foo" => { ... }` arms (see
  `--reranker-model`/`--reranker-freq-table`, lines 195–207). Coverage
  mode's flags follow the same shape, not a new parsing framework.
- The core search entry point is a single call:
  `search::find_routes(&target_smiles, &env, &rules, &config)` (line
  456), called exactly once today. Coverage mode calls it **twice**
  from the CLI/Python layer with two different `rules` (template) sets
  — `find_routes` itself, `SearchConfig`, and `search.rs`'s search loop
  are **not modified**. This is a caller-side orchestration feature,
  same layer as `compare_run.py`/`scripts/phase_b2_orchestrator.py`
  were for the benchmark harness, just now inside the product binary
  and Python module instead of a research script.
- Output-JSON backward compatibility already has a proven pattern to
  copy exactly: `Output.search_diagnostics` and `Output.reranker_failures`
  are `Option<T>` fields with `#[serde(skip_serializing_if = "Option::is_none")]`,
  populated via `.then_some(...)` only when the relevant flag was
  actually used (`src/main.rs:24-39, 601-602`). Every new coverage-mode
  output field uses this exact mechanism — omitted, not `null`, when
  `search_mode` is `standard` (the default).
- `renkin` has **no internal wall-clock timeout today** — depth/beam
  width are the only search budget (`SearchConfig` has no time/node
  cutoff; confirmed by `docs/design/synthesizability-kernel-v0.md`
  §3's independent trace of the same fact). Every timeout enforced
  anywhere in this program so far (`compare_run.py`'s `/usr/bin/time`
  wrapper, `scripts/compare_renkin_adapter.py`'s `_run_with_time_wrapper`)
  is **external**, process-level, never inside `find_routes`. This
  matters directly for §1.3 below — `--coverage-timeout-secs` needs a
  real design decision, not an assumption that "timeout" already means
  something inside the engine.
- Artifact distribution has one working precedent to mirror, not
  invent: `scripts/fetch_reranker_model.py` (v0.23.0, Issue #101) —
  GitHub Release asset, two manifests (`freeze_manifest.json` for
  training-time identity, `release_asset_manifest.json` for
  download-authenticity), SHA-256-verified, atomic download
  (temp-file-then-rename), deletes-on-failure, never mutates the
  release itself. §6 proposes reusing this exact shape, not a new one.

## 2. CLI surface

```
--search-mode standard|coverage      default: standard (unchanged behavior)
--coverage-templates <PATH>          required iff --search-mode coverage
--coverage-timeout-secs <N>          optional, Stage 2 only; default: no cutoff (matches today's no-timeout Stage 1 behavior)
```

- `--search-mode` absent or `standard` → **zero behavior change**.
  `--templates` continues to mean exactly what it means today. This is
  the whole backward-compatibility argument in one sentence: coverage
  mode is additive, gated behind an explicit opt-in flag nothing
  reaches by accident.
- `--search-mode coverage` without `--coverage-templates` → **hard CLI
  error at startup** (`bail!`, before any search runs), not a silent
  fallback to standard mode. This is a deliberate divergence from the
  reranker's own precedent (missing/bad reranker config degrades
  gracefully to legacy ordering with a stderr warning) — reasoned, not
  copied blindly: a degraded reranker still returns a correct,
  requested-shape answer (routes, just reordered). Silently downgrading
  `--search-mode coverage` to standard-mode behavior would silently
  under-deliver on an explicit request for more coverage without ever
  telling the caller it didn't happen. Fail loud instead, per the
  user's own instruction and consistent with every fail-loud invariant
  `phase_b2_orchestrator.py` already enforces.
- `--templates` still selects Stage 1's template set (default or
  user-supplied) — coverage mode does not repurpose that flag. Stage 2
  always uses `--coverage-templates`, independently.
- `--coverage-timeout-secs` applies **only to Stage 2**, not Stage 1.
  Rationale: Stage 1 in coverage mode is byte-identical to standard
  mode's search, which has never had an internal timeout — no reason
  to add an asymmetric one now. Stage 2 is the new, expensive,
  opt-in-cost path where an unbounded run is the real risk this flag
  exists to bound (every real timeout Phase B.1/B.2 measured happened
  on a Stage-2-equivalent search, never Stage 1).

## 3. Python surface

```python
renkin.find_routes(
    target,
    search_mode="coverage",             # default: "standard"
    coverage_templates_path="templates_2000.smi",   # required iff search_mode="coverage"
    coverage_timeout_seconds=600,        # optional, Stage 2 only
    reranker_model_path="model.txt",     # unchanged, orthogonal
    reranker_freq_table_path="frequency_table.json",
)
```

Mirrors `src/python.rs`'s existing `#[pyo3(signature = (...))]` pattern
exactly (new kwargs appended with defaults, same file, same
`find_routes_py` function — not a second `find_routes_coverage_py`
function, matching how `reranker_model_path`/`reranker_freq_table_path`
were added to the existing function rather than forking it). Same
fail-loud-on-missing-asset and graceful-reranker-degrade rules as the
CLI, since both surfaces call into the same core orchestration (§4).

## 4. Core orchestration boundary

Lives in `src/main.rs` and `src/python.rs` only — **no new
`search.rs`/`SearchConfig` fields, no change to `find_routes`'s
signature or search loop.** Shape (illustrative, not final code):

```rust
let (routes1, stats1) = search::find_routes(&target, &env, &rules_stage1, &config)?;

let (selected_routes, selected_stats, selected_stage, stage2_invoked, stage2_timeout) =
    if search_mode == SearchMode::Standard || !routes1.is_empty() {
        (routes1, stats1, Stage::Stage1, false, false)
    } else {
        let rules_stage2 = load_rules_from_file(&coverage_templates_path); // fail-loud if unreadable
        match run_stage2_with_timeout(&target, &env, &rules_stage2, &config, coverage_timeout_secs) {
            Ok((routes2, stats2)) => (routes2, stats2, Stage::Stage2, true, false),
            Timeout => (vec![], stats1 /* or a synthetic empty-stats */, Stage::Stage2, true, true),
        }
    };
```

**The core invariant — Stage 1's valid route is never overwritten —
holds by construction, not by a priority rule applied after the fact**:
Stage 2 only ever runs when `routes1.is_empty()`. There is no merge
step and no "which one wins" decision, exactly the same disjoint-by-
construction property `phase_b2_orchestrator.merge_arm` enforces for
the benchmark harness (`data/phase_b1_frontier/findings.md`, "Phase
B.2" section) — the product code gets the same guarantee for free from
the same control-flow shape, not from re-deriving the rule.

**Stage 2 is a fully independent search call** — same as the
pre-registered Phase B.2 constraint: no warm-start, no candidate reuse
between stages, `find_routes` called fresh with `rules_stage2`.

**Timeout mechanism: cooperative cancellation, decided.** An earlier
draft of this section recommended spawning Stage 2's `find_routes`
call on a thread and racing it against `coverage_timeout_secs` via
`mpsc::Receiver::recv_timeout`, leaving the thread detached (not
joined) and still computing in the background on timeout. **Rejected.**
A single CLI invocation, that's a one-time harmless leak. Coverage
mode's own measured Stage-2 invocation rate is 83.5% (`findings.md`,
Phase B.2) — most invocations escalate, so a long-lived caller (the
Python module in a notebook loop, a server process, `renkin-mcp`)
making repeated coverage-mode calls would accumulate detached,
still-running search threads on every Stage-2 timeout. That is not an
acceptable v0 property for a mode designed to be invoked this often,
not an edge case to defer.

**Decided instead: give `find_routes`'s main loop an optional
deadline, checked periodically, that actually stops the search and
returns whatever routes/stats it has so far when the deadline passes.**
This does touch `search.rs` — a real change to the core search
algorithm this document originally tried to avoid — but it is the
correct product boundary for a mode whose whole value proposition is
"call Stage 2 often." Shape (illustrative):

```rust
pub struct SearchConfig {
    // ...existing fields...
    pub deadline: Option<std::time::Instant>,  // None = no cutoff (today's behavior, unchanged)
}

// inside find_routes's main frontier loop, checked once per iteration
// (not per-candidate -- the loop iteration is already the natural,
// cheap checkpoint; this is not a hot-path cost):
if let Some(deadline) = config.deadline
    && std::time::Instant::now() >= deadline
{
    stats.timed_out = true;
    break;
}
```

`config.deadline: None` (the default, matching every existing caller
including standard mode and Stage 1) is **exactly today's behavior,
byte-for-byte** — this is additive, not a change to any existing
search's semantics. Coverage mode's Stage 2 is the only caller that
ever sets it, computed from `coverage_timeout_secs` at call time.

**Why this is not just "the same as the old approach but tidier"**:
cooperative cancellation gives up the same wall-clock non-guarantee
option 1 would have had (a deadline check once per loop iteration can
still overshoot by however long one iteration takes), but it never
leaves work running after the caller stops waiting for it — the
in-process equivalent of the external wrapper's `SIGTERM`, done
cleanly instead of by killing a subprocess.

### The two-layer guarantee this design actually offers

Phase B.2d's diagnostic (see §0's Gate note and
`data/phase_b1_frontier/findings.md`) exists because the original
600s-gate mismatch conflated two genuinely different properties that
this design keeps separate, in both the implementation and the product
contract documentation:

1. **Algorithmic semantic determinism** — same target, same stock,
   same templates, same config, **sufficient budget to actually
   complete**: the result (`route_found`, canonical route/tree,
   validator outcome, `reranker_failures`) is deterministic. This is
   what `find_routes` itself guarantees and what every dedicated
   determinism test in this codebase (`reranker_some_is_also_fully_
   deterministic_across_repeated_runs`, the base-architecture 37/37
   Phase B.2 determinism gate) actually verifies.
2. **Operational timeout classification near the deadline** — whether
   a given invocation completes or gets cut off by `coverage_timeout_secs`
   is a wall-clock race against real elapsed time, and elapsed time for
   a fixed amount of work varies with system load. Near the deadline,
   two runs of the identical search can land on opposite sides of the
   cutoff. **This is expected, not a bug, and coverage mode's product
   contract says so explicitly** rather than implying every observable
   result is deterministic.

The original 600s gate's FAIL is not softened by this distinction —
that gate is specified as a single check, mixing both properties by
construction (comparing `run_status`/`is_timeout`, which are downstream
of wall-clock classification, not just semantic outcome), and its
result stands. The distinction matters for what the *product* promises
callers, which is a separate question from what that one research gate
measured.

**Not proposed for v0**: a deterministic search budget
(`max_nodes`/`max_expansions` instead of wall-clock) would let identical
inputs produce identical timeout/no-timeout classification too, closing
this gap at the algorithm level rather than accepting it as a product
contract nuance. Real option, bigger change, not required to ship
coverage mode's actual value (converting candidate-pool coverage into
route coverage) — a candidate for a later iteration if operational
timeout variability turns out to matter more in practice than this
design currently expects.

## 5. Observability (output JSON — CLI and Python, same shape)

All fields below are `Option<T>` with `skip_serializing_if`, `None`
(hence omitted) whenever `search_mode` is `standard` — **existing
standard-mode consumers see byte-identical JSON, unconditionally.**

| field | type | present when |
|---|---|---|
| `search_mode` | `"standard"` \| `"coverage"` | only when `coverage` (never emitted for the default) |
| `selected_stage` | `"stage1"` \| `"stage2"` | coverage mode only |
| `stage2_invoked` | `bool` | coverage mode only |
| `stage1_outcome` | `"solved"` \| `"unsolved"` | coverage mode only |
| `stage1_elapsed_ms` / `stage2_elapsed_ms` | `f64` | coverage mode only (new capability — `renkin`'s CLI has never self-reported elapsed time; measured today only by external wrappers like `compare_renkin_adapter.py`'s `/usr/bin/time`. `std::time::Instant`, already used the same way in `src/bin/pool_gen.rs`'s per-target timing) |
| `total_elapsed_ms` | `f64` | coverage mode only |
| `stage1_timeout` / `stage2_timeout` | `bool` | coverage mode only |
| `reranker_failures` | `u64` | unchanged existing contract — present whenever a reranker is configured, **independent of `search_mode`** |

**`reranker_failures` in coverage mode is the sum across every stage
that actually ran** (Stage 1 alone, or Stage 1 + Stage 2), not just the
selected/winning stage's count. This is a deliberate, and worth
flagging, difference from how `phase_b2_orchestrator.semantic_projection`
derives it for the *research* determinism check (selected-stage only,
because that check compares the winning outcome across two runs). The
product's audit use case is different: a caller asking "was the
reranker healthy for everything this invocation actually computed"
needs the total, not just the part that happened to win — a Stage-1
reranker hiccup on an eventually-Stage-2-solved target is real signal,
not noise to discard.

## 6. Backward compatibility

- `standard` mode is the default with no flag needed — identical to
  today in every respect: same CLI invocation shape, same output JSON
  byte-for-byte (verified by §8's regression test), same Python
  function signature with all new kwargs defaulted off.
- Coverage-asset resolution failure (`--coverage-templates` path
  missing/unreadable) is **fail-loud** (§2) — never a silent
  degrade-to-standard.
- The reranker's own existing config-error contract (missing one of
  the two paths, or a load failure → graceful degrade to legacy
  ordering with a stderr warning, never a hard error) is **unchanged**
  and **orthogonal** — coverage mode and the reranker compose
  independently; neither flag's error-handling policy leaks into the
  other's.

## 7. Artifact distribution

**Options considered** (per the user's explicit request to compare,
not just pick):

| option | pro | con |
|---|---|---|
| User-supplied path only (no fetch tooling) | Zero new infra | Ordinary users can't actually run coverage mode — the exact gap this design exists to close (v0.23's reranker had this problem before `fetch_reranker_model.py` fixed it) |
| Bundle the templates file in the crate/wheel/npm package | Works with zero extra steps | `templates_2000.smi` is 187 KB (`data/phase_a5_template_scaling/templates/templates_2000.smi`) — not huge, but this is a research/provenance artifact still under active measurement (Phase B.2), and bundling now means every package rebuild ships it whether or not it's actually validated for general release; also raises the same upstream-data-licensing question the reranker's `model.txt` was deliberately kept **out** of packages for |
| **Versioned GitHub Release asset + SHA-256-verified fetch script (recommended)** | Directly mirrors the one artifact-distribution mechanism this repo has already shipped and proven (`fetch_reranker_model.py`, v0.23.0) — same manifest-pinning, same atomic-download-and-verify shape, same "not bundled, not silently trusted" posture | One more script to maintain; requires an actual Release upload step before v0.24 ships (not done by this document — see below) |

**Recommendation: versioned GitHub Release asset, `scripts/fetch_coverage_templates.py`
built as a near-direct structural copy of `scripts/fetch_reranker_model.py`**
— one manifest (`data/phase_a5_template_scaling/coverage_templates_release_asset_manifest.json`
or similar, exact naming TBD at implementation time) pinning
`release_tag` + whole-file SHA-256, `--version` defaulting to the
manifest's own pinned tag (not `Cargo.toml`'s version, same reasoning
as the reranker script's docstring), atomic temp-file-then-rename
download, delete-and-raise on any check failure.

**Not done by this document, explicitly deferred**:
- No asset has been uploaded to any Release yet.
- License/provenance documentation for `templates_2000.smi` is the
  user's own action item (stated explicitly), not resolved here.
- **Packaging gap worth flagging now, before it becomes a silent
  problem**: `Cargo.toml`'s `exclude` list only excludes
  `data/templates_extracted*.smi` — a glob that does **not** match
  `data/phase_a5_template_scaling/templates/templates_2000.smi`. If
  that file is git-tracked (it is, per Phase A.5), it is **currently
  includable in a `cargo package`/crates.io publish** unless
  separately excluded. Whichever artifact-distribution option ships,
  `Cargo.toml`'s `exclude` needs a new entry for
  `data/phase_a5_template_scaling/**` (or the specific file) so
  coverage mode doesn't end up simultaneously "not bundled" (per this
  design) and "accidentally bundled anyway" (per a stale exclude
  list) — a one-line fix, called out here so it isn't missed at
  implementation time.

## 8. Test plan

Mirrors the six invariants `scripts/tests/test_phase_b2_orchestrator.py`
already enforces for the benchmark harness — now as real Rust product
tests, not just research-script coverage:

- `coverage_mode_standard_output_is_byte_identical_to_pre_coverage_mode` —
  regression test: `--search-mode` omitted produces JSON with no
  coverage-mode keys present at all (not `null` — literally absent),
  same shape as before this feature existed.
- `coverage_mode_stage1_solved_never_invokes_stage2` — Stage 1 route
  found → `stage2_invoked: false`, `selected_stage: "stage1"`, no
  Stage-2 template file even opened (assert via a template path that,
  if read, would panic/error — proves it's genuinely never touched).
- `coverage_mode_stage1_unsolved_invokes_stage2_with_coverage_templates` —
  Stage 1 empty → Stage 2 runs with exactly `--coverage-templates`'s
  rule set, `stage2_invoked: true`.
- `coverage_mode_missing_coverage_templates_path_fails_loud` — CLI/Python
  both: `--search-mode coverage` without `--coverage-templates` (or
  with a nonexistent path) errors before any search runs, never
  silently falls back to standard mode.
- `coverage_mode_stage2_timeout_reports_stage1_result_if_any` and
  `coverage_mode_stage2_timeout_flag_set` — the cooperative-cancellation
  deadline (§4) actually stops Stage 2's search loop and surfaces
  `stage2_timeout: true`, not just eventually returning past
  `coverage_timeout_secs`.
- `find_routes_deadline_none_is_byte_identical_to_pre_deadline_behavior`
  — `SearchConfig::deadline: None` (every existing caller, standard
  mode, Stage 1) produces identical output to before this field
  existed; regression-guards §4's core backward-compatibility claim at
  the `search.rs` level, not just at the CLI/Python output-JSON level
  (§6/§8's other tests).
- `find_routes_deadline_stops_search_and_returns_partial_stats` — a
  deadline in the past (or one that expires mid-search on a
  deliberately slow synthetic case) causes the main loop to `break`
  rather than run to natural completion, with `stats.timed_out: true`.
- `coverage_mode_reranker_failures_is_sum_across_invoked_stages` —
  Stage 1 + Stage 2 both reranker-active, both report nonzero via a
  test double/fault injection → top-level field equals the sum, not
  either stage alone.
- `coverage_mode_python_signature_defaults_match_cli_defaults` — a
  Python-side smoke test (same shape as
  `scripts/tests/test_compare_renkin_adapter.py`'s existing
  `requires_renkin_bin`-gated tests) confirming
  `search_mode="standard"` (the Python default) and CLI-with-no-flag
  produce the same output shape on the same target.
- CLI integration smoke: one end-to-end `--search-mode coverage` run
  on a known Stage-1-unsolved-at-500 / Stage-2-solved-at-2000 target
  (reuse one of Phase B.2's own newly-solved targets from
  `data/phase_b1_frontier/phase_b2/decision/` as the fixture — already
  known-good, no new fixture data needed).

No formal-TEST-split test is proposed here — matches this whole
program's VAL-only discipline; the one pre-registered formal-TEST
confirmation run happens once, later, per the v0.24 sequencing in
`ROADMAP.md`, not as part of unit/integration test coverage.

## 9. Explicitly out of scope for this document

- Any actual code change (this is Phase 41.17 — design only).
- Uploading the coverage-templates Release asset.
- Deciding `templates_2000.smi`'s final license/provenance text.
- Retraining the reranker on the 2,000-template candidate distribution
  (noted as a possible future idea in the reranker compatibility gate
  writeup, not something this design depends on or proposes).
- A formal-TEST confirmation run (comes after implementation, per the
  v0.24 sequencing).
