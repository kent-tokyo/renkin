# Corrective verification: L4703 invalid-route-tree fix

## Context

`results_v2/gate_verdict.json` (commit `45baa2f`) recorded the v2 post-anomaly
replacement confirmation's final verdict: **FAIL**, 6/7 criteria passing,
failing only `invalid_zero` on a single target, `uspto50k_test#L4703`. That
verdict is **not** re-labeled or overwritten by anything below -- it stands
as the permanent record of what the v2 run, on the frozen binary
(`f6ff52a9a6c942787d7b5f7c099e0d0f60cc6e57f52f563bd9ca96ffdd0a4250`), actually
produced.

## Root cause

`uspto50k_test#L4703` (`COC(=O)c1ccc[n+]([O-])c1-c1ccc(F)cc1`, a pyridine
N-oxide) solves in Arm C's Stage 2 via a Stille-type biaryl disconnection
template containing a `[#7:5]` hash-atom (wildcard nitrogen). RENKIN's
`[#N]`-expansion (`hash_atom_candidate_symbols`, `src/chem_env.rs`) only ever
offers the neutral spellings `"N"`/`"n"` for nitrogen. Applying that expanded,
literal, always-neutral template atom to the real (charged) substrate builds
the output precursor molecule from the template's literal spelling, not from
the real matched atom -- `run_reactants`'s output atoms carry no `atom_map`,
so there is no way to recover the real atom's charge from the public API
(confirmed by direct inspection). The resulting precursor fragment kept its
real `[O-]` substituent but lost the paired `[n+]`, producing an
unkekulizable, RDKit-unparseable SMILES -- exactly `route_tree_parseable:
false` with `route_found: true`.

## Fix

`src/chem_env.rs`: a new `repair_spectator_oxide_charge` function, applied to
every product molecule in `apply_retro`'s hash-atom-variant path, restores
one narrow, ring-size-independent invariant: an aromatic N/P bonded
(non-aromatically) to a negatively-charged exocyclic O/S must itself carry a
positive charge. It keys only on that O/S⁻ neighbor, not on substituent
degree or ring size, so it cannot fire on N-alkyl/N-aryl-substituted azoles
(N-methylpyrrole, N-methylindole, N-arylpyrrole, caffeine's N-methyl, ...) --
guarded explicitly by regression tests.

Does **not** touch `chematic` (external crate, pinned at 0.11.0, unmodified)
and does **not** relax `aromaticity_integrity_violation`, exclude the target,
or force `route_found=false` to hide the defect -- the route is still found,
now with a chemically valid precursor.

## Verification

1. **Direct reproduction**: `renkin --target 'COC(=O)c1ccc[n+]([O-])c1-c1ccc(F)cc1' --max-routes 1` with the exact Arm C flags now produces a precursor
   fragment (`c1[n+]([O-])cccc1C(OC)=O.C(CC)C.C(CC)C.C(C)CC.[Sn]`) that parses
   under both RDKit and `scripts/compare_route_graph.py`'s own validator
   (`route_tree_parseable: true`, `defects: []`).
2. **Unit tests** (`src/chem_env.rs`, 7 new tests): the repair itself, its
   idempotence on already-correct N-oxides, three false-positive guards
   (N-methylpyrrole/indole/arylpyrrole stay unchanged), and an end-to-end
   `apply_retro` regression on the exact L4703 SMIRKS. Full suite: 473 lib
   tests + all integration tests green, `cargo fmt`/`clippy -D warnings`
   clean.
3. **Locality (VAL-scale corpus diff)**: `val25_sample_list.jsonl` (25
   targets, `uspto50k_val` split -- disjoint from the formal-TEST's
   `uspto50k_test` cohort) run through the identical Arm-C-style coverage-mode
   invocation (`--depth 5 --beam-width 100 --coverage-timeout-secs 600`),
   once on the pre-fix binary (`val25_before_rows.jsonl`, first 25 rows of an
   interrupted 100-target sweep) and once on the post-fix binary
   (`val25_after_rows.jsonl`, `binary_sha256:
   b2e0de8c54fae71fe010edf29256fb51c32e03f38e2f52d352b686dcbae2ae6f` in
   `val25_after_manifest.json`). Diff on `route_found` +
   `normalized_route_sha256`: **25/25 match**.

   One target (`uspto50k_val#L2371`) showed `run_status: timeout` in the
   after-run's external wrapper (`--timeout-s 600`, same cap as the before
   run) but not in the before-run. This VAL-scale check did **not** enforce
   the same environment preflight (AC power, load average ≤2.0, no
   competing jobs) that the formal Arm A/C v2 runs did, and the machine was
   observed at load average 5.85/11.71/14.99 during this check. Investigated
   directly: three independent measurements of this exact target all agree
   on the substantive outcome (`route_found=False`, `stage2_timeout=False`)
   and differ only in wall-clock time (469s before-run / 600s+ after-batch-run
   / 576s after-standalone-rerun) -- consistent with uncontrolled system
   contention on an already-borderline-slow target, not a behavioral change.
   `repair_spectator_oxide_charge` is a cheap per-product atom/neighbor scan,
   not a plausible source of 100+ seconds of extra work. This is a locality
   claim about **route output**, not about performance under controlled
   conditions -- performance was not measured under preflight-controlled
   conditions here and no such claim is made.

## Conclusion

The fix is local to the N-oxide/hash-atom-charge defect class. Per
`protocol_v2.md` Section 4 and the explicit corrective-verification decision
this record documents, this does not require re-running the 500-target Arm C.
The original v2 `FAIL` (`45baa2f`) is preserved as-is; this corrective
verification is the basis for proceeding to release once the source fix and
this record are committed and `RELEASE_CANDIDATE_SHA` is re-frozen.
