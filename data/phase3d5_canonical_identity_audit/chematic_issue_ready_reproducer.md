# Issue-ready reproducer: `to_canonical` is not a fixed point across `Molecule` construction paths

**Not filed yet** -- prepared per the user's explicit Phase 3D.5-8 instruction
("このphaseではGitHub Issueをまだ投稿しない"). File against `kent-tokyo/chematic`
only after the user's go-ahead (this is an outward-facing action).

## Environment

- chematic version: `0.11.0` (`source = "registry+https://github.com/rust-lang/crates.io-index"`)
- Pinned via `Cargo.lock` in `kent-tokyo/renkin` at commit `4d8e4853e5b83a0a002a2be0b04bc70cc0962cda`
- `Cargo.lock` SHA-256: `sha256:ada045bd1b65319d8ede433417d50991b5f7866464c522eedca07de7a6775174`

## Summary

`chematic_smiles::canonical_smiles` (wrapped by RENKIN as `chem_env::to_canonical`)
is documented/expected to produce a stable, unique canonical SMILES per
molecular graph. It does not: canonicalizing the **same graph** via two
different valid construction paths -- (1) build a `Molecule` from a mapped
SMILES, then structurally clear the atom-map annotations
(`chematic`'s `Molecule`/`MoleculeBuilder`, RENKIN's `clear_atom_maps`), vs.
(2) parse the *canonical SMILES text that path (1) produced* fresh via the
normal SMILES parser -- yields two **different**, but each individually
stable/idempotent, canonical strings for the identical molecule. No atom
maps are present in path (2) at all, so this is not simply "atom-map
presence changes the tie-break" (an earlier internal writeup mischaracterized
it that way; corrected here).

## Minimal reproducer

```
MAPPED='[CH3:1][N:2]1[CH2:3][c:4]2[cH:5][c:6]([Cl:7])[cH:8][cH:9][c:10]2-[n:11]2[c:12]([Br:13])[n:14][n:15][c:16]2[CH2:17]1'

# Path 1: parse mapped SMILES, structurally clear atom maps, canonicalize.
STEP1=$(echo "$MAPPED" | renkin-canonicalize --clear-atom-maps)
echo "$STEP1"
# => n12c(nnc1CN(C)Cc3cc(ccc23)Cl)Br

# Path 2: take STEP1's own (map-free) output text, parse it FRESH, canonicalize again.
STEP2=$(echo "$STEP1" | renkin-canonicalize)
echo "$STEP2"
# => N3(C)Cc1n(c2ccc(cc2C3)Cl)c(nn1)Br     <-- DIFFERS from STEP1

# Confirm STEP2 is itself a stable fixed point (not further non-determinism).
STEP3=$(echo "$STEP2" | renkin-canonicalize)
echo "$STEP3"
# => N3(C)Cc1n(c2ccc(cc2C3)Cl)c(nn1)Br     <-- STEP2 == STEP3
```

`renkin-canonicalize` is a thin CLI wrapper: `mol_from_smiles(input)` then,
optionally, `clear_atom_maps(&mol)`, then `to_canonical(&mol)`
(`chematic_smiles::canonical_smiles`). See
`src/bin/canonicalize.rs` in the `renkin` repo for the exact 15-line
wrapper if a chematic-only reproducer (bypassing RENKIN entirely) is
preferred -- the same two-line call sequence (`Molecule::from_smiles` /
equivalent parse, then `canonical_smiles`) should reproduce this directly
against `chematic_smiles` and `chematic_chem` alone.

## Independent confirmation the two forms are the same molecule

RDKit (`2026.03.4`), an independent toolkit, confirms `STEP1` and `STEP2`
denote the identical molecule including identical absolute stereochemistry
(same InChI). This was re-confirmed at scale in a downstream audit: of 2,245
analogous divergent SMILES pairs (drawn from real retrosynthesis candidate
precursors, not hand-picked) that RDKit could parse, **100%** had identical
InChI on both sides. This is not a graph-corruption bug -- it is a
canonical-form non-uniqueness bug: the same graph has (at least) two
distinct, individually-stable canonical representations depending on
construction path.

## Scale / frequency (measured on real data, not synthetic)

Across three independent corpora (39,668 unique TRAIN targets, 4,924 unique
VAL targets, 4,903 unique quarantined TEST targets from the USPTO-50k
retrosynthesis benchmark), re-canonicalizing each already-canonical
(map-free) target string once more:

| corpus | n | n differing from input | rate |
|---|---|---|---|
| TRAIN | 39,668 | 109 | 0.275% |
| VAL | 4,924 | 14 | 0.284% |
| TEST | 4,903 | 13 | 0.265% |

Every one of the 136 (109+14+13) differing cases stabilizes after exactly
one re-canonicalization pass (`A -> B -> B`, never `A -> B -> C` requiring a
third pass) -- consistent with "two fixed points reachable from different
starting representations," not unbounded drift.

The same phenomenon reproduces on retrosynthesis-candidate precursor SMILES
(a completely different, non-target-corpus sample): re-canonicalizing
985,896 unique candidate precursor strings (TRAIN) and 131,163 (VAL) found
1,959 (0.199%) and 324 (0.247%) differing respectively -- same order of
magnitude, same single-drift-then-stable pattern. ~99% of these are pure
stereo-descriptor (`@`/`@@`) re-encodings of an unchanged atom/ring
ordering (RDKit-confirmed same absolute configuration on both sides where
parseable); the remainder show the same full-reordering pattern as the
target-string cases.

## Suspected contributing area (not confirmed -- for chematic maintainers to investigate)

`chematic_smiles::canonical_partition::VertexColor.atom_map`
(`canonical_partition.rs:66,185`) folds `atom.atom_map` into vertex coloring
used to prune redundant individualize-refine branches during canonicalization
search. Whether `clear_atom_maps`'s `MoleculeBuilder`-rebuilt `Molecule`
leaves this (or some other derived/cached state populated by the normal
parser's post-parse perception pass -- e.g. ring perception) in a state that
differs from a molecule obtained by parsing plain SMILES text fresh is an
open question this reproducer does not resolve. Flagging as a starting
point, not a diagnosis.

## Impact statement (for the issue body, once filed)

Low practical impact at current corpus scale (~0.2-0.3% of identities
affected, no chirality-flip observed, all confirmed same-molecule by an
independent toolkit) but a real violation of the "canonical SMILES is a
pure function of the graph" contract that downstream identity-based logic
(exact-string target/precursor matching, benchmark decontamination,
cross-split dedup) implicitly assumes.
