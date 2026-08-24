# Rule-safety census — 2026-08-24

**Status: static screen only.** A flagged row here is a candidate for a target fixture, never a verdict by itself — a rule only gets fixed/disabled once a specific target reproduces the defect via direct `apply_retro` calls, matching the standard this project already applied to `aryl_amine_retro`/`buchwald_hartwig_retro`.

**Updated (round 2)**: regenerated after `negishi_retro` and `grignard_addition_retro` were also confirmed via direct `apply_retro` reproduction and removed from `default_rules()` — see `negishi_retro_removed_from_default_rules`/`negishi_retro_would_corrupt_a_ring_fused_target_if_re_enabled` and `grignard_addition_retro_removed_from_default_rules`/`grignard_addition_retro_would_corrupt_a_ring_fused_target_if_re_enabled` in `src/chem_env.rs`. This follows round 1's removal of `n_benzylation_retro`/`michael_retro`. `default_rules()` now has 22 hand-crafted rules (was 26 before this round-2/round-1 pair, 24 after round 1), 6 flagged (was 10, then 8).

## Purpose

v0.36.0 Phase 1: before scaling the building-block stock (Phase 2), mechanically screen every hand-crafted `default_rules()` SMIRKS for the shape that broke `aryl_amine_retro`/`buchwald_hartwig_retro` (issue #77) — a minimally-constrained LHS plus a bare single-atom RHS product fragment, which lets chematic's substituent-carry-through BFS wander unchecked into a ring elsewhere in the real target.

## Reproduce

```
cargo run --example rule_safety_census
```

Deterministic and reproducible — pure static analysis of `default_rules()`'s own SMIRKS strings, no target data, no search. Output: `docs/validation/rule-safety-census-2026-08-24.md` (this file, hand-annotated with the cross-reference section below) + `data/rule_safety_census_2026-08-24.json` (raw per-rule data, regenerated verbatim by the command above).

**Cargo incremental-build caveat**: a stale `target/debug/.fingerprint` entry for the `renkin` package can serve an old example binary under `cargo run --example` even after `src/chem_env.rs` changes (observed once while producing the round-1 report — a `default_rules()` edit didn't take effect until `rm -rf target/debug/.fingerprint/renkin-*`). `cargo test` recompiles reliably; if `cargo run --example rule_safety_census`'s rule count looks stale, clear that package's fingerprint before trusting the output.

Static SMIRKS screen of all 22 `default_rules()` entries against the risk shape that broke `aryl_amine_retro`/`buchwald_hartwig_retro` (issue #77). Screening only -- a flag here is a reason to build a fixture, not a verdict. See `docs/design/` for the full v0.36.0 plan.

## Cross-reference: real SpectatorBondLoss findings already on record

The existing 2026-08-24 smoke measurement (`docs/validation/spectator-bond-smoke-2026-08-24.md`, `SpectatorBondPolicy::DiagnosticsOnly` against `default_rules()` + `data/templates_extracted_5000.smi` across 15 real targets) already ran every hand-crafted rule against real search traces — no new measurement needed to check whether any of them actually fired. Grepping that run's raw local output (`data/spectator_bond_smoke_2026-08-24/run.out`, gitignored — not re-committed here, only its aggregated counts) for non-`extracted_` rule names:

| Rule | Findings | Case | Status |
|---|---:|---|---|
| `michael_retro` | 46 | CrossProductTerritory (Case B) | **Confirmed and removed** (round 1: ring-fused C-CH2-C=O target, atom-accounting defect) |
| `n_benzylation_retro` | 28 | CrossProductTerritory (Case B) | **Confirmed and removed** (round 1: ring-fused N-CH2-Ar target, atom-accounting defect) |
| `grignard_addition_retro` | 16 | CrossProductTerritory (Case B) | **Confirmed and removed** (round 2: ring-fused tertiary-alcohol target — a 2-substituted indanol — atom-duplication defect, 18 observed heavy atoms across precursors vs. a chemically correct 11) |

`michael_retro`, `n_benzylation_retro`, and `grignard_addition_retro` combined the static-screen flag with real, already-observed findings — stronger evidence than the static screen alone. All three reproduced on real ring-fused targets via direct `apply_retro` calls (same BFS-carry-through-across-a-ring-fusion mechanism as `aryl_amine_retro`/`buchwald_hartwig_retro`), each with a specific, mechanism-consistent atom-count signature (a clean excess over the chemically correct total — duplication, not an arbitrary mismatch), and are now all removed.

`negishi_retro` matched the risk shape structurally (near-identical to the already-removed rules) but produced zero findings in the 15-target smoke sample — round 1 left it as "not cleared, just no free evidence either way." Round 2 built a deliberate fixture anyway (a real indane/tetralin-type target, chosen for the exact ring-fusion shape the mechanism needs, not pulled from the smoke sample) and confirmed the same defect: a 25-atom target produced a single outcome summing to 49 heavy atoms across its precursors, against a chemically correct 26. **Now confirmed and removed**, closing out the last of the plan's originally-named priority table.

**Caveat, same as the smoke doc's own**: n=15 targets, right-censored by a 90-second timeout — absence of a finding for a rule here is not evidence of safety, only absence of evidence from this small sample. Zero findings in that sample is exactly why `negishi_retro` needed its own deliberately-constructed fixture rather than relying on the smoke data alone.

## Flagged: multi-product RHS with a bare single-atom fragment

| Rule | SMIRKS | LHS mapped atoms | Bare RHS fragment(s) |
|---|---|---:|---|
| `friedel_crafts_acylation_retro` | `[c:1][C:2](=[O:3])>>[c:1].[C:2](=[O:3])Cl` | 3 | [c:1] |
| `aryl_carboxylation_retro` | `[c:1][C:2](=O)[OH]>>[c:1].[C:2](=O)O` | 2 | [c:1] |
| `cc_single_cleavage` | `[C:1][C:2]>>[C:1].[C:2]` | 2 | [C:1], [C:2] |
| `reductive_amination_retro` | `[C:1][N:2]>>[C:1]=O.[N:2]` | 2 | [N:2] |
| `cn_aliphatic_cleavage` | `[C:1][N:2]>>[C:1].[N:2]` | 2 | [C:1], [N:2] |
| `co_aliphatic_cleavage` | `[C:1][O:2]>>[C:1].[O:2]` | 2 | [C:1], [O:2] |

These six remain flagged-but-unattempted this round. `cc_single_cleavage`/`cn_aliphatic_cleavage`/`co_aliphatic_cleavage` are the plan's own "new, unverified angle" (BFS carry-through from an aliphatic cut site into an adjacent/fused aromatic ring elsewhere in the molecule, not yet tested against any fixture) — genuinely distinct future work, not attempted this round. `friedel_crafts_acylation_retro`/`aryl_carboxylation_retro`/`reductive_amination_retro` were flagged by the static screen but never had a dedicated fixture attempt either.

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
