# Element-accounting gate smoke measurement (v0.37.0 stage 5), 2026-08-30

Per `docs/design/candidate-time-element-accounting-gate-v0.md` §9 stage 5
("a lightweight smoke measurement... under Gated, recording excluded-
candidate counts and route-count deltas, before any default change or
release"), following stage 4 (CLI/Python/WASM parity, PR #228).

## Design

Parameters chosen to mirror `examples/spectator_bond_smoke.rs`'s own
precedent exactly (user-approved, 2026-08-30 -- offered "same shape,
larger N" and "different parameters" as alternatives, user picked the
literal mirror):

- Sample: first 15 lines of `data/finding4_pilot_2026-08-23/
  target_sample_n300_seed42.smi` (same n=300 pilot sample every other
  smoke in this project reuses).
- Rules: `default_rules()` + `data/templates_extracted_5000.smi`.
- `--depth 5 --beam-width 100 --max-routes 1`, 90s per-target
  cooperative-cancellation timeout via `SearchControl::with_timeout`
  (fresh per target, per `inspect_validation.rs`'s own documented
  precedent against a shared/hoisted control silently breaking this).

**Structural difference from `spectator_bond_smoke.rs`**: that example
runs once under `DiagnosticsOnly` and correlates findings against the one
route found, since `SpectatorBondLoss` findings are detected at the
(rule, target) level independent of which candidate survives.
`ElementAccountingGateVerdict` is already directly per-candidate, and
`CrowdOutDiagnostics::element_accounting_gated_out` is only ever
populated under `Gated` (empty under `DiagnosticsOnly` by design), so
there is no equivalent after-the-fact correlation available from a single
`DiagnosticsOnly` pass. `examples/element_accounting_smoke.rs` instead
runs each target **twice** (once `Off`, once `Gated`) and reports the
direct route-found delta alongside the real excluded-candidate count.

Full raw output: `run.log` (this directory).

## Summary (verbatim)

```
targets: 15
off: solved=5 timed_out=6
gated: solved=5 timed_out=7
regressions (off solved, gated did not): 0
new solves (gated solved, off did not): 0
total excluded candidates under gated: 1151 (13/15 targets with >=1 exclusion)
```

## Finding 1 (the load-bearing one): `Gated` disagrees with the existing Synthesizability Kernel's own allowlist for the exact same two rules

`boc_deprotection_retro` and `cbz_deprotection_retro` fire as candidate-time
`Rejected` repeatedly in this run (7 occurrences across targets 7, 11, 12,
14 -- see `run.log`), each because the Boc/Cbz-protected target carries 5+
carbons/2+ oxygens (heavy atoms) the deprotected precursor doesn't supply
-- a real, structural, always-true consequence of how these two rules are
written (`chem_env.rs:1093-1094`, dispatched to `boc_deprotection(mol)`/
`cbz_deprotection(mol)`, `smirks: ""`, `template_id: "rule:boc_deprotection_retro"`
/`"rule:cbz_deprotection_retro"` via `rr()`, `chem_env.rs:2015-2024`).

**These exact two `template_id` strings are already on the Synthesizability
Kernel's own `default_reagent_omission_allowlist()`**
(`src/synthesizability/schema.rs:490-495`):

```rust
fn default_reagent_omission_allowlist() -> Vec<String> {
    vec![
        "rule:boc_deprotection_retro".to_string(),
        "rule:cbz_deprotection_retro".to_string(),
    ]
}
```

used by `SynthesizabilityConfig::conservative()`
(`schema.rs:504-515`) to mean exactly "this template's own target-element
accounting failure is a known, intentional reagent omission, not a genuine
defect" at the **route level**, post-hoc, after a route is already
assembled.

**The candidate-time `ElementAccountingGatePolicy::Gated` gate (stages
1-4, this v0.37.0 slice) has no connection to this allowlist at all.** It
is a strict binary (`Off`/`DiagnosticsOnly`/`Gated`) with no per-template
override. This means: if `Gated` were ever the candidate-time default (or
even just recommended for interested users), it would **exclude every
Boc/Cbz-deprotection candidate before a route is even assembled** --
completely bypassing the fact that the Synthesizability Kernel's own
`conservative()` policy has already, deliberately, ruled these two
templates' element loss acceptable. A route that would pass the kernel's
own assessment would never get the chance to, because the candidate would
never survive to be assembled into one in the first place.

`ElementAccountingGatedCandidateRecord` already carries `template_id`
(`src/candidate.rs`), so the fix shape -- should this ever be pursued --
is mechanical: consult the same `reagent_omission_template_allowlist`
(keyed identically, `template_id`, byte-for-byte matching entries
confirmed above) before excluding under `Gated`, not a new allowlist
design. **This is a new design decision the design doc never scoped**
(not in §9's stages 1-6) and is not being made unilaterally here -- see
"Recommendation" below.

## Finding 2 (weaker, an open question, not a defect): halogen-swap rules aren't on either allowlist

`aryl_chloride_to_bromide` (`[c:1][Cl]>>[c:1][Br]`, `chem_env.rs:2111`)
and `acyl_chloride_from_acid` (`[C:1](=[O:2])Cl>>[C:1](=[O:2])O`,
`chem_env.rs:2274`) dominate this run's exclusion volume by far (the
overwhelming majority of the 1151 total) -- both are per-element-heavy-atom
swaps (Cl-for-Br, Cl-for-OH) where one heavy atom takes another's place at
the same position, via an implied/untracked reagent, rather than a genuine
loss. `aryl_chloride_to_bromide` is explicitly defended elsewhere in this
codebase as legitimate: `chem_env.rs`'s own
`aryl_chloride_to_bromide_unaffected_by_halide_rule_removal` test
(`chem_env.rs:4955-4969`) calls it "a different, **atom-preserving** rule
(halogen-for-halogen swap)" and asserts it must keep firing.

Unlike Finding 1, **neither rule is on the Synthesizability Kernel's own
`default_reagent_omission_allowlist()` either** -- this is not "two
mechanisms disagree about a rule the project already ruled on," it's an
open question about *both* mechanisms: possibly a real, pre-existing gap
in the kernel's own allowlist (never asked to cover heavy-atom-for-heavy-
atom swaps, only carbon-skeleton protecting-group removal), possibly a
deliberate omission for a reason not documented anywhere found in this
research pass. Reported here as a question worth someone's attention, not
folded into Finding 1's verdict.

## Finding 3 (minor, expected): one additional timeout under `Gated`

`off_timed_out=6` -> `gated_timed_out=7`, with **`regressions: 0`** (no
solved target was lost) -- exactly one target moved from `UNSOLVED`
(search completed, no route) to `TIMEOUT` (search didn't complete within
90s) under `Gated`. At n=15 this is directional, not a sized effect, but
it is the real per-candidate `heavy_atom_counts`/`mol_from_smiles` parsing
cost showing up under `Gated` -- consistent with, not contradicting, the
reason `Off`'s own short-circuit (never calling `step_element_accounting`)
exists on the default path. Not alarming on its own; worth remembering if
a future, larger measurement is ever run.

## Recommendation

**`Gated` is not viable as a default, or even a confident recommendation
to interested users, until Finding 1 is resolved.** Two options, neither
decided here:

1. Wire the candidate-time gate to consult
   `SynthesizabilityConfig::reagent_omission_template_allowlist` (or an
   equivalent allowlist reachable from `SearchConfig`) before excluding a
   `Rejected` candidate under `Gated` -- the mechanical fix Finding 1's
   own evidence points to.
2. Scope `Gated`'s v1 usage to exclude known-allowlisted rule categories
   entirely, mirroring `SpectatorBondPolicy::Gated`'s own v1 scope-limit
   (`#`-free rules only, per `docs/design/spectator-bond-fail-closed-
   gating-v0.md` §2) -- i.e. `Gated` simply never touches a
   template on the allowlist, full stop.

Both are new design decisions outside `docs/design/candidate-time-
element-accounting-gate-v0.md` §9's six scoped rollout stages -- **not
implemented in this PR**, parked for explicit approval per this project's
own greenlane discipline (a new policy-semantics decision is Red, not
Green). `DiagnosticsOnly` remains fully safe and useful as-is: it computes
and records the verdict for every candidate without ever excluding one,
so nothing here blocks recommending `DiagnosticsOnly` for anyone who wants
visibility into this signal today.
