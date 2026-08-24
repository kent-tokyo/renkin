# Rule-safety census — 2026-08-24

**Status: static screen only.** A flagged row here is a candidate for a target fixture, never a verdict by itself — a rule only gets fixed/disabled once a specific target reproduces the defect via direct `apply_retro` calls, matching the standard this project already applied to `aryl_amine_retro`/`buchwald_hartwig_retro`.

**Updated**: regenerated after two flagged rules (`n_benzylation_retro`, `michael_retro`) were confirmed via direct `apply_retro` reproduction and removed from `default_rules()` — see `n_benzylation_retro_removed_from_default_rules`/`n_benzylation_retro_would_corrupt_a_ring_fused_target_if_re_enabled` and `michael_retro_removed_from_default_rules`/`michael_retro_would_corrupt_a_ring_fused_target_if_re_enabled` in `src/chem_env.rs`. `default_rules()` now has 24 hand-crafted rules (was 26), 8 flagged (was 10).

## Purpose

v0.36.0 Phase 1: before scaling the building-block stock (Phase 2), mechanically screen every hand-crafted `default_rules()` SMIRKS for the shape that broke `aryl_amine_retro`/`buchwald_hartwig_retro` (issue #77) — a minimally-constrained LHS plus a bare single-atom RHS product fragment, which lets chematic's substituent-carry-through BFS wander unchecked into a ring elsewhere in the real target.

## Reproduce

```
cargo run --example rule_safety_census
```

Deterministic and reproducible — pure static analysis of `default_rules()`'s own SMIRKS strings, no target data, no search. Output: `docs/validation/rule-safety-census-2026-08-24.md` (this file, hand-annotated with the cross-reference section below) + `data/rule_safety_census_2026-08-24.json` (raw per-rule data, regenerated verbatim by the command above).

**Cargo incremental-build caveat**: a stale `target/debug/.fingerprint` entry for the `renkin` package can serve an old example binary under `cargo run --example` even after `src/chem_env.rs` changes (observed once while producing this report — a `default_rules()` edit didn't take effect until `rm -rf target/debug/.fingerprint/renkin-*`). `cargo test` recompiles reliably; if `cargo run --example rule_safety_census`'s rule count looks stale, clear that package's fingerprint before trusting the output.

Static SMIRKS screen of all 24 `default_rules()` entries against the risk shape that broke `aryl_amine_retro`/`buchwald_hartwig_retro` (issue #77). Screening only -- a flag here is a reason to build a fixture, not a verdict. See `docs/design/` for the full v0.36.0 plan.

## Cross-reference: real SpectatorBondLoss findings already on record

The existing 2026-08-24 smoke measurement (`docs/validation/spectator-bond-smoke-2026-08-24.md`, `SpectatorBondPolicy::DiagnosticsOnly` against `default_rules()` + `data/templates_extracted_5000.smi` across 15 real targets) already ran every hand-crafted rule against real search traces — no new measurement needed to check whether any of them actually fired. Grepping that run's raw local output (`data/spectator_bond_smoke_2026-08-24/run.out`, gitignored — not re-committed here, only its aggregated counts) for non-`extracted_` rule names:

| Rule | Findings | Case | Status |
|---|---:|---|---|
| `michael_retro` | 46 | CrossProductTerritory (Case B) | **Confirmed and removed** (ring-fused C-CH2-C=O target, atom-accounting defect) |
| `n_benzylation_retro` | 28 | CrossProductTerritory (Case B) | **Confirmed and removed** (ring-fused N-CH2-Ar target, atom-accounting defect) |
| `grignard_addition_retro` | 16 | CrossProductTerritory (Case B) | Flagged, real findings, but its LHS (`[C:1]([OH:2])([C:3])[C:4]`, a tertiary alcohol) never matched the one candidate target tried this round (0 outcomes) — no evidence either way, not a negative reproduction attempt; left in `default_rules()` as flagged-but-unconfirmed, not cleared |

`michael_retro` and `n_benzylation_retro` are the natural first candidates this round precisely because they combined the static-screen flag with real, already-observed findings — stronger evidence than the static screen alone. Both reproduced on real ring-fused targets via direct `apply_retro` calls (same BFS-carry-through-across-a-ring-fusion mechanism as `aryl_amine_retro`/`buchwald_hartwig_retro`), with a specific, mechanism-consistent atom-count signature: `n_benzylation_retro`'s broken outcome sums to 56 heavy atoms across its two precursors against a chemically-correct 46 (target's 45 + one new Br); `michael_retro`'s broken outcomes sum to 30 against a chemically-correct 24 (atom-conserving SMIRKS, no new atoms) — both a clean excess, confirming duplication (the bare fragment's carry-through re-collecting atoms the other declared fragment already claims), not just an arbitrary mismatch. Both are now removed.

`negishi_retro` matches the risk shape (structurally near-identical to the already-removed rules) but produced zero findings in this same 15-target sample — not cleared, just no free evidence either way; still needs its own deliberate fixture if pursued further. `grignard_addition_retro` needs a *different* target next time (one that actually contains a tertiary alcohol matching its LHS) before its flagged-but-unconfirmed status can move either direction — this round's one attempt tested nothing, since the rule never fired.

**Caveat, same as the smoke doc's own**: n=15 targets, right-censored by a 90-second timeout — absence of a finding for a rule here is not evidence of safety, only absence of evidence from this small sample.

## Flagged: multi-product RHS with a bare single-atom fragment

| Rule | SMIRKS | LHS mapped atoms | Bare RHS fragment(s) |
|---|---|---:|---|
| `friedel_crafts_acylation_retro` | `[c:1][C:2](=[O:3])>>[c:1].[C:2](=[O:3])Cl` | 3 | [c:1] |
| `aryl_carboxylation_retro` | `[c:1][C:2](=O)[OH]>>[c:1].[C:2](=O)O` | 2 | [c:1] |
| `negishi_retro` | `[c:1][CH2:2]>>[c:1][Br].[CH3:2]` | 2 | [CH3:2] |
| `cc_single_cleavage` | `[C:1][C:2]>>[C:1].[C:2]` | 2 | [C:1], [C:2] |
| `reductive_amination_retro` | `[C:1][N:2]>>[C:1]=O.[N:2]` | 2 | [N:2] |
| `cn_aliphatic_cleavage` | `[C:1][N:2]>>[C:1].[N:2]` | 2 | [C:1], [N:2] |
| `co_aliphatic_cleavage` | `[C:1][O:2]>>[C:1].[O:2]` | 2 | [C:1], [O:2] |
| `grignard_addition_retro` | `[C:1]([OH:2])([C:3])[C:4]>>[C:1](=O)[C:3].[C:4]` | 4 | [C:4] |

## Not flagged: single-product SMIRKS (no second-fragment risk)

| Rule | SMIRKS |
|---|---|
| `aryl_chloride_to_bromide` | `[c:1][Cl]>>[c:1][Br]` |
| `alcohol_oxidation_retro` | `[C:1][OH:2]>>[C:1]=O` |
| `acyl_chloride_from_acid` | `[C:1](=[O:2])Cl>>[C:1](=[O:2])O` |

## Not flagged: graph-based (ring-guarded by construction)

| Rule |
|---|
| `ester_cleavage` |
| `amide_cleavage` |
| `aryl_ether_retro` |
| `suzuki_retro` |
| `sulfonamide_retro` |
| `diaryl_sulfone_retro` |
| `boc_deprotection_retro` |
| `cbz_deprotection_retro` |

## Full per-rule detail

### `ester_cleavage`

Graph-based (empty SMIRKS).

- graph-based: requires is_bridge_bond (non-ring) by construction

### `amide_cleavage`

Graph-based (empty SMIRKS).

- graph-based: requires is_bridge_bond (non-ring) by construction

### `friedel_crafts_acylation_retro`

SMIRKS: `[c:1][C:2](=[O:3])>>[c:1].[C:2](=[O:3])Cl`

- multi-product RHS: 2 fragments
- bare single-atom RHS fragment(s): [c:1] -- exact shape of the confirmed aryl_amine_retro/buchwald_hartwig_retro defect
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `aryl_carboxylation_retro`

SMIRKS: `[c:1][C:2](=O)[OH]>>[c:1].[C:2](=O)O`

- minimal LHS: only 2 mapped atom(s) declared -- little context walling off BFS carry-through
- multi-product RHS: 2 fragments
- bare single-atom RHS fragment(s): [c:1] -- exact shape of the confirmed aryl_amine_retro/buchwald_hartwig_retro defect
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `aryl_ether_retro`

Graph-based (empty SMIRKS).

- graph-based: requires is_bridge_bond (non-ring) by construction

### `aryl_chloride_to_bromide`

SMIRKS: `[c:1][Cl]>>[c:1][Br]`

- minimal LHS: only 1 mapped atom(s) declared -- little context walling off BFS carry-through
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `suzuki_retro`

Graph-based (empty SMIRKS).

- graph-based: requires is_bridge_bond (non-ring) by construction

### `heck_retro`

SMIRKS: `[c:1][CH:2]=[CH:3]>>[c:1][Br].[CH2:2]=[CH:3]`

- multi-product RHS: 2 fragments
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `heck_retro_terminal`

SMIRKS: `[c:1][CH:2]=[CH2:3]>>[c:1][Br].[CH2:2]=[CH2:3]`

- multi-product RHS: 2 fragments

### `negishi_retro`

SMIRKS: `[c:1][CH2:2]>>[c:1][Br].[CH3:2]`

- minimal LHS: only 2 mapped atom(s) declared -- little context walling off BFS carry-through
- multi-product RHS: 2 fragments
- bare single-atom RHS fragment(s): [CH3:2] -- exact shape of the confirmed aryl_amine_retro/buchwald_hartwig_retro defect

### `cc_single_cleavage`

SMIRKS: `[C:1][C:2]>>[C:1].[C:2]`

- minimal LHS: only 2 mapped atom(s) declared -- little context walling off BFS carry-through
- multi-product RHS: 2 fragments
- bare single-atom RHS fragment(s): [C:1], [C:2] -- exact shape of the confirmed aryl_amine_retro/buchwald_hartwig_retro defect
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `wittig_retro`

SMIRKS: `[C:1]=[C:2]>>[C:1]=O.[C:2]=O`

- minimal LHS: only 2 mapped atom(s) declared -- little context walling off BFS carry-through
- multi-product RHS: 2 fragments
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `reductive_amination_retro`

SMIRKS: `[C:1][N:2]>>[C:1]=O.[N:2]`

- minimal LHS: only 2 mapped atom(s) declared -- little context walling off BFS carry-through
- multi-product RHS: 2 fragments
- bare single-atom RHS fragment(s): [N:2] -- exact shape of the confirmed aryl_amine_retro/buchwald_hartwig_retro defect
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `cn_aliphatic_cleavage`

SMIRKS: `[C:1][N:2]>>[C:1].[N:2]`

- minimal LHS: only 2 mapped atom(s) declared -- little context walling off BFS carry-through
- multi-product RHS: 2 fragments
- bare single-atom RHS fragment(s): [C:1], [N:2] -- exact shape of the confirmed aryl_amine_retro/buchwald_hartwig_retro defect
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `co_aliphatic_cleavage`

SMIRKS: `[C:1][O:2]>>[C:1].[O:2]`

- minimal LHS: only 2 mapped atom(s) declared -- little context walling off BFS carry-through
- multi-product RHS: 2 fragments
- bare single-atom RHS fragment(s): [C:1], [O:2] -- exact shape of the confirmed aryl_amine_retro/buchwald_hartwig_retro defect
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `alcohol_oxidation_retro`

SMIRKS: `[C:1][OH:2]>>[C:1]=O`

- minimal LHS: only 2 mapped atom(s) declared -- little context walling off BFS carry-through
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `sonogashira_retro`

SMIRKS: `[c:1][C:2]#[C:3]>>[c:1]Br.[C:2]#[C:3]`

- multi-product RHS: 2 fragments
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `sulfonamide_retro`

Graph-based (empty SMIRKS).

- graph-based: requires is_bridge_bond (non-ring) by construction

### `diaryl_sulfone_retro`

Graph-based (empty SMIRKS).

- graph-based: requires is_bridge_bond (non-ring) by construction

### `boc_deprotection_retro`

Graph-based (empty SMIRKS).

- graph-based: requires is_bridge_bond (non-ring) by construction

### `grignard_addition_retro`

SMIRKS: `[C:1]([OH:2])([C:3])[C:4]>>[C:1](=O)[C:3].[C:4]`

- multi-product RHS: 2 fragments
- bare single-atom RHS fragment(s): [C:4] -- exact shape of the confirmed aryl_amine_retro/buchwald_hartwig_retro defect
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `claisen_retro`

SMIRKS: `[C:1](=O)[CH2:2][C:3](=O)[O:4]>>[C:1](=O)O.[C:2]=[C:3][O:4]`

- multi-product RHS: 2 fragments

### `acyl_chloride_from_acid`

SMIRKS: `[C:1](=[O:2])Cl>>[C:1](=[O:2])O`

- minimal LHS: only 2 mapped atom(s) declared -- little context walling off BFS carry-through
- LHS declares no ring closure of its own (background fact for this whole corpus, not independently discriminating)

### `cbz_deprotection_retro`

Graph-based (empty SMIRKS).

- graph-based: requires is_bridge_bond (non-ring) by construction
