# Retro-Rule Precision & Coverage Gaps — Design Doc

Status: **Findings #1/#2 fixed and merged (PR #171, 2026-08-23). Findings
#3/#4/#5 still open.** These are external, hands-on findings surfaced
while writing a public "tried it" article comparing RENKIN against
AiZynthFinder (`find_routes` and `audit_route` run against
v0.31.0–v0.34.0; behavior confirmed unchanged across all four). Two were
new, previously untracked correctness bugs in hand-crafted rules — both
fixed. #5 was investigated further after the initial writing and turned
out to be a real, sizeable gap (see its own section below), not a small
one as first guessed. #3/#4 cross-reference existing open issues with an
independent reproduction case, not yet posted as issue comments (needs
explicit authorization — posting to GitHub is a visible external action,
not autonomous work).

## 1. `aryl_ether_retro` mislabels ester-bond cleavage as Ullmann coupling (NEW, not tracked)

**Location:** `src/chem_env.rs:1825`

```rust
// Ar-O → Ar-OH + leaving fragment (retro-Ullmann ether synthesis)
rr("aryl_ether_retro", "[c:1][O:2]>>[c:1]O.[O:2]"),
```

**Repro:** target aspirin (`CC(=O)Oc1ccccc1C(=O)O`), default hand-crafted
rules only (no `templates_path`). `find_routes` returns two routes with
*identical* precursors (`CC(O)=O` + `c1cccc(C(O)=O)c1O`, i.e. acetic acid
+ salicylic acid) from two different rules:

- `ester_cleavage` → `reaction_family: esterification`, conditions `NaOH
  or LiOH (2 eq), THF/H₂O, rt → 60°C`. Correct.
- `aryl_ether_retro` → `reaction_family: ullmann_ether`, conditions
  `Cs₂CO₃ (2 eq), DMF, 110°C`, `procedure_hint: "Mix aryl halide + phenol
  + Cs₂CO₃..."`. Wrong: the bond being cut is the ester's acyl–O bond,
  not an aryl ether. Ullmann conditions don't apply, and the
  `procedure_hint` describes a reaction (aryl halide + phenol) that
  doesn't match either precursor (neither is an aryl halide).

**Root cause:** the SMIRKS `[c:1][O:2]` matches any aromatic-C–O single
bond, with no check on what's attached to the O side. When O is also
bonded to an acyl carbon (i.e. an ester `Ar-O-C(=O)-R`), the pattern
still fires and mislabels the disconnection.

**Same fix pattern already exists in this file** — `aryl_carboxylation_retro`
(line ~1805) was deliberately tightened from a bare `O` match to `[OH]`
specifically to exclude ester oxygens (see the comment directly above it,
lines 1797–1804: *"a bare O also matches an ester oxygen... Requiring a
terminal hydroxyl restricts the match to genuine free carboxylic acids"*).
`aryl_ether_retro` needs the analogous exclusion, e.g.
`[c:1][O;!$(OC=O):2]>>[c:1]O.[O:2]` (negative recursive SMARTS: O not
bonded to a carbonyl carbon), or an equivalent post-match filter.

**Impact:** every target with an aryl ester produces a spurious duplicate
route with wrong reaction_family/conditions. Silent — nothing flags it as
wrong, it just looks like two independently-valid routes.

## 2. `suzuki_retro` never attaches boron to either fragment (NEW, not tracked)

**Location:** `src/chem_env.rs:548` (`biaryl_cleavage`), used by
`suzuki_retro` (line 958). Regression tests already pin the current
(incomplete) behavior: `suzuki_retro_biphenyl_gives_bromobenzene_and_benzene`
(line 3596), `suzuki_retro_biphenyl_solvable_with_bb` (line 3629).

**Repro:** target biphenyl (`c1ccc(-c2ccccc2)cc1`), default hand-crafted
rules. `suzuki_retro` returns bromobenzene (`c1cc(Br)ccc1`) + **plain
benzene** (`c1ccccc1`) — no boronic acid/ester on either fragment. A real
retro-Suzuki disconnection needs one aryl halide/pseudohalide partner
*and* one boron-containing partner (boronic acid, boronate ester, or
trifluoroborate); "aryl halide + arene" isn't a Suzuki coupling.

**Root cause, read from `biaryl_cleavage` directly:** the function finds
a bridge bond between two aromatic carbons, then for each orientation
calls `build_sub_molecule_with_br(mol, comp_br, cut)` on one fragment and
plain `build_sub_molecule(mol, comp_plain)` on the other (chem_env.rs:574–580)
— the second fragment never gets anything attached at the cut site.
`build_sub_molecule_with_br` (line 419) and the sibling
`build_sub_molecule_with_cl` (line 446) both already show the pattern for
appending a single halogen atom at `cut_atom`; there's no
`build_sub_molecule_with_boronic_acid` equivalent.

**Suggested fix:** add a `build_sub_molecule_with_boronic_acid` helper
(same shape as `build_sub_molecule_with_br`, but appends `B` + two `OH`
atoms bonded to the cut site instead of one `Br` atom) and call it on
`comp_plain` in `biaryl_cleavage` instead of `build_sub_molecule`. The two
existing regression tests pinning "bromobenzene + benzene" will need
updating to "bromobenzene + arylboronic acid" as part of the same change
— they're pinning the bug, not a deliberate design choice, as far as this
doc's reproduction can tell; worth confirming intent before changing them
since no comment states which.

**Impact:** every `suzuki_retro` route is missing atoms/a real reagent on
one side. Doesn't fail atom-balance checks (Br and H differ in count, but
whatever downstream check exists for stoichiometric plausibility should
be flagging "no boron in a coupling that requires it" — worth checking
whether this is also a gap in `synthesizability/schema.rs`'s reagent
checks, related in spirit to `reagent_omission_template_allowlist`
mentioned in the `aryl_amine_retro` removal comment at line 1814).

## 3. Ibuprofen: search doesn't converge without an explicit beam cap (cross-references #101, #128)

**Repro:** target ibuprofen (`CC(C)Cc1ccc(cc1)C(C)C(=O)O`), 500 extracted
templates (`data/templates_extracted_500.smi`), `depth=5`,
`beam_width=0` (unlimited). Killed after >6 minutes at ~95% CPU with no
result. At `depth=3, beam_width=50`: terminates fast but 0 routes, with
diagnostics `matched_templates: 1016, nodes_expanded: 41, stock_hits: 76,
beam_limit_hit: true`.

This looks like the same territory as #128 (per-node cost blowup,
`depth=5`/`beam=100` "dramatically slower" than the Phase 31 baseline —
targets 2/4/5 in that issue's 5-target precheck ran 200–350s for **0
routes each**) and #101 (beam-width crowd-out after hash-atom template
coverage increase — 1016 matched templates for one target is a lot of
branching factor for a beam of 50 to survive). Not filing as a new issue;
adding this as a third independent repro case (a real end-user target,
not a corpus sample) for whichever of #101/#128 turns out to be the
actual root cause, since both are currently "reproduced, uncaused" /
"not yet known".

## 4. Validator-confirmed rate collapses from 15.41% to 0.88% (relates to #61)

The current corrected USPTO-50k baseline (commit `e20dc8c`, frozen,
`v0.15.5`): search-to-stock 20.09% (986/4,907) → atom-balance-filtered
15.41% (756/4,907) → validator-confirmed **0.88%** (43/4,907). The drop
from atom-balance-filtered to validator-confirmed is the single biggest
cliff in the pipeline — roughly 94% of atom-balanced routes fail forward
replay. Findings #1 and #2 above are concrete instances of the kind of
thing that would cause exactly this: a route that balances atoms fine but
whose declared reaction doesn't actually replay correctly (or doesn't
have a correct declared reaction to replay at all). #61 already tracks
"benchmark and improve forward reaction prediction quality" as a roadmap
item; suggest treating a sweep for more `aryl_ether_retro`/`suzuki_retro`-shaped
bugs (rules whose SMIRKS pattern is broader than the reaction_family label
they're assigned) as a concrete first step under that roadmap, since it's
cheap to audit (grep every `rr(...)` call for a SMIRKS pattern with no
compensating negative-match constraint) and directly explains part of the
0.88% number rather than requiring new measurement infrastructure.

## 5. `find_routes()`'s own output can't be forward-validated by `audit_route()`

**Repro:** `find_routes(target=aspirin, ...)` piped straight into
`audit_route(..., format="renkin")` → `forward_validation.status:
not_evaluable`, `reason: missing_reaction_representation`, overall route
`status: partial` even with a matching stock supplied. AiZynthFinder's
export format, by contrast, carries enough info for
`forward_validation.status: pass` on the same audit pipeline.

**Root cause, investigated and confirmed 2026-08-23 (was "not
investigated further" originally)**: this is **not** a data-transport
gap in `find_routes()`'s `--format json` output as first guessed — it's
rule-*class*-dependent, and traced to one exact place.
`normalize_renkin_route` already builds a real
`ReactionEvidence::RenkinTemplate { template_id }` for every step, and
`bridge::audit_route.rs`'s `AuditRouteFormat::Renkin` dispatch already
passes the real rule set (`Some(rules)`) into `audit`. The gap is in
`forward.rs`'s `declared_smirks` match arm for `RenkinTemplate`:

```rust
ReactionEvidence::RenkinTemplate { template_id } => {
    let rule = rules_by_template_id.and_then(|m| m.get(template_id.as_str()))
        .ok_or(MissingReactionRepresentation)?;
    if rule.smirks.is_empty() {
        return Err(MissingReactionRepresentation);
    }
    Ok((rule.smirks.as_str(), false))
}
```

Confirmed by re-running the exact aspirin repro after this doc's own #1/#2
fixes landed: `find_routes` finds this target via **two** different
rules, and only one reaches `pass`:

```
route 1 (rule:co_aliphatic_cleavage, real SMIRKS): forward_validation: {status: pass}
route 2 (rule:ester_cleavage,        empty smirks): forward_validation: {status: not_evaluable, reason: missing_reaction_representation}
```

**Every graph-based default rule (empty `smirks`, dispatched by name in
`apply_retro`) hits this same wall — not just `ester_cleavage`.**
Counted directly against `default_rules()`: **8 of 27 hand-crafted rules
(~30%)** are graph-based today: `ester_cleavage`, `amide_cleavage`,
`aryl_ether_retro` (as of this doc's #1 fix), `suzuki_retro` (as of #2),
`sulfonamide_retro`, `diaryl_sulfone_retro`, `boc_deprotection_retro`,
`cbz_deprotection_retro`. Any route whose winning disconnection used one
of these — several of the most common real reaction types RENKIN
handles — can *never* reach `forward_validation: pass` via self-audit,
regardless of how chemically correct the route actually is. This doc's
own #1/#2 fixes (converting `aryl_ether_retro` and effectively touching
`suzuki_retro` to graph-based form, for good, independent reasons — see
above) *widened* this gap by one rule, worth flagging explicitly rather
than treated as a free side effect.

**Not fixed here — needs a real design decision, not a small patch**:
unlike a SMIRKS-string rule, a graph-based rule has no positional
template for `chematic::rxn::run_reactants` to reverse-apply; "forward
replay" for these needs either (a) a per-rule forward-verification
function mirroring each graph-based cleavage function structurally
(8 functions to write and keep in sync with their retro counterparts),
or (b) each graph-based function additionally emitting a real,
atom-mapped forward SMIRKS as evidence at the moment it performs the
cut (it already knows exactly which atoms/bonds moved), stored
alongside `template_id` on the route step and consumed by the *existing*
`declared_smirks` machinery unchanged. (b) looks more promising — it
reuses `forward.rs`'s existing replay path entirely, no new verification
logic — but wasn't designed further here; flagging the shape of the
decision, not making it. See `ROADMAP.md`'s "Make RENKIN's own routes
fully self-auditable (P0)" section for the release-gate framing this
would need to satisfy.

## Suggested priority

1 and 2 were small, self-contained, and each had an existing sibling code
pattern to copy (the `[OH]` fix for `aryl_carboxylation_retro`; the
`build_sub_molecule_with_br`/`with_cl` pattern) — both merged (PR #171).
Both directly reduce the "looks solved but isn't" surface area, which is
what most visibly erodes trust when someone tries the tool by hand.

**5 turned out NOT to be small** once actually investigated (see its own
section above, updated after this doc's initial writing): it's a real
~30%-of-default-rules gap (8/27 graph-based rules can never reach
`forward_validation: pass` via self-audit) needing a genuine design
decision on how graph-based rules produce forward-replay evidence —
recorded as a P0 item in `ROADMAP.md` for a real design/implementation
round, not something to patch in passing. 3 and 4 are bigger/
already-tracked — this doc just adds independent repro evidence to
existing issues rather than proposing new investigation threads.
