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

## Addendum (2026-08-01) — post-Issue-#71-fix re-measurement

Part 1 above describes RENKIN's original 21-solved-route measurement,
before Issue #71's fix (PR #74, merged `de6a6d4`) removed the
`ChemEnv::is_building_block` VF2 subgraph-isomorphism fallback this audit
identified as the root cause of the 3 stock-check failures. Re-measuring
against the fixed binary drops RENKIN's native `route_found_rate` from
21/100 to **16/100** (see `aggregate_report.md`'s header note for the full
before/after). Part 1's table above is **not** re-run in place — it stays
as a historical record of the pre-fix state — this addendum instead
documents what actually changed.

Of the original 21 solved routes, 5 no longer produce a solved route at all,
and 1 produces a different route than before:

| `target_id` | Pre-fix classification | Post-fix outcome |
|---|---|---|
| `uspto50k_test#L679` | stock-fail (known VF2 false positive, `C=C/C/C=C`) | **not found** |
| `uspto50k_test#L1640` | stock-fail (known VF2 false positive, `O=C/C(=O)O`) | **not found** |
| `uspto50k_test#L4575` | stock-fail (known VF2 false positive, `c1ccc(cc1)CC=O`) | **still found, different route**: depth 3→4, now `all_leaves_in_configured_stock=true` (the false-positive leaf is gone) but `target_element_accounting_status=unaccounted_target_element` — the search now returns a real, deeper, stock-valid route that happens to fail the accounting check instead |
| `uspto50k_test#L3400` | accounting-fail (`boc_deprotection_retro`) | **not found** |
| `uspto50k_test#L4092` | clean (`true`/`accounted`) | **not found** |
| `uspto50k_test#L626` | clean (`true`/`accounted`) | **not found** |

The first 3 rows are exactly the 3 targets this audit's "3 common-stock-fail
targets" section identified and traced to the VF2 fallback — their
disappearance/change is the expected, predicted effect of the fix. `L3400`
was one of the 4 accounting-fail targets (`boc_deprotection_retro`); it no
longer produces any route.

**`L4092` and `L626` are new to this list.** Both were classified
`true`/`accounted` in Part 1 — i.e. their *reported* route's own leaves were
genuinely in stock even before the fix, by this audit's own per-leaf check.
Traced directly by building both the pre-#74 (`8e3a7cd`) and post-#74
binaries in separate worktrees (confirmed clean isolation: identical
chematic pin `0.8.1`; the only functional source diff between the two is
exactly the VF2-fallback removal in `is_bb`) and re-running each target
against both:

- **`L4092`: the pre-#74 binary is itself nondeterministic for this
  target.** Two separate invocations of the identical command against the
  identical (old) binary returned two *different* routes (a 2-step route
  via `extracted_34`+`cc_single_cleavage` on one run, the checked-in 5-step
  route on another). The post-#74 binary was checked 3x directly and is
  consistently unsolved; separately, the full 100-target repeatability
  cross-check below confirms post-#74 RENKIN is deterministic across the
  whole sample (99/100 byte-identical, the only difference being the
  disclosed boundary timeout). So the pre-fix "clean" classification for
  `L4092` was never a stable, repeatable result to begin with — this
  doesn't need to be attributed to the VF2 fix specifically.
- **`L626`: the pre-#74 binary is deterministic here** (3/3 identical
  routes via `extracted_12`+`co_aliphatic_cleavage`). The terminal leaf
  `C1[C@H](N)CC[C@@H2]C1` carries an invalid stereo descriptor (an
  explicit-2H atom can't be a stereocenter) from `co_aliphatic_cleavage`
  (a handcrafted graph rule, unrelated to the extracted-template ring-topology
  class in Issue #72) — a plausible-looking lead, since a malformed leaf
  behaving differently under an exact-identity check vs. the old VF2
  subgraph fallback was exactly the mechanism found for the original 3
  stock-fail targets. **Checked directly and ruled out**: RDKit parses this
  SMILES cleanly and canonicalizes it to `NC1CCCCC1` — genuine
  cyclohexylamine, present in `data/building_blocks.smi` — i.e. a correct
  canonicalizer discards the meaningless stereo flag and this leaf was
  likely always a valid stock match under both binaries. `L626`'s mechanism
  remains unestablished. Establishing it would mean instrumenting the
  search itself or chematic's own canonicalization on this exact string
  (not just RDKit's) — closer to Issue #72-adjacent work than a
  re-measurement, which this round does not do. Flagged for follow-up, same
  disposition as `L984`'s `extracted_9` defect above.

(Both pre-#74 routes terminate through `co_aliphatic_cleavage` producing
this same invalid-stereo-notation leaf shape — worth a narrowly-scoped bug
report of its own if someone wants to chase it, not filed here.)

Net effect on the "3 stock-fail" / "4 accounting-fail" partition from Part
1: the stock-fail bucket is now **empty** (0/16, down from 3/21) — every
route RENKIN now reports solved also passes independent stock
re-verification, closing the exact gap this fix targeted. The
accounting-fail bucket is still 4, but its membership changed: `L2263`,
`L984`, and `L4259` remain (same systemic/handcrafted-template causes
described above); `L3400` dropped out (route no longer found); `L4575`
joined it (via its new alternate route, not its old one).

A boundary-case timeout also differs between the two post-fix arms run for
this addendum: `uspto50k_test#L3345` timed out in the native-mode run,
`uspto50k_test#L4422` timed out in the shared-stock run (each completes
normally, unsolved, in the other arm). Neither is a stock/accounting-related
finding — both are consistent with running on shared, non-dedicated
hardware — and neither changes any headline rate, since a timeout counts as
not-found the same as a completed-but-unsolved run.
