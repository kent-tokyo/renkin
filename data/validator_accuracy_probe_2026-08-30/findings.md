# Validator accuracy measurement (v0.37.0), 2026-08-30

Per `docs/design/validator-accuracy-measurement-v0.md`, measuring
`validation::validate_step` only (never `bridge::forward::validate_step_forward`
-- the two are deliberately not unified, design doc §1.1). Second half of
v0.37.0 "Verified Candidate Integrity", following the element-accounting
gate (stages 1-5, PRs #218/#226-#229).

Full raw output: `rows.jsonl` (gitignored, 102,963 rows = 4,903 targets x
21 rules, regenerable via `cargo run --release --example
validator_accuracy_probe`), `summary.json` (tracked, this directory).

## True-accept side (design doc §2(b), the attribution-free probe)

For each of `data/reranker_labels_uspto50k_test.jsonl`'s 4,903 real,
USPTO-derived `(target, correct_precursor_set)` pairs, constructed a
candidate `ReactionStep` under every one of `default_rules()`'s 21
hand-crafted rule names (the design doc's own count, 22, predates
`heck_retro`'s 2026-08-29 removal) and called `validate_step` directly --
zero new data collection, ~24 seconds total (design doc's own "seconds
not hours" estimate confirmed, not just assumed -- dry-run on 50 targets
timed at 0.6s first, extrapolated, then the full run launched).

```json
{
  "n_targets": 4903,
  "true_accept_rate": {"n_numerator": 2261, "value": 0.461},
  "true_accept_via_smirks_rule": {"n_numerator": 120, "value": 0.024},
  "true_accept_via_graph_rule_only": {"n_numerator": 2141, "value": 0.437},
  "all_not_evaluable": {"n_numerator": 0, "value": 0.0}
}
```

**Reported split by validation branch, not blended into one number** --
`validate_step` has two internal mechanisms with very different
discriminative power (see next section), and averaging them would repeat
exactly the mistake the design doc forbids between `validate_step`/
`validate_step_forward` (§1.1: "never average them into one 'the
validator' number") one level down:

- **SMIRKS branch** (`forward::rule_reproduces`, reverses *the claimed
  rule's own* SMIRKS -- genuinely rule-specific): only **2.4%** (120/4903)
  of real, correct disconnections validate this way.
- **Graph branch** (`validate_graph_step`, a small delta table): **43.7%**
  (2141/4903) validate *only* through this branch -- the overwhelming
  majority of the headline 46.1% true-accept number. See below for why
  this branch's signal is much weaker than it looks.

**Per-`reaction_family` hit counts** (design doc §1.3's own caveat stated
here, not left implicit -- these are grouped by RENKIN's own asserted
`reaction_family_for_rule`, never validated against an independent,
corpus-native class label; `data/uspto50k_raw_*_split.jsonl`'s own
`class` field is the literal string `"UNK"` for every row, confirmed in
the design doc's own research pass):

```json
{
  "amide_coupling": 794,
  "esterification": 794,
  "ullmann_ether": 794,
  "friedel_crafts_sulfonylation": 896,
  "sulfonamide_formation": 896,
  "suzuki_coupling": 452,
  "reductive_amination": 66,
  "carbonyl_reduction": 44,
  "friedel_crafts_acylation": 1
}
```

**`amide_coupling` = `esterification` = `ullmann_ether` = 794, exactly.**
**`friedel_crafts_sulfonylation` = `sulfonamide_formation` = 896,
exactly.** This is not a coincidence and not a family-level tie needing
its own explanation -- it is the direct, quantified consequence of the
next finding: these are literally the *same* 794 (respectively 896)
targets, each counted once per rule name sharing one delta bucket, not
three (or two) independently-covered reaction families.

## Why the graph branch's numbers look inflated: shared delta buckets

`validate_graph_step` (`src/validation/graph_rules.rs:121-129`)
deliberately maps multiple rule names to one shared delta check, each
documented as an intentional design choice, not an oversight:

```rust
"ester_cleavage" | "amide_cleavage" | "aryl_ether_retro" => {
    validate_delta(target, precursors, ESTER_AMIDE_DELTA)
}
"sulfonamide_retro" | "diaryl_sulfone_retro" => {
    validate_delta(target, precursors, SULFONYL_DELTA)
}
```

The comment above the first arm is explicit: *"aryl_ether_retro: Ar-O-R
-> Ar-OH + R-OH... Net delta: +1 O, +2 H -- formally the same
hydrolysis-shaped delta as ester/amide cleavage, confirmed by direct atom
counting."* This means: **for these five rule names, `validate_step`
checks mass-balance shape, not rule identity.** A real ester-cleavage
target attributed to `aryl_ether_retro` (a *wrong* attribution) validates
`Valid` exactly as readily as the correct `ester_cleavage` attribution,
because both hit the identical `ESTER_AMIDE_DELTA` check. Confirmed
directly (not inferred) via a throwaway probe before building anything on
this assumption:

```
phenyl acetate mislabel  (aryl_ether_retro, correct-split precursors): Valid
aspirin mislabel         (aryl_ether_retro, correct-split precursors): Valid
phenyl acetate (correct: ester_cleavage, same precursors):             Valid
```

All three return `Valid` — the graph branch cannot tell the wrong
attribution apart from the right one within a shared bucket.

## True-reject side: the design doc's own premise doesn't hold

Design doc §3 planned to reuse "the already-existing confirmed-wrong
cases" (`chem_env.rs`'s `aryl_ether_retro_skips_aryl_ester_oxygen`/
`aryl_ether_retro_skips_aspirin_ester_oxygen`, `search.rs`'s
`isoindolinone_ring_disconnection_is_rejected_not_returned`) as the
starting point for a small hand-curated negative corpus. **None of the
three serve as a `validate_step` negative case:**

- The two `aryl_ether_retro` cases test a *different layer*:
  `apply_retro`'s own candidate-generation guard (confirming
  `aryl_ether_retro` never *proposes* a disconnection on an ester
  oxygen), not `validate_step`'s validation logic. Constructing the
  equivalent `validate_step` triple by hand (claim `aryl_ether_retro`,
  supply the real correct ester-cleavage precursors) returns `Valid`, not
  `Invalid` -- per the shared-bucket finding above, this is
  **structurally unrejectable by `validate_graph_step`**, not merely an
  untested case. `n=0` usable existing cases from this pair, not `n=2`.
- The `isoindolinone_ring_disconnection_is_rejected_not_returned` case is
  a whole-route-search finding (an extracted template from
  `templates_extracted_500.smi` dropping the target's nitrogen, caught by
  the route-level element-accounting check,
  `stats.route_integrity.unaccounted_target_element`) -- it does not
  isolate a single `(rule_name, target, precursors)` triple usable with
  `validate_step` at all without further investigation to identify which
  specific extracted template(s) caused it. Not converted to a probe case
  here.

**The one genuinely confusable graph-rule pair with *distinguishable*
deltas** is `boc_deprotection_retro` (`BOC_DELTA = -C5H8O2`) vs.
`cbz_deprotection_retro` (`CBZ_DELTA = -C8H6O2`, `src/validation/
graph_rules.rs:46-47`) -- same reaction shape (protecting-group removal),
different atom counts, so a cross-attribution genuinely tests the
validator's discriminative power rather than being a strawman (e.g.
`suzuki_retro` vs. `sulfonamide_retro` -- unrelated shapes, any check
would catch it, not informative). Constructed and verified both
directions:

```
Boc-protected aniline, correct (boc_deprotection_retro):    Valid
Boc-protected aniline, wrong   (cbz_deprotection_retro):    Invalid
Cbz-protected aniline, correct (cbz_deprotection_retro):    Valid
Cbz-protected aniline, wrong   (boc_deprotection_retro):    Invalid
```

**True-reject result: n=2 (both directions of one pair), 2/2 correctly
rejected.** This is a real, useful, mechanically-verified data point, but
explicitly **not** the design doc's own ~30-50-case target. Per this
project's own standing rule (restrict/remove only on a *confirmed*
defect, never pattern-matching), no additional cases were hand-authored
toward that target this session -- inventing plausible-but-unverified
wrong attributions would produce a true-reject number that looks measured
but isn't. Expanding this corpus is real, separate chemistry-verification
work, not something to rush.

## Assessment

**Two separate, non-conflated findings** (same discipline this project
applies throughout):

1. **True-accept, `validate_step` overall**: 46.1% of real, correct USPTO
   disconnections validate under *some* rule attribution -- but only
   2.4% via the rule-specific SMIRKS branch. The other 43.7% comes from a
   coarse, deliberately-shared delta check that (confirmed directly) also
   validates *wrong* attributions within the same bucket just as readily.
   **The 46.1% headline number substantially overstates how much of it
   reflects genuine rule-specific discrimination.**
2. **True-reject, graph branch**: for the five rule names sharing
   `ESTER_AMIDE_DELTA`/`SULFONYL_DELTA`, a wrong-but-same-shape
   attribution is structurally unrejectable, by design, not a gap to
   close casually -- unifying or splitting these buckets is a real
   chemistry-validation design question (would a stricter per-rule check
   reject some currently-`Valid` correct attributions too, e.g. via
   stereo/regiochemistry it doesn't currently examine?), out of scope to
   decide here. The one pair that *is* distinguishable (Boc/Cbz)
   correctly discriminates both directions.

## Recommendation

Not a call to action on its own -- reported as a data point for whoever
next touches `validate_graph_step` or cites this project's "validator
accuracy" number. If a stricter, more discriminative graph-rule check is
ever proposed, this measurement's `true_accept_via_smirks_rule` (2.4%)
vs. `true_accept_via_graph_rule_only` (43.7%) split is the baseline to
re-measure against, not the blended 46.1%. No production code changed in
this measurement (design doc §9 stage 5's own "no production code changes
expected... unless the probe surfaces a real, confirmed validator
defect" -- the shared-bucket behavior is confirmed-and-documented-as-
intentional, not a defect to fix here).
