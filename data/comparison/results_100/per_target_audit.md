# Per-target audit — RENKIN native 21/100 solved routes + AiZynthFinder step-extraction sample

Conducted before the shared-stock rework (see "Known gaps" in
`docs/guides/open-source-retrosynthesis-comparison.md`), per the explicit
request to classify every currently-solved RENKIN route and to empirically
confirm AiZynthFinder's step-extraction walks the whole tree, not just the
first/last step. All findings below are reproduced directly against the
real `renkin` release binary / real `aizynthfinder:4.4.1` container — no
synthetic fixtures.

## Part 1 — RENKIN native, 21/100 solved routes classified

| `all_leaves_in_configured_stock` | `target_element_accounting_status` | count |
|---|---|---|
| `true` | `accounted` | 14 |
| `true` | `unaccounted_target_element` | 4 |
| `false` | `accounted` | 3 |
| `false` | `unaccounted_target_element` | 0 |
| **Total** | | **21** |

18 rows pass `all_leaves_in_configured_stock` (18/21); 3 fail. 17 rows pass
`target_element_accounting_status=accounted` (17/21); 4 fail. These two
partitions are independent — no target fails both checks.

### 3 common-stock-fail targets — root cause identified

For each, the harness's independent RDKit-canonical stock check
(`data/building_blocks.smi`, 449 lines) was cross-checked against every leaf
RENKIN's own route JSON labeled a "building block". In all 3 cases exactly
one leaf per route is not present in the stock file under **any** notation
(confirmed via full RDKit canonicalization of the entire stock file — this
is not a stereo-layer or aromaticity notation artifact; see reason below).

| `target_id` | Failing leaf (RENKIN's raw SMILES) | RDKit canonical | In `data/building_blocks.smi`? | Producing rule |
|---|---|---|---|---|
| `uspto50k_test#L679` | `C=C/C/C=C` | `C=CCC=C` (1,4-pentadiene) | No | `cc_single_cleavage` (handcrafted) |
| `uspto50k_test#L1640` | `O=C/C(=O)O` | `O=CC(=O)O` (glyoxylic acid) | No | `wittig_retro` (handcrafted) |
| `uspto50k_test#L4575` | `c1ccc(cc1)CC=O` | `O=CCc1ccccc1` (phenylacetaldehyde) | No | `co_aliphatic_cleavage` (handcrafted) |

**Reason: actual unresolved leaf (RENKIN engine-side stock false-positive),
not a canonicalization or notation difference.** Traced to
`ChemEnv::is_building_block` (`src/chem_env.rs:144`): the primary check is
an O(1) canonical-SMILES `HashSet` lookup, but it falls back to VF2
subgraph-isomorphism matching (`src/chem_env.rs:149-158`) whenever the
stock is small enough (`bb_count <= VF2_THRESHOLD = 2000`; the 402-compound
stock qualifies). This fallback accepts a candidate whenever a full-coverage
subgraph match exists against a `parse_smarts`-converted stock entry — a
looser equivalence than canonical-SMILES identity — and is the only
plausible explanation given none of the 3 failing leaves match *any* stock
entry under RDKit canonicalization in any notation. This is a real,
reportable RENKIN-side defect distinct from the stock-conversion issues in
Part 2 of "Known gaps"; **no engine change is made here** (out of scope for
this PR — see "What this PR deliberately does NOT include").

### 4 target-element-accounting-fail targets — root cause identified

| `target_id` | Failing step's rule | Element(s) unaccounted | Reason |
|---|---|---|---|
| `uspto50k_test#L2263` | `aryl_amine_retro` (`chan_lam_coupling`, handcrafted) | N (1 atom) | **Atom-loss rule** — a Chan-Lam retro-disconnection genuinely requires TWO precursors (arylboronic acid + amine), but this handcrafted template returns only one, silently deleting the ring nitrogen instead of returning it as a separate amine fragment. |
| `uspto50k_test#L984` | `extracted_9` (data-driven extracted template) | N, several ring/side-chain C's | **Atom-loss rule / template-application defect** — this extracted template's reverse-SMIRKS application returns a precursor (`c1cc(C(=O)O)ccc1Br`) missing the target's entire isoindolinone ring extension and amino-acid side chain, despite `atom_economy: 100.0` in RENKIN's own tool-reported field. This is a template-database quality issue distinct from the two handcrafted-rule cases below, and is the most concerning of the 4 — flagged for follow-up, not fixed here. |
| `uspto50k_test#L4259` | `cbz_deprotection_retro` + `boc_deprotection_retro` (both handcrafted; both trigger) | C, O (protecting-group atoms) | **Atom-loss rule** — both protecting-group-removal templates model deprotection as a single-precursor transform, omitting the reagent (Cbz-Cl / Boc₂O) that installs the group going forward. By construction, any atoms contributed by the protecting group are unaccounted. |
| `uspto50k_test#L3400` | `boc_deprotection_retro` (handcrafted) | C, O | Same systemic cause as above. |

3 of the 4 (`L2263`, `L4259`, `L3400`) trace to the **same systemic root
cause**: handcrafted single-precursor functional-group-interconversion
retro-templates (`*_deprotection_retro`, `aryl_amine_retro`) don't model the
second reagent that supplies or removes atoms in the real forward reaction.
This is a disclosed, structural property of the handcrafted reaction-family
rule set, not a per-target random bug. `L984`'s `extracted_9` defect is
different in kind (a specific extracted-template SMIRKS quality issue) and
is called out separately.

No route-normalization defect (i.e., a bug in this harness's own
`compare_route_graph.py`/`compare_validation.py`) was found — all 4 failures
are genuine, reproducible properties of RENKIN's own route output, not
artifacts of how the harness parses or checks it.

## Part 2 — AiZynthFinder native, step-extraction depth-walk verification

Concern: does `normalize_aizynthfinder_route`'s tree walk (and
`check_target_element_accounting`'s graph walk) actually visit every
`mol → reaction → mol` level, or could a shallow/first-last-only bug produce
a spuriously clean 66/66 `accounted` rate?

**Static check**: both `build_mol` (`compare_route_graph.py:192`) and
`check_target_element_accounting`'s `walk` (`compare_validation.py`) recurse
unconditionally into every child — `build_mol` calls itself for every
`reaction_node`'s `mol_child`, and `walk` calls itself for every
`node.children`, regardless of depth. There is no depth cap or
first/last-only special case in either function.

**Empirical check**: stratified sample of 10 `accounted` routes across the
observed depth range (`best_route_depth` 1–5; distribution among all 66
accounted routes: depth 1×35, 2×15, 3×4, 4×5, 5×5, 6×2 — i.e. 31/66 have
`best_route_depth >= 2`, refuting a "shallow-only" hypothesis on its own):

| `target_id` | depth | steps | leaves | warnings |
|---|---|---|---|---|
| `uspto50k_test#L1010` | 1 | 1 | 2 | — |
| `uspto50k_test#L1027` | 1 | 1 | 2 | — |
| `uspto50k_test#L112` | 2 | 3 | 4 | — |
| `uspto50k_test#L1166` | 2 | 3 | 4 | — |
| `uspto50k_test#L3015` | 3 | 3 | 2 | — |
| `uspto50k_test#L3592` | 3 | 4 | 5 | — |
| `uspto50k_test#L1708` | 4 | 6 | 5 | — |
| `uspto50k_test#L1845` | 4 | 4 | 5 | — |
| `uspto50k_test#L2603` | 5 | 5 | 4 | `charge_imbalance` (informational, doesn't gate) |
| `uspto50k_test#L2668` | 5 | 6 | 4 | — |

Additionally, 3 of the deepest routes in the whole 66 (`L3990`: depth 5,
9 steps; `L3345` and `L4489`: depth 6, 8 steps) were re-run directly against
the real container and their **full raw route trees manually reconstructed
and read end-to-end** (not sampled from the JSONL summary). `L3990`'s
9-step, 6-leaf route was printed and hand-traced level by level (ester
formation → lactam reduction → Weinreb amide → methyl isocyanate coupling →
Friedel-Crafts acylation chain), confirming: (a) the parsed depth/step/leaf
counts the harness reports match a manual count of the raw tree exactly,
and (b) every intermediate step (not just the root or the terminal leaves)
carries real, distinct chemistry that the element-accounting walk
necessarily traverses to reach the leaves it correctly reports.

**Conclusion**: the step-extraction logic genuinely evaluates every step.
The 66/66 `accounted` rate for AiZynthFinder is not an artifact of a
shallow or first/last-only check.
