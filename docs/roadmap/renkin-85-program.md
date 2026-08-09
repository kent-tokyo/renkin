# RENKIN 85-point program — Phase 0 ground-truth audit and sequencing plan

Status: **Phase 0 only — audit and planning, no Phase 1–7 implementation in
this document's PR.** Base commit: `37b168e` (`origin/master`, v0.21.0,
tagged and released).

This document exists so every later phase of the program below is sequenced
against what the repository actually contains today, not against what an
instruction or a stale memory assumes it contains. Every claim below was
verified directly against the real commit, PR, issue, and source-file state
at the time of writing — see the citations inline.

> **2026-08-09 update (rebased onto current `master`, no unresolved
> content):** substantial work has landed since this audit was written
> against `37b168e`, changing the status of two of the phases below. This
> note summarizes what changed; the rest of the document below is preserved
> as-written for historical context and is **not** re-verified line-by-line
> against current state — treat §2/§4's per-phase claims as of 2026-08-05,
> not current.
>
> - **Phase 2's 217-template gap is resolved**, not just diagnosed. It turned
>   out to be the same defect Issue #88/PR #89/#91 fixed: `[#N]`/`[#N:map]`
>   bare atomic-number SMARTS primitives failing at *apply* time (not the
>   `[N;H1,H2:2]` multi-condition class this document guessed at in §4 —
>   confirmed empirically, that class doesn't actually occur in this corpus).
>   `crates/renkin-forward`'s `reverse_smirks_validated_extracted_templates_accept_reject_partition_is_stable`
>   test now audits `forward_smirks_variants` instead of a raw
>   `reverse_smirks_validated` call, and the historical 217-rejected baseline
>   no longer applies. See PR #89/#91 and CHANGELOG `[Unreleased]`/prior
>   entries for the fix and its measured impact (including a real, disclosed
>   beam-budget crowd-out side effect — see Issue #101 below).
> - **Phase 3's design conflict is resolved, and the fix has already landed
>   and been measured**, upgrading it from "blocked" to "implemented,
>   opt-in, formally measured, default unchanged." PR #82 merged the
>   match-level `RingContextConfig` guard this section anticipated;
>   `--ring-context-policy conservative` was **RENKIN's official/headline
>   configuration** for the Issue #66 500-target comparison (not an
>   ablation-only arm), and a Conservative-vs-Disabled ablation within that
>   round found no statistically significant `route_to_configured_stock`
>   cost (−0.8pt, 95% CI [−1.6, −0.2], p=0.125, n=4 discordant pairs). Full
>   detail in the 2026-08-09 comment on Issue #72. `Disabled` remains the
>   compiled-in default (unchanged legacy behavior) — whether to flip that
>   default is a policy call this program hasn't made yet.
> - **Issue #66's formal 500-target comparison (§4 Phase 4's blocker) has
>   completed and is published** (v0.21.0, tag `issue66-500-base-e479b27`,
>   `data/comparison/results_500/`) — this is the "4,903-target full
>   comparison" this document lists as Phase 4, run instead at 500-target
>   scale first per the program's own "measurement PR" discipline. The
>   4,903-target full-corpus round remains not started.
> - **Open PR/issue counts below (§2) are stale.** #68 and #56 are still
>   open and still the two PRs this document names; #87 (this document's own
>   PR) has been rebased and is otherwise unchanged. Several new issues
>   opened since (#98, #99, #100, #101 — split off Issue #88's closure;
>   #101 specifically is a beam-width search crowd-out effect from the
>   Phase-2-adjacent fix above, with its own diagnostics-only PR #102).
>   #77 remains open and unaddressed.
> - Phase 1 (large matched stock), Phase 5 (retro reranker gate), Phase 6
>   (Synth Kernel CLI surface), and Phase 7 are unchanged from this
>   document's original assessment — still not started.

## 1. Win condition

RENKIN's target is not feature-parity with AiZynthFinder's route coverage or
with ASKCOS's bundled conditions/yield/model stack. The win condition is:

> Approach AiZynthFinder-class route coverage while keeping a clear,
> demonstrable lead in lightweight execution, determinism, auditability,
> reproducibility, local/offline execution, and embeddability across
> Rust/Python/WASM.

Conditions, yields, side-reactions, and search infrastructure are added only
where real evidence or a validated model backs them — never as
"looks-complete" surface area.

## 2. Ground-truth snapshot

| Item | Value |
|---|---|
| `origin/master` | `37b168e27c6ea289092a737e161b58b1879b5b53` |
| Version | `0.21.0` (tagged, released to crates.io/PyPI/npm) |
| Master CI | green (Dependency Graph, Docs, CI, CodeQL, Security Audit all `success`) |
| Benchmark baseline tag | `issue66-500-base-e479b27` |
| Checked-in benchmark artifacts | `data/comparison/{results_100,results_100_repeatability,results_500,shared_stock,aizynthfinder_public_data}` — a 500-target native+shared-stock (393-compound) comparison already exists for both engines under `disabled`/`conservative` ring-context policy |
| Repo hygiene | `data/` is 1.4G on disk locally but only ~14M is git-tracked; large stock/corpus files are already correctly gitignored (`.gitignore` lines 48-78). Phase 1 should extend this existing pattern, not invent a new one. |

### Open PRs (2)

| # | Title | State | Mergeable |
|---|---|---|---|
| #68 | `feat(forward): add forward-prediction benchmark protocol and harness` | draft, CI 11/11 green | **CONFLICTING/DIRTY** vs current master — needs rebase |
| #56 | `feat: support MCP 2026-07-28 alongside legacy clients` | draft | **CONFLICTING** vs current master — unrelated to this program |

Neither PR is touched by this document's PR. #68 is directly relevant (see
§4) and will need a rebase by its owner before any Track-B work builds on
top of it; #56 is out of scope entirely.

### Open issues (4)

| # | Title | Relevance |
|---|---|---|
| #86 | Large shared-stock sensitivity arm (match AiZynthFinder's ~17.4M-compound ZINC catalog) | Directly Phase 1 |
| #77 | `bug(retro): aryl_amine_retro` deletes ring nitrogen instead of returning a second amine precursor | **Not mentioned in the program brief** — a real open correctness bug discovered during this audit. Out of this program's explicit scope, but flagged for awareness; fixing it doesn't compete with any Phase 1-7 track and could be picked up independently at any time. |
| #72 | `bug(templates)`: extracted templates carry no ring-topology info | Directly Phase 3. Still open; last comment (2026-08-03) predates the 500-target round's completion, so its "hasn't started yet" note is simply stale, not a live contradiction. |
| #61 | Roadmap: benchmark and improve forward reaction prediction quality | Directly Phase 2/Track B (see §4) |

## 3. Track A vs Track B — do not conflate (per explicit program instruction)

Two rerankers exist in this codebase. They are genuinely separate,
non-overlapping code paths — confirmed by direct grep, not assumption.

**Track A — retrosynthesis candidate reranker** (`src/candidate.rs`, top-level
`renkin` crate, merged via PR #59 / v0.19.0):

- Full feature-extraction + pool-export + baseline-scoring pipeline exists:
  `CandidateReranker` trait, `ProposalMode`, `CandidatePool`,
  `extract_features`, `FEATURE_SCHEMA_VERSION` (`src/candidate.rs`).
- `src/pool_export.rs` produces JSONL candidate rows + manifest, but "does
  not decide *which* targets to run or generate a pool at any particular
  scale — that is a driver's responsibility" (its own doc comment). No
  driver script exists to run it at scale, and no real pool/labeled-row data
  is checked in under `data/`.
- `CandidateReranker` is **unimplemented anywhere**; `src/main.rs`,
  `src/python.rs`, `src/wasm.rs` have zero references to it or to
  `pool_export` — library-only, no runtime wiring.
- `scripts/train_reranker.py` and `docs/guides/reranker-candidate-pools.md`
  state, in the project's own words: *"No reranker has been formally
  trained or evaluated ... has not been run against a real corpus, and no
  offline-gate decision ... has been made."*
- **This is exactly the gap Phase 5 targets** — a clean, self-contained,
  no-external-dependency task: build the missing real-data driver, run
  `pool_export` + `train_reranker.py` for the first time against real
  historical reaction data, and formally evaluate the existing offline gate.

**Track B — forward reaction reranker** (`crates/renkin-forward`, tracked in
Issue #61):

- Issue #61 is a 5-phase roadmap: Phase 0 (freeze benchmark protocol) →
  Phase 1 (benchmark harness) → Phase 2 (proposal-coverage improvements) →
  Phase 3 (forward-specific learned reranker, explicitly gated on Phase 2's
  measurement) → Phase 4 (acceptance gate) → Phase 5 (optional generative
  model). **Note the phase numbers in Issue #61 are Issue #61's own
  numbering, distinct from this document's Phase 0-7 — do not conflate the
  two numbering schemes when cross-referencing.**
- PR #68 is exactly "PR A" of that roadmap (Issue #61 Phase 0+1 only —
  protocol freeze + harness). It explicitly excludes the reranker itself.
  CI is green but it's not currently mergeable (see §2).
- The program brief's own gating ("Track B starts after PR #68's benchmark
  baseline is established") is confirmed correct against real state — Track
  B's reranker work has no code to build on yet beyond what #68 proposes,
  and #68 itself isn't landed.

## 4. Per-phase current state

| Phase | State | Evidence |
|---|---|---|
| 0 (this doc) | In progress — this PR | — |
| 1 (large matched stock) | Not started. Reusable foundation exists: `scripts/compare_shared_stock.py` (builds RENKIN `.smi` + AiZynthFinder InChIKey HDF5 from one shared source, with a verified zero-mismatch round trip) already does at 393-compound scale exactly what Phase 1 needs at 10k-250k scale. Open external risk: sourcing a structure-level ZINC corpus matching AiZynthFinder's exact `/public/zinc_stock.hdf5` revision (licensing, availability) — same gap Issue #86 already tracks. | §2, Issue #86 |
| 2 (217-template gap) | Root cause already diagnosed, not a blind unknown. `crates/renkin-forward/src/lib.rs:1712-1734` hard-asserts `(accepted, rejected) == (283, 217)` against `data/templates_extracted.smi`. Cause: `reverse_smirks_validated` (built on `chematic::rxn::parse_reaction`) rejects legitimate multi-condition SMARTS atom primitives (e.g. `[N;H1,H2:2]`), which many extracted USPTO templates use. Gives Wave A a concrete starting taxonomy bucket. | `crates/renkin-forward/src/lib.rs:1712-1734` |
| 3 (ring-context default) | Blocked on a real design conflict — see §5. Not started otherwise; `RingContextConfig::Disabled` remains default (`src/ring_context.rs:110-113`, `src/main.rs:288-289`), unchanged in v0.21.0 per its own release scope. | §5 |
| 4 (4,903-target full comparison) | Correctly gated behind Phase 1-3 per the program brief's own rule — nothing to do yet. | — |
| 5 (retro reranker offline gate) | Infrastructure 100% built (Track A above), genuinely never run on real data. Clean, independent, startable immediately. | §3 |
| 6 (Synth Kernel runtime surface) | `renkin::synthesizability::assess_routes` (`src/synthesizability/{mod,assessment,schema,signals,element_accounting,provenance}.rs`) confirmed library-only — zero references in `src/main.rs`, `src/python.rs`, `src/wasm.rs`. Design doc `docs/design/synthesizability-kernel-v0.md` already exists. Clean, independent, startable immediately. | `src/synthesizability/mod.rs`, `src/main.rs`, `src/python.rs`, `src/wasm.rs` |
| 7 (conditions/yield/side-reactions) | Greenfield, no existing code. Correctly last — lowest priority, largest scope, most prone to "looks complete" risk the program brief explicitly warns against. | — |

## 5. New finding: Phase 3's "Auto" option conflicts with current packaging

`Cargo.toml`'s `exclude` list (added during the v0.21.0 release, see
`b2885b6`) currently excludes **both** `data/templates_extracted*.smi` **and**
`data/ring_context_metadata_500.json` from the packaged crate — a deliberate
choice at the time, to keep the published crates.io package small (several
hundred MB of stock/corpus data was the actual concern; these two files were
swept into the same exclude glob).

This means Phase 3's proposed option **C ("Auto" — activate `Conservative`
automatically when bundled extracted templates and a matching sidecar are
both present)** cannot work as specified for any real `cargo
install`/`pip install`/npm-installed user today: neither file ships in the
published package, so the bundled-detection condition is never true outside
a git checkout.

This is not resolved here — it's a design question for whoever runs Phase 3.
The likely resolution is cheap: `data/ring_context_metadata_500.json` is
308K and `data/templates_extracted_500.smi` is a few hundred KB, both tiny
next to the multi-hundred-MB files the exclude rule was actually written
for — un-excluding just these two is plausible. But that's a packaging-size
trade-off decision, not something to decide in this audit.

## 6. Dependency graph

```
Phase 0 (this doc) ──┬── unblocks everything below (shared context)
                      │
Phase 1 (large stock) ─┐
Phase 2 (template gap) ─┤── independent of each other, independent of 3
                        │
Phase 3 (ring-context   │
  default decision) ────┤
                        │
                        ▼
              Phase 4 (4,903-target full comparison)
              [requires Phase 1 + 2 + 3 all resolved]

Phase 5 (retro reranker gate) ── independent, startable now, blocks nothing
Phase 6 (Synth Kernel surface) ── independent, startable now, blocks nothing
Phase 7 (conditions/yield/etc.) ── depends on nothing above; deliberately last
```

## 7. Proposed branch/PR sequence and parallelization

Mapped to the program brief's own Agent A-E structure:

| Agent | Branch | Scope |
|---|---|---|
| A | `feat/large-matched-stock-phase1` | Phase 1: stock-sourcing research spike + builder skeleton (extend `compare_shared_stock.py`'s approach, don't rewrite it) |
| B | `feat/forward-template-compat-wave-a` | Phase 2 Wave A: 283→350+, starting from the diagnosed SMARTS-primitive gap |
| C | `feat/retro-reranker-real-data-gate` | Phase 5: build the missing real-data driver for `pool_export`/`train_reranker.py`, run the existing offline gate for the first time |
| D | `feat/synth-kernel-cli-surface` | Phase 6 PR 1: Rust API cleanup + CLI `assess` subcommand |
| E | — (no branch) | Independent reviewer / scientific-claim auditor role across all of the above |

Parallel-safe: A + B (different crates: top-level `renkin` templates path
vs. `crates/renkin-forward`'s parser, though both touch chematic-adjacent
code — verify no shared-file collision before starting both); C's data
design; D's API design.

Not parallel-safe (restated from the program brief's own rules): no two
agents editing the same `Cargo.toml`/`Cargo.lock`/`CHANGELOG.md` section
concurrently; no formal benchmark run before its protocol is frozen; no
model-tuning before its gate is finalized; measurement PRs and
default-behavior-change PRs must stay in separate PRs.

Note: PR #68 already occupies `crates/renkin-forward` conceptually (Track
B). Phase 2's Wave A work also touches `crates/renkin-forward` (template
parsing), so Agent B's branch should be cut fresh from current `master`
(not from #68's stale branch) to avoid inheriting #68's existing conflicts;
the two will still need reconciling whenever #68 itself gets rebased, but
that's #68's owner's task, not Agent B's.

## 8. Go/no-go thresholds

Restated from the program brief, tied to concrete artifacts:

- **Phase 1**: 100-target gate stops on stock-identity mismatch, schema
  error, target duplication/loss, unexplained crash, false stock hit, memory
  exhaustion, or an anomalously high target-in-stock rate that would distort
  sample design.
- **Phase 2**: Wave A ≥350/500, Wave B ≥425/500, Wave C ≥450/500, stretch
  ≥475/500 — each wave a separate PR, each requiring a full 500-template
  compatibility rerun, the existing 609+ workspace tests, forward
  enumerate/hints regression, retro 100-target regression, and no
  invalid/no-op product rate regression.
- **Phase 3**: practical-equivalence margin starts at 1.5pp absolute route
  coverage; promotion requires zero unexplained route loss, zero
  crash/schema regressions, all known-invalid ring applications blocked,
  full lost/swapped-route audit, confirmed sidecar availability in a real
  bundled install (which requires resolving §5 first), no silent fallback
  for custom templates, and identical policy behavior across
  Rust/CLI/Python/WASM.
- **Phase 5**: coverage unchanged, end-to-end top-1 delta ≥+1.0pp, MRR delta
  ≥+0.01, top-10 regression ≤0.2pp, top-1 paired-bootstrap 95% CI lower
  bound >0 (target_id-clustered), deterministic inference, no invalid-rate
  regression, no meaningful p95 latency regression.
- **Phase 6**: identical assessment JSON and reproducibility hash across
  Rust/CLI/Python/WASM for the same fixture, route order and search
  algorithm unchanged, no new implicit score, "no route" never phrased as
  "unsynthesizable."

## 9. Effort/compute estimate (qualitative)

| Phase | Nature | Risk |
|---|---|---|
| 1 | Data acquisition (licensing, sourcing) + bounded engineering (builder) | **Open-ended** — success depends on finding a usable structure-level ZINC source; no guarantee |
| 2 | Bounded engineering, taxonomy already scaffolded | **Bounded** — clear waves, measurable targets |
| 3 | Analysis + a packaging decision (§5) + measurement reruns | **Bounded**, but blocked until §5 is resolved |
| 4 | Long-running compute (4,903 targets × 4 arms), no new design | **Bounded but slow** — mostly wall-clock, gated behind 1-3 anyway |
| 5 | Bounded engineering (driver script) + one real training/eval run | **Bounded** — infra already exists, this is its first real exercise |
| 6 | Bounded engineering, 4 small mechanical PRs (Rust/CLI/Python/WASM) | **Low risk** — well-scoped, no chemistry-correctness surface |
| 7 | Greenfield schema design + evidence sourcing | **Largest, most open-ended** — correctly last |

## 10. Recommendation: Phase 2 before Phase 1

**Start Phase 2 first.** Its 217-template gap has a concrete, already-diagnosed
root cause (§4) and zero external dependency — bounded engineering risk with
measurable waves. Phase 1 depends on sourcing a structure-level ZINC corpus
matching AiZynthFinder's exact stock revision, which carries real
licensing/availability uncertainty (the same open question Issue #86
already tracks) — open-ended risk with no guaranteed payoff regardless of
engineering effort spent.

Recommend Phase 1 proceed only as a **low-cost sourcing/licensing research
spike** in parallel (Agent A), not a committed build, until that uncertainty
resolves one way or another.

Phase 5 and Phase 6 can start immediately regardless of the Phase 1-vs-2
choice — both are internal, self-contained, and block nothing else in this
program.
