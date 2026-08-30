use std::fs;
#[cfg(feature = "perf-instrumentation")]
use std::sync::atomic::{AtomicU64, Ordering};

use rustc_hash::{FxHashMap, FxHashSet};

use anyhow::{Context, Result};
use chematic::chem::standardize::{StandardizeOptions, ZwitterionHandling, standardize};
use chematic::core::{Atom, AtomIdx, BondIdx, BondOrder, Element, MoleculeBuilder};
use chematic::rxn::run_reactants;
use chematic::smarts::parse_smarts;
use chematic::smiles::{canonical_smiles, parse};
use sha2::{Digest, Sha256};

pub use chematic::core::Molecule;

#[derive(Debug, Clone)]
pub struct RetroRule {
    pub name: String,
    /// Stable identity, independent of file position/order/count and of the
    /// `name` display string. Hand-crafted rules: `rule:<name>`. Extracted
    /// templates: `smirks-sha256:<hex>` (see `template_id_for_smirks`).
    pub template_id: String,
    /// SMIRKS in "reactant>>product1.product2" form (retro direction).
    pub smirks: String,
    /// Log-frequency weight from USPTO training data. Hand-crafted rules use 1.0 (neutral).
    /// Extracted templates use ln(count + 1) — higher = more frequent in training set.
    pub weight: f64,
    /// Bitmask of required atomic numbers (bit N set ⟺ element N must appear in the target).
    /// Zero means no pre-screening (always attempt). Set at load time from SMIRKS or rule name.
    pub required_elements: u64,
}

impl Default for RetroRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            template_id: String::new(),
            smirks: String::new(),
            weight: 1.0,
            required_elements: 0,
        }
    }
}

/// Stable identity for an extracted SMIRKS template: SHA-256 of the *trimmed*
/// SMIRKS string, hex-encoded, formatted as `smirks-sha256:<hex>`. Independent
/// of file position, load order, and count. Purely syntactic — no SMIRKS
/// canonicalization is performed, so two semantically-equivalent SMIRKS written
/// differently (e.g. different atom-map numbering) get different IDs.
pub fn template_id_for_smirks(smirks: &str) -> String {
    let digest = Sha256::digest(smirks.trim().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("smirks-sha256:{hex}")
}

/// Building-block library.
///
/// Stock membership is an exact-identity check: every entry is standardized
/// (see `STANDARDIZE_OPTS` — explicit H removed, tautomers and charges left
/// as written, stereo preserved) and stored by canonical SMILES. A query
/// molecule is a stock hit iff its canonicalized-and-standardized form
/// exactly matches an entry — never a subgraph/substructure match. This
/// project previously used a VF2 subgraph-isomorphism fallback here; it was
/// removed because full-coverage subgraph matches do not imply molecular
/// identity, which produced false-positive stock hits (see the
/// `stock membership must not use subgraph matching` tests below).
pub struct ChemEnv {
    /// Standardized canonical SMILES of every BB — the sole identity lookup.
    canon_set: FxHashSet<String>,
    bb_count: usize,
}

impl ChemEnv {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read building blocks from {path}"))?;
        let smiles_iter = content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|line| line.split_whitespace().next().map(str::to_owned));
        Ok(Self::from_smiles_iter(smiles_iter))
    }

    pub fn in_memory(smiles_list: &[&str]) -> Self {
        Self::from_smiles_iter(smiles_list.iter().map(|s| s.to_string()))
    }

    fn from_smiles_iter(iter: impl Iterator<Item = String>) -> Self {
        let mut canon_set: FxHashSet<String> = FxHashSet::default();
        let mut bb_count = 0usize;

        for smiles in iter {
            let Ok(mol) = parse(&smiles) else { continue };
            let canon = canonical_stock_identity(&mol);
            if !canon_set.insert(canon) {
                continue; // duplicate
            }
            bb_count += 1;
        }

        Self {
            canon_set,
            bb_count,
        }
    }

    /// Number of building blocks in the library.
    pub fn bb_count(&self) -> usize {
        self.bb_count
    }

    /// Fast O(1) BB check for an already-canonical, already-standardized
    /// SMILES string. Skips molecule parsing/standardization/re-canonicalization.
    /// Use this only when the input is guaranteed to already be in that exact
    /// form (e.g. `FEntry.smiles` in search, which is produced by
    /// `split_fragments`'s `standardize` + `canonical_smiles` pipeline).
    pub fn is_building_block_smiles(&self, canonical_smi: &str) -> bool {
        self.canon_set.contains(canonical_smi)
    }

    /// Check if `mol` is in the building-block library.
    ///
    /// Exact-identity check only: standardizes `mol` with the same policy
    /// used to build `canon_set`, then does an O(1) canonical-SMILES lookup.
    /// No subgraph/substructure matching — a partial match is not membership.
    pub fn is_building_block(&self, mol: &Molecule) -> bool {
        self.canon_set.contains(&canonical_stock_identity(mol))
    }

    /// Content hash over every canonical BB SMILES (sorted before hashing,
    /// so it is order-independent) -- distinct from a caller-supplied
    /// `stock_identity` label: a manifest that only records a label can't
    /// tell a real stock swap apart from a re-labeled identical stock, or a
    /// silently-truncated one under the same label. This hashes what's
    /// actually IN the stock.
    pub fn content_sha256(&self) -> String {
        let mut sorted: Vec<&str> = self.canon_set.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let mut hasher = Sha256::new();
        hasher.update(b"renkin-retrospect-stock-v1\0");
        hasher.update((sorted.len() as u64).to_be_bytes());
        for smi in sorted {
            hasher.update((smi.len() as u64).to_be_bytes());
            hasher.update(smi.as_bytes());
        }
        format!("sha256:{}", crate::sha256_hex(hasher.finalize()))
    }
}

pub fn mol_from_smiles(smiles: &str) -> Result<Molecule> {
    parse(smiles).with_context(|| format!("Failed to parse SMILES: {smiles}"))
}

pub fn to_canonical(mol: &Molecule) -> String {
    canonical_smiles(mol)
}

/// Structurally clear every atom's `atom_map`, leaving every other property
/// (element, charge, isotope, aromaticity, chirality, hydrogen count,
/// wildcard, stereo groups, stereo neighbor order, bond directions, bond
/// orders, ring topology) untouched.
///
/// Ground-truth label pipelines that need map-free canonical SMILES from
/// mapped reaction data (e.g. `scripts/generate_real_labels.py`) must clear
/// atom maps this way, not via string manipulation on the SMILES text --
/// `:` is also SMILES bond syntax (explicit aromatic/double bonds), so a
/// text-level `:\d+` strip can corrupt a ring-closure digit that follows an
/// explicit bond symbol instead of an atom map (see the regression tests
/// below for a concrete case). Rebuilds via [`MoleculeBuilder`] rather than
/// mutating in place because [`Molecule`] exposes no atom mutator for
/// `atom_map` (only `set_charge`/`set_isotope`/`set_element`/etc. -- by
/// design, atom maps are meant to be set once during parsing/reaction
/// construction, not edited post hoc).
///
/// Atoms and bonds are re-added in exactly the same order as `mol` yields
/// them, so atom/bond indices are unchanged and `stereo_groups`/
/// `stereo_neighbor_order`/`bond_directions` (all keyed by index) can be
/// copied over verbatim -- see `MoleculeBuilder::from_molecule`'s own doc
/// comment for why that invariant is what makes the verbatim copy valid.
pub fn clear_atom_maps(mol: &Molecule) -> Molecule {
    let mut builder = MoleculeBuilder::new();
    for (_, atom) in mol.atoms() {
        let mut a = atom.clone();
        a.atom_map = None;
        builder.add_atom(a);
    }
    for (_, bond) in mol.bonds() {
        let _ = builder.add_bond(bond.atom1, bond.atom2, bond.order);
    }
    builder.copy_stereo_groups_from(mol);
    builder.copy_stereo_from(mol);
    builder.copy_bond_directions_from(mol);
    builder.build()
}

#[cfg(test)]
mod clear_atom_maps_tests {
    use super::*;

    fn canon_after_clear(smiles: &str) -> String {
        let mol = mol_from_smiles(smiles).unwrap_or_else(|e| panic!("{smiles}: {e}"));
        to_canonical(&clear_atom_maps(&mol))
    }

    #[test]
    fn normal_mapped_atom_matches_unmapped_canonical() {
        assert_eq!(
            canon_after_clear("[CH3:1]O"),
            to_canonical(&mol_from_smiles("CO").unwrap())
        );
    }

    #[test]
    fn multi_digit_atom_map_matches_unmapped_canonical() {
        assert_eq!(
            canon_after_clear("[CH3:123]O"),
            to_canonical(&mol_from_smiles("CO").unwrap())
        );
    }

    #[test]
    fn isotope_survives_map_clearing() {
        assert_eq!(
            canon_after_clear("[13CH4:1]"),
            to_canonical(&mol_from_smiles("[13CH4]").unwrap())
        );
    }

    #[test]
    fn formal_charge_survives_map_clearing() {
        assert_eq!(
            canon_after_clear("[NH4+:1]"),
            to_canonical(&mol_from_smiles("[NH4+]").unwrap())
        );
    }

    #[test]
    fn tetrahedral_stereo_survives_map_clearing() {
        assert_eq!(
            canon_after_clear("[C@H:1](F)(Cl)Br"),
            to_canonical(&mol_from_smiles("[C@H](F)(Cl)Br").unwrap())
        );
        // Also confirm it's not just "some canonical form" but the correct
        // one -- the mirror-image @@ stereocenter must differ.
        assert_ne!(
            canon_after_clear("[C@H:1](F)(Cl)Br"),
            canon_after_clear("[C@@H:1](F)(Cl)Br")
        );
    }

    #[test]
    fn aromatic_atoms_survive_map_clearing() {
        assert_eq!(
            canon_after_clear("[cH:1]1ccccc1"),
            to_canonical(&mol_from_smiles("c1ccccc1").unwrap())
        );
    }

    #[test]
    fn disconnected_fragments_survive_map_clearing() {
        assert_eq!(
            canon_after_clear("[CH3:1]O.[Na+:2]"),
            to_canonical(&mol_from_smiles("CO.[Na+]").unwrap())
        );
    }

    #[test]
    fn already_unmapped_smiles_is_a_no_op() {
        assert_eq!(
            canon_after_clear("CC(=O)O"),
            to_canonical(&mol_from_smiles("CC(=O)O").unwrap())
        );
    }

    /// Pins the exact failure mode a text-level `re.sub(r":\d+", "", smiles)`
    /// atom-map strip has: `:` is also explicit SMILES bond syntax, so a
    /// mapped aromatic ring written with explicit `:` bonds and a `:`-led
    /// ring-closure digit gets its ring closure silently deleted along with
    /// the atom map, corrupting the ring into an open chain.
    #[test]
    fn explicit_colon_bond_with_ring_closure_digit_is_not_corrupted() {
        // Benzene, atom-mapped, every ring bond written with an explicit
        // aromatic `:` bond symbol (including the closing ring bond, whose
        // digit immediately follows a `:`).
        let mapped = "[cH:1]:1:c:c:c:c:c:1";
        let benzene = to_canonical(&mol_from_smiles("c1ccccc1").unwrap());
        assert_eq!(
            canon_after_clear(mapped),
            benzene,
            "structural atom-map clearing must keep the ring closed"
        );

        // The old regex's actual output on this exact input, replayed
        // structurally: `:\d+` strips both the atom-map `:1` and the
        // ring-closure `:1`, leaving `[cH]:c:c:c:c:c` -- an open chain, a
        // different molecule entirely. If this ever equals `benzene`, the
        // fixture stopped being a real regression pin and must be replaced.
        let regex_corrupted = to_canonical(&mol_from_smiles("[cH]:c:c:c:c:c").unwrap());
        assert_ne!(
            regex_corrupted, benzene,
            "fixture no longer demonstrates the regex-corruption failure mode"
        );
    }

    #[test]
    fn heavy_atom_count_is_unchanged_by_map_clearing() {
        let mol = mol_from_smiles("[CH3:1][CH2:2][OH:3]").unwrap();
        let cleared = clear_atom_maps(&mol);
        assert_eq!(mol.atoms().count(), cleared.atoms().count());
        assert_eq!(mol.bonds().count(), cleared.bonds().count());
        assert!(cleared.atoms().all(|(_, a)| a.atom_map.is_none()));
    }
}

pub(crate) static STANDARDIZE_OPTS: StandardizeOptions = StandardizeOptions {
    canonical_tautomer: false,
    neutralize_charges: false,
    remove_explicit_h: true,
    largest_fragment_only: false,
    zwitterion_handling: ZwitterionHandling::Keep,
};

/// The single, shared stock-identity policy: standardize (see
/// `STANDARDIZE_OPTS`'s doc comment for exactly what that does and doesn't
/// fold together), then canonicalize. This is the *only* function that
/// decides "what counts as the same molecule for stock-membership
/// purposes" — `ChemEnv::is_building_block`/`from_smiles_iter` and any
/// other consumer that needs an independent stock-identity check (e.g. the
/// Synthesizability Kernel, `src/synthesizability/signals.rs`) must call
/// this rather than hand-duplicating `STANDARDIZE_OPTS`, so the policy can
/// only ever drift by being changed here.
pub(crate) fn canonical_stock_identity(mol: &Molecule) -> String {
    canonical_smiles(&standardize(mol, &STANDARDIZE_OPTS))
}

/// Parses `smiles` and applies [`canonical_stock_identity`]. `Err` if the
/// SMILES doesn't parse -- callers that need to distinguish "doesn't parse"
/// from "parses but isn't a stock hit" should match on the `Result` rather
/// than collapsing both to `false`/`None`.
pub(crate) fn canonical_stock_identity_from_smiles(smiles: &str) -> Result<String> {
    let mol = parse(smiles).with_context(|| format!("Failed to parse SMILES: {smiles}"))?;
    Ok(canonical_stock_identity(&mol))
}

// ── Graph-based Ar-Ar bond cleavage (Suzuki retro) ─────────────────────────
//
// chematic's run_reactants seeds BFS globally, so applying the SMIRKS
// [c:1][c:2]>>[c:1]Br.[c:2] to biphenyl produces broken fragments like
// c(Br)(-c1ccccc1)cccc instead of clean Brc1ccccc1 + c1ccccc1.
// We work around this by computing the two connected components directly
// from the molecular graph using MoleculeBuilder.

/// Test whether removing the bond (a, b) disconnects the graph (i.e., it is a bridge bond).
pub(crate) fn is_bridge_bond(mol: &Molecule, a: AtomIdx, b: AtomIdx) -> bool {
    // BFS from `a`, skipping the direct a→b edge. If b is not reachable → bridge.
    let mut visited = FxHashSet::default();
    let mut stack = vec![a];
    visited.insert(a);
    while let Some(cur) = stack.pop() {
        for (neighbor, _) in mol.neighbors(cur) {
            if cur == a && neighbor == b {
                continue;
            }
            if visited.insert(neighbor) {
                stack.push(neighbor);
            }
        }
    }
    !visited.contains(&b)
}

/// Collect all atoms reachable from `start` when the bond (bridge_a, bridge_b) is removed.
fn get_component(
    mol: &Molecule,
    start: AtomIdx,
    bridge_a: AtomIdx,
    bridge_b: AtomIdx,
) -> FxHashSet<AtomIdx> {
    let mut visited = FxHashSet::default();
    let mut stack = vec![start];
    visited.insert(start);
    while let Some(cur) = stack.pop() {
        for (neighbor, _) in mol.neighbors(cur) {
            if (cur == bridge_a && neighbor == bridge_b)
                || (cur == bridge_b && neighbor == bridge_a)
            {
                continue;
            }
            if visited.insert(neighbor) {
                stack.push(neighbor);
            }
        }
    }
    visited
}

/// Build a sub-molecule from a set of atom indices, preserving all intra-set bonds.
fn build_sub_molecule(mol: &Molecule, atoms: &FxHashSet<AtomIdx>) -> Option<Molecule> {
    let mut builder = MoleculeBuilder::new();
    let mut idx_map: FxHashMap<AtomIdx, AtomIdx> = FxHashMap::default();

    for &old_idx in atoms {
        let new_idx = builder.add_atom(mol.atom(old_idx).clone());
        idx_map.insert(old_idx, new_idx);
    }
    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);
        if atoms.contains(&a) && atoms.contains(&b) {
            let (&new_a, &new_b) = (idx_map.get(&a)?, idx_map.get(&b)?);
            builder.add_bond(new_a, new_b, bond.order).ok()?;
        }
    }
    Some(builder.build())
}

/// Build a sub-molecule and append a Br atom bonded to `cut_atom`.
fn build_sub_molecule_with_br(
    mol: &Molecule,
    atoms: &FxHashSet<AtomIdx>,
    cut_atom: AtomIdx,
) -> Option<Molecule> {
    let mut builder = MoleculeBuilder::new();
    let mut idx_map: FxHashMap<AtomIdx, AtomIdx> = FxHashMap::default();

    for &old_idx in atoms {
        let new_idx = builder.add_atom(mol.atom(old_idx).clone());
        idx_map.insert(old_idx, new_idx);
    }
    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);
        if atoms.contains(&a) && atoms.contains(&b) {
            let (&new_a, &new_b) = (idx_map.get(&a)?, idx_map.get(&b)?);
            builder.add_bond(new_a, new_b, bond.order).ok()?;
        }
    }
    // Add Br single-bonded to the cut site
    let br_idx = builder.add_atom(Atom::new(Element::BR));
    let &cut_new = idx_map.get(&cut_atom)?;
    builder.add_bond(cut_new, br_idx, BondOrder::Single).ok()?;
    Some(builder.build())
}

/// Build a sub-molecule and append a Cl atom bonded to `cut_atom`.
fn build_sub_molecule_with_cl(
    mol: &Molecule,
    atoms: &FxHashSet<AtomIdx>,
    cut_atom: AtomIdx,
) -> Option<Molecule> {
    let mut builder = MoleculeBuilder::new();
    let mut idx_map: FxHashMap<AtomIdx, AtomIdx> = FxHashMap::default();

    for &old_idx in atoms {
        let new_idx = builder.add_atom(mol.atom(old_idx).clone());
        idx_map.insert(old_idx, new_idx);
    }
    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);
        if atoms.contains(&a) && atoms.contains(&b) {
            let (&new_a, &new_b) = (idx_map.get(&a)?, idx_map.get(&b)?);
            builder.add_bond(new_a, new_b, bond.order).ok()?;
        }
    }
    let cl_idx = builder.add_atom(Atom::new(Element::CL));
    let &cut_new = idx_map.get(&cut_atom)?;
    builder.add_bond(cut_new, cl_idx, BondOrder::Single).ok()?;
    Some(builder.build())
}

/// Build a sub-molecule and append a boronic acid (-B(OH)2) bonded to
/// `cut_atom` -- the boron gets one bond to `cut_atom` plus two bonds to
/// O atoms, each O picking up an implicit H from valence (same
/// implicit-H-from-valence mechanism `build_sub_molecule_with_br`/
/// `with_cl` already rely on for their own appended atom).
fn build_sub_molecule_with_boronic_acid(
    mol: &Molecule,
    atoms: &FxHashSet<AtomIdx>,
    cut_atom: AtomIdx,
) -> Option<Molecule> {
    let mut builder = MoleculeBuilder::new();
    let mut idx_map: FxHashMap<AtomIdx, AtomIdx> = FxHashMap::default();

    for &old_idx in atoms {
        let new_idx = builder.add_atom(mol.atom(old_idx).clone());
        idx_map.insert(old_idx, new_idx);
    }
    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);
        if atoms.contains(&a) && atoms.contains(&b) {
            let (&new_a, &new_b) = (idx_map.get(&a)?, idx_map.get(&b)?);
            builder.add_bond(new_a, new_b, bond.order).ok()?;
        }
    }
    let b_idx = builder.add_atom(Atom::new(Element::B));
    let &cut_new = idx_map.get(&cut_atom)?;
    builder.add_bond(cut_new, b_idx, BondOrder::Single).ok()?;
    for _ in 0..2 {
        let o_idx = builder.add_atom(Atom::new(Element::O));
        builder.add_bond(b_idx, o_idx, BondOrder::Single).ok()?;
    }
    Some(builder.build())
}

/// Graph-based retro for Ar-SO2-Ar diaryl sulfones:
/// cleave each Ar-S bridge bond to give [Ar-SO2-Cl, Ar'-H].
fn diaryl_sulfone_cleavage(mol: &Molecule) -> Vec<Vec<PrecursorMol>> {
    let mut results: Vec<Vec<PrecursorMol>> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);

        // One end must be aromatic C, the other must be S
        let (ar_idx, s_idx) = {
            let atom_a = mol.atom(a);
            let atom_b = mol.atom(b);
            if atom_a.element == Element::S && atom_b.aromatic && atom_b.element == Element::C {
                (b, a)
            } else if atom_b.element == Element::S
                && atom_a.aromatic
                && atom_a.element == Element::C
            {
                (a, b)
            } else {
                continue;
            }
        };

        // S must be a sulfone: at least two double bonds to O
        let o_double_count = mol
            .neighbors(s_idx)
            .filter(|&(nb, bond_idx): &(AtomIdx, BondIdx)| {
                mol.atom(nb).element == Element::O && mol.bond(bond_idx).order == BondOrder::Double
            })
            .count();
        if o_double_count < 2 {
            continue;
        }

        // Must be a bridge bond
        if !is_bridge_bond(mol, ar_idx, s_idx) {
            continue;
        }

        let comp_ar = get_component(mol, ar_idx, ar_idx, s_idx); // Ar' side (gets H)
        let comp_s = get_component(mol, s_idx, ar_idx, s_idx); // Ar-SO2 side (gets Cl)

        let Some(frag_arh) = build_sub_molecule(mol, &comp_ar) else {
            continue;
        };
        let Some(frag_so2cl) = build_sub_molecule_with_cl(mol, &comp_s, s_idx) else {
            continue;
        };

        let precs_arh = split_fragments(&frag_arh);
        let precs_so2cl = split_fragments(&frag_so2cl);
        if precs_arh.is_empty() || precs_so2cl.is_empty() {
            continue;
        }

        let mut key_parts: Vec<&str> = precs_arh
            .iter()
            .chain(precs_so2cl.iter())
            .map(|p| p.smiles.as_str())
            .collect();
        key_parts.sort_unstable();
        let key = key_parts.join("|");
        if !seen.insert(key) {
            continue;
        }

        let mut prec_set = precs_arh;
        prec_set.extend(precs_so2cl);
        results.push(prec_set);
    }
    results
}

/// Graph-based retro-Suzuki: cleave every Ar–Ar bridge bond and return
/// [Ar-Br, Ar'] and [Ar, Ar'-Br] precursor sets.
fn biaryl_cleavage(mol: &Molecule) -> Vec<Vec<PrecursorMol>> {
    let mut results: Vec<Vec<PrecursorMol>> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);

        // Both endpoints must be aromatic carbon
        let atom_a = mol.atom(a);
        let atom_b = mol.atom(b);
        if !atom_a.aromatic || atom_a.element != Element::C {
            continue;
        }
        if !atom_b.aromatic || atom_b.element != Element::C {
            continue;
        }

        // Must be a bridge bond (not inside any ring)
        if !is_bridge_bond(mol, a, b) {
            continue;
        }

        let comp_a = get_component(mol, a, a, b);
        let comp_b = get_component(mol, b, a, b);

        // Generate both orientations: which ring gets Br vs. the boronic
        // acid -- a real retro-Suzuki needs one aryl-halide partner *and*
        // one boron-containing partner, not "aryl halide + plain arene".
        for (comp_br, cut_br, comp_b_acid, cut_b_acid) in
            [(&comp_a, a, &comp_b, b), (&comp_b, b, &comp_a, a)]
        {
            let Some(frag_br) = build_sub_molecule_with_br(mol, comp_br, cut_br) else {
                continue;
            };
            let Some(frag_plain) =
                build_sub_molecule_with_boronic_acid(mol, comp_b_acid, cut_b_acid)
            else {
                continue;
            };

            let precs_br = split_fragments(&frag_br);
            let precs_plain = split_fragments(&frag_plain);
            if precs_br.is_empty() || precs_plain.is_empty() {
                continue;
            }

            // De-duplicate identical orientations (e.g. symmetric biaryls)
            let mut key_parts: Vec<&str> = precs_br
                .iter()
                .chain(precs_plain.iter())
                .map(|p| p.smiles.as_str())
                .collect();
            key_parts.sort_unstable();
            let key = key_parts.join("|");
            if !seen.insert(key) {
                continue;
            }

            let mut prec_set = precs_br;
            prec_set.extend(precs_plain);
            results.push(prec_set);
        }
    }
    results
}

/// Graph-based amide cleavage: C(=O)-N → carboxylic acid + amine.
///
/// Uses graph splitting to avoid BFS-leakage from chematic's run_reactants,
/// which duplicates unmapped atoms into both product templates.
fn amide_cleavage(mol: &Molecule) -> Vec<Vec<PrecursorMol>> {
    let mut results: Vec<Vec<PrecursorMol>> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);
        if bond.order != BondOrder::Single {
            continue;
        }

        // Identify which end is the carbonyl C and which is N.
        let (c_idx, n_idx) = {
            let aa = mol.atom(a);
            let ab = mol.atom(b);
            if aa.element == Element::C && ab.element == Element::N {
                (a, b)
            } else if aa.element == Element::N && ab.element == Element::C {
                (b, a)
            } else {
                continue;
            }
        };

        // The carbon must have an adjacent double-bond O (i.e. be a carbonyl C).
        let has_keto_o = mol.neighbors(c_idx).any(|(nb, bond_idx)| {
            nb != n_idx
                && mol.atom(nb).element == Element::O
                && mol.bond(bond_idx).order == BondOrder::Double
        });
        if !has_keto_o {
            continue;
        }

        // Only bridge bonds produce two clean fragments.
        if !is_bridge_bond(mol, c_idx, n_idx) {
            continue;
        }

        let comp_c = get_component(mol, c_idx, c_idx, n_idx);
        let comp_n = get_component(mol, n_idx, c_idx, n_idx);

        // C side: add explicit OH to mimic carboxylic acid.
        let Some(frag_acid) = build_sub_molecule_with_oh(mol, &comp_c, c_idx) else {
            continue;
        };
        let Some(frag_amine) = build_sub_molecule(mol, &comp_n) else {
            continue;
        };

        let precs_acid = split_fragments(&frag_acid);
        let precs_amine = split_fragments(&frag_amine);
        if precs_acid.is_empty() || precs_amine.is_empty() {
            continue;
        }

        let mut key_parts: Vec<&str> = precs_acid
            .iter()
            .chain(precs_amine.iter())
            .map(|p| p.smiles.as_str())
            .collect();
        key_parts.sort_unstable();
        let key = key_parts.join("|");
        if !seen.insert(key) {
            continue;
        }

        let mut prec_set = precs_acid;
        prec_set.extend(precs_amine);
        results.push(prec_set);
    }
    results
}

/// Graph-based ester cleavage: R-C(=O)-O-R' → carboxylic acid + alcohol/phenol.
///
/// Mirrors amide_cleavage but cuts C-O instead of C-N.
/// Avoids BFS-leakage that affects the SMIRKS version of this rule.
/// Skips terminal -OH (free carboxylic acids) by checking the O-side component size.
fn ester_cleavage_graph(mol: &Molecule) -> Vec<Vec<PrecursorMol>> {
    let mut results: Vec<Vec<PrecursorMol>> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);
        if bond.order != BondOrder::Single {
            continue;
        }

        // Identify which end is the carbonyl C and which is the ester O.
        let (c_idx, o_idx) = {
            let aa = mol.atom(a);
            let ab = mol.atom(b);
            if aa.element == Element::C && ab.element == Element::O {
                (a, b)
            } else if aa.element == Element::O && ab.element == Element::C {
                (b, a)
            } else {
                continue;
            }
        };

        // The carbon must be a carbonyl C (has adjacent C=O, not the O we're cutting).
        let has_keto_o = mol.neighbors(c_idx).any(|(nb, bond_idx)| {
            nb != o_idx
                && mol.atom(nb).element == Element::O
                && mol.bond(bond_idx).order == BondOrder::Double
        });
        if !has_keto_o {
            continue;
        }

        // Only bridge bonds produce two clean fragments.
        if !is_bridge_bond(mol, c_idx, o_idx) {
            continue;
        }

        let comp_c = get_component(mol, c_idx, c_idx, o_idx);
        let comp_o = get_component(mol, o_idx, c_idx, o_idx);

        // Skip free carboxylic acids: the O side has only the O atom itself (terminal -OH).
        if comp_o.len() <= 1 {
            continue;
        }

        // C side: add OH → carboxylic acid fragment.
        let Some(frag_acid) = build_sub_molecule_with_oh(mol, &comp_c, c_idx) else {
            continue;
        };
        // O side: the O keeps its bond to R'; implicit H fills valence → R'-OH.
        let Some(frag_alcohol) = build_sub_molecule(mol, &comp_o) else {
            continue;
        };

        let precs_acid = split_fragments(&frag_acid);
        let precs_alcohol = split_fragments(&frag_alcohol);
        if precs_acid.is_empty() || precs_alcohol.is_empty() {
            continue;
        }

        let mut key_parts: Vec<&str> = precs_acid
            .iter()
            .chain(precs_alcohol.iter())
            .map(|p| p.smiles.as_str())
            .collect();
        key_parts.sort_unstable();
        let key = key_parts.join("|");
        if !seen.insert(key) {
            continue;
        }

        let mut prec_set = precs_acid;
        prec_set.extend(precs_alcohol);
        results.push(prec_set);
    }
    results
}

/// Graph-based aryl ether cleavage: Ar-O-R → Ar-OH + R-OH (retro-Ullmann
/// ether synthesis, simplified "leaving fragment" semantics matching the
/// retired SMIRKS-string version of this rule).
///
/// Excludes an ester oxygen (Ar-O-C(=O)-R): mirrors ester_cleavage_graph's
/// own carbonyl-neighbor check, applied to the *other* carbon the O is
/// bonded to (not the aromatic one being cut). Without this exclusion, an
/// aryl ester's ester bond gets mislabeled as a retro-Ullmann ether
/// disconnection (wrong reaction_family/conditions/procedure_hint) --
/// see docs/design/retro-rule-precision-gaps-v0.md #1.
fn aryl_ether_cleavage(mol: &Molecule) -> Vec<Vec<PrecursorMol>> {
    let mut results: Vec<Vec<PrecursorMol>> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);
        if bond.order != BondOrder::Single {
            continue;
        }

        // Identify which end is the aromatic C and which is O.
        let (ar_idx, o_idx) = {
            let aa = mol.atom(a);
            let ab = mol.atom(b);
            if aa.aromatic && aa.element == Element::C && ab.element == Element::O {
                (a, b)
            } else if ab.aromatic && ab.element == Element::C && aa.element == Element::O {
                (b, a)
            } else {
                continue;
            }
        };

        // Exclude an ester oxygen: O also bonded to a carbonyl carbon (a
        // carbon with an adjacent C=O, other than the O being cut here).
        let is_ester_oxygen = mol.neighbors(o_idx).any(|(nb, _)| {
            nb != ar_idx
                && mol.atom(nb).element == Element::C
                && mol.neighbors(nb).any(|(nb2, bond_idx2)| {
                    nb2 != o_idx
                        && mol.atom(nb2).element == Element::O
                        && mol.bond(bond_idx2).order == BondOrder::Double
                })
        });
        if is_ester_oxygen {
            continue;
        }

        // Only bridge bonds produce two clean fragments.
        if !is_bridge_bond(mol, ar_idx, o_idx) {
            continue;
        }

        let comp_ar = get_component(mol, ar_idx, ar_idx, o_idx);
        let comp_o = get_component(mol, o_idx, ar_idx, o_idx);

        // Aromatic side: add OH → phenol fragment.
        let Some(frag_phenol) = build_sub_molecule_with_oh(mol, &comp_ar, ar_idx) else {
            continue;
        };
        // O side: keeps its existing bond(s); implicit H fills valence → R-OH.
        let Some(frag_alcohol) = build_sub_molecule(mol, &comp_o) else {
            continue;
        };

        let precs_phenol = split_fragments(&frag_phenol);
        let precs_alcohol = split_fragments(&frag_alcohol);
        if precs_phenol.is_empty() || precs_alcohol.is_empty() {
            continue;
        }

        let mut key_parts: Vec<&str> = precs_phenol
            .iter()
            .chain(precs_alcohol.iter())
            .map(|p| p.smiles.as_str())
            .collect();
        key_parts.sort_unstable();
        let key = key_parts.join("|");
        if !seen.insert(key) {
            continue;
        }

        let mut prec_set = precs_phenol;
        prec_set.extend(precs_alcohol);
        results.push(prec_set);
    }
    results
}

/// Graph-based sulfonamide cleavage: Ar-SO2-NHR → Ar-SO2Cl + H2NR.
///
/// Cuts the S-N bond of a sulfonamide where S is a sulfonyl (S(=O)(=O)).
/// Mirrors diaryl_sulfone_cleavage (sulfonyl check) and amide_cleavage (bridge split).
/// Avoids BFS-leakage present in the SMIRKS version.
fn sulfonamide_cleavage_graph(mol: &Molecule) -> Vec<Vec<PrecursorMol>> {
    let mut results: Vec<Vec<PrecursorMol>> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);
        if bond.order != BondOrder::Single {
            continue;
        }

        // Identify which end is S (sulfonyl) and which is N.
        let (s_idx, n_idx) = {
            let aa = mol.atom(a);
            let ab = mol.atom(b);
            if aa.element == Element::S && ab.element == Element::N {
                (a, b)
            } else if aa.element == Element::N && ab.element == Element::S {
                (b, a)
            } else {
                continue;
            }
        };

        // S must be a sulfone: at least two double-bond O neighbours.
        let o_double_count = mol
            .neighbors(s_idx)
            .filter(|&(nb, bond_idx): &(AtomIdx, BondIdx)| {
                mol.atom(nb).element == Element::O && mol.bond(bond_idx).order == BondOrder::Double
            })
            .count();
        if o_double_count < 2 {
            continue;
        }

        // Only bridge bonds produce two clean fragments.
        if !is_bridge_bond(mol, s_idx, n_idx) {
            continue;
        }

        let comp_s = get_component(mol, s_idx, s_idx, n_idx); // Ar-SO2 side (gets Cl)
        let comp_n = get_component(mol, n_idx, s_idx, n_idx); // amine side (gets H)

        let Some(frag_so2cl) = build_sub_molecule_with_cl(mol, &comp_s, s_idx) else {
            continue;
        };
        let Some(frag_amine) = build_sub_molecule(mol, &comp_n) else {
            continue;
        };

        let precs_so2cl = split_fragments(&frag_so2cl);
        let precs_amine = split_fragments(&frag_amine);
        if precs_so2cl.is_empty() || precs_amine.is_empty() {
            continue;
        }

        let mut key_parts: Vec<&str> = precs_so2cl
            .iter()
            .chain(precs_amine.iter())
            .map(|p| p.smiles.as_str())
            .collect();
        key_parts.sort_unstable();
        let key = key_parts.join("|");
        if !seen.insert(key) {
            continue;
        }

        let mut prec_set = precs_so2cl;
        prec_set.extend(precs_amine);
        results.push(prec_set);
    }
    results
}

/// Build a sub-molecule and append an OH group bonded to `cut_atom`.
fn build_sub_molecule_with_oh(
    mol: &Molecule,
    atoms: &FxHashSet<AtomIdx>,
    cut_atom: AtomIdx,
) -> Option<Molecule> {
    let mut builder = MoleculeBuilder::new();
    let mut idx_map: FxHashMap<AtomIdx, AtomIdx> = FxHashMap::default();

    for &old_idx in atoms {
        let new_idx = builder.add_atom(mol.atom(old_idx).clone());
        idx_map.insert(old_idx, new_idx);
    }
    for (_, bond) in mol.bonds() {
        let (a, b) = (bond.atom1, bond.atom2);
        if atoms.contains(&a) && atoms.contains(&b) {
            let (&new_a, &new_b) = (idx_map.get(&a)?, idx_map.get(&b)?);
            builder.add_bond(new_a, new_b, bond.order).ok()?;
        }
    }
    let o_idx = builder.add_atom(Atom::new(Element::O));
    let &cut_new = idx_map.get(&cut_atom)?;
    builder.add_bond(cut_new, o_idx, BondOrder::Single).ok()?;
    Some(builder.build())
}

#[cfg(feature = "perf-instrumentation")]
static APPLY_RETRO_CALLS: AtomicU64 = AtomicU64::new(0);

/// Total number of `apply_retro` calls made so far in this process, for the
/// apply-retro-performance-regression gate. Only tracked when the
/// `perf-instrumentation` feature is enabled (off by default; the default
/// build path never touches this counter, so there's no shared-atomic
/// contention on the `apply_retro` hot path in production).
#[cfg(feature = "perf-instrumentation")]
pub fn apply_retro_call_count() -> u64 {
    APPLY_RETRO_CALLS.load(Ordering::Relaxed)
}

#[cfg(not(feature = "perf-instrumentation"))]
pub fn apply_retro_call_count() -> u64 {
    0
}

/// Reset the counter above (e.g. between gate segments/targets).
#[cfg(feature = "perf-instrumentation")]
pub fn reset_apply_retro_call_count() {
    APPLY_RETRO_CALLS.store(0, Ordering::Relaxed);
}

#[cfg(not(feature = "perf-instrumentation"))]
pub fn reset_apply_retro_call_count() {}

/// Repair a specific charge-loss defect in `[#N]` hash-atom template
/// application (see `hash_atom_candidate_symbols`): expanding a spectator
/// `[#7:m]`/`[#15:m]` into the literal, always-neutral candidate `N`/`n`/
/// `P`/`p` discards the real matched atom's formal charge, because
/// `run_reactants` builds the output atom from the template's literal
/// spelling, not from the real substrate atom it stood in for (confirmed:
/// `run_reactants`'s output atoms never carry `atom_map`, so there is no
/// way to recover the real atom's own charge from the public API -- see
/// the L4703 root-cause investigation, Issue #106).
///
/// The one narrow, ring-size-independent invariant this restores: an
/// aromatic N/P bonded, via a non-aromatic bond, to an exocyclic O/S atom
/// carrying a negative charge, must itself carry a positive charge -- the
/// standard N-oxide/phosphine-oxide-anion charge-separation pair (e.g.
/// pyridine N-oxide's `[n+]...[O-]`). Without the matching `+`, the ring is
/// unkekulizable and round-trips as invalid SMILES in any strict external
/// parser (RDKit), even though chematic's own looser kekulization heuristic
/// (which treats a neutral substituted aromatic N as a lone-pair donor,
/// same as N-alkylpyrrole) accepts it silently.
///
/// Deliberately does NOT key on substituent degree or ring size -- only on
/// the O/S⁻ neighbor -- so it cannot fire on N-alkyl/N-aryl-substituted
/// azoles (pyrrole, indole, caffeine's N-methyl, ...), whose exocyclic
/// substituent is carbon, not an anionic heteroatom.
fn repair_spectator_oxide_charge(mol: &mut Molecule) {
    let mut to_charge: Vec<AtomIdx> = Vec::new();
    for (idx, atom) in mol.atoms() {
        if !atom.aromatic || atom.charge != 0 {
            continue;
        }
        if !matches!(atom.element.atomic_number(), 7 | 15) {
            continue;
        }
        let has_negative_oxide_neighbor = mol.neighbors(idx).any(|(nidx, bidx)| {
            mol.bond(bidx).order != BondOrder::Aromatic
                && matches!(mol.atom(nidx).element.atomic_number(), 8 | 16)
                && mol.atom(nidx).charge < 0
        });
        if has_negative_oxide_neighbor {
            to_charge.push(idx);
        }
    }
    for idx in to_charge {
        mol.set_charge(idx, 1);
    }
}

/// Apply a single retro-rule to a molecule.
/// Returns all possible precursor sets as (canonical_smiles, Molecule) pairs.
///
/// Rules with an empty `smirks` field are dispatched to graph-based handlers
/// (keyed by `name`). SMIRKS rules use chematic's run_reactants; fragments are
/// split on '.' in canonical SMILES and filtered for BFS-leakage artefacts.
pub fn apply_retro(mol: &Molecule, rule: &RetroRule) -> Vec<Vec<PrecursorMol>> {
    #[cfg(feature = "perf-instrumentation")]
    APPLY_RETRO_CALLS.fetch_add(1, Ordering::Relaxed);
    if rule.smirks.is_empty() {
        return match rule.name.as_str() {
            "suzuki_retro" => biaryl_cleavage(mol),
            "diaryl_sulfone_retro" => diaryl_sulfone_cleavage(mol),
            "amide_cleavage" => amide_cleavage(mol),
            "ester_cleavage" => ester_cleavage_graph(mol),
            "aryl_ether_retro" => aryl_ether_cleavage(mol),
            "sulfonamide_retro" => sulfonamide_cleavage_graph(mol),
            "boc_deprotection_retro" => boc_deprotection(mol),
            "cbz_deprotection_retro" => cbz_deprotection(mol),
            _ => vec![],
        };
    }
    if !rule.smirks.contains('#') {
        // Fast path: the ~57% of the corpus with no [#N] atoms, byte-for-
        // byte identical to this function's pre-Issue-88-fix behavior --
        // zero extra allocation or lookup.
        return run_reactants(&rule.smirks, &[mol])
            .unwrap_or_default()
            .into_iter()
            .map(|products| {
                products
                    .into_iter()
                    .flat_map(|product_mol| split_fragments(&product_mol))
                    .collect()
            })
            .collect();
    }

    // [#N] path: try every independently-validated concrete-element
    // reading (see `application_smirks_variants`), deduping outcomes by
    // their resulting precursor SMILES set so two variants that happen to
    // agree on a real molecule don't double-count as separate outcomes.
    let variants = application_smirks_variants(&rule.smirks);
    let mut outcomes: Vec<Vec<PrecursorMol>> = Vec::new();
    let mut seen_signatures: FxHashSet<Vec<String>> = FxHashSet::default();
    for variant in variants.iter() {
        for mut products in run_reactants(variant, &[mol]).unwrap_or_default() {
            for product_mol in products.iter_mut() {
                repair_spectator_oxide_charge(product_mol);
            }
            // Fail-closed: reject the whole outcome if any raw product
            // molecule (before split_fragments' own text round-trip, which
            // can silently repair or reject the same defect -- see
            // `aromaticity_integrity_violation`'s doc comment) fails the
            // aromaticity-integrity check. This is never a partial/best-
            // effort acceptance -- one bad fragment invalidates the outcome.
            if products
                .iter()
                .any(|p| aromaticity_integrity_violation(p).is_some())
            {
                continue;
            }
            let precursors: Vec<PrecursorMol> = products
                .into_iter()
                .flat_map(|product_mol| split_fragments(&product_mol))
                .collect();
            let mut signature: Vec<String> = precursors.iter().map(|p| p.smiles.clone()).collect();
            signature.sort_unstable();
            if seen_signatures.insert(signature) {
                outcomes.push(precursors);
            }
        }
    }
    outcomes
}

/// Returns a clone of `mol` with every atom given a fresh, sequential
/// `atom_map` (1-based, in `mol.atoms()` iteration order) -- atoms that
/// already carry a map are overwritten. Confirmed empirically that
/// `MoleculeBuilder::add_atom` preserves insertion order as `AtomIdx`
/// (mirrors `clear_atom_maps`'s own rebuild pattern above, which relies on
/// the same property for its unmapped bond indices to stay valid), and that
/// `atom_map` survives the `canonical_smiles`/`split_fragments` round-trip
/// unchanged -- so pre-mapping the input here and calling an existing,
/// *unmodified* graph-based cleavage function through the normal
/// [`apply_retro`] dispatch is sufficient to recover real atom-level
/// correspondence between target and precursors, with zero changes needed
/// to any of the 8 graph-based rule functions themselves.
fn with_sequential_atom_maps(mol: &Molecule) -> Molecule {
    let mut builder = MoleculeBuilder::new();
    for (idx, atom) in mol.atoms() {
        let mut a = atom.clone();
        a.atom_map = Some(idx.0 as u16 + 1);
        builder.add_atom(a);
    }
    for (_, bond) in mol.bonds() {
        let _ = builder.add_bond(bond.atom1, bond.atom2, bond.order);
    }
    builder.copy_stereo_groups_from(mol);
    builder.copy_stereo_from(mol);
    builder.copy_bond_directions_from(mol);
    builder.build()
}

/// Derives a real, atom-mapped forward SMIRKS for one specific
/// `(rule_name, target, precursors)` outcome of a **graph-based** default
/// rule (a Rust function dispatched by name in [`apply_retro`], not a
/// SMIRKS string -- `ester_cleavage`, `amide_cleavage`, `aryl_ether_retro`,
/// `suzuki_retro`, `sulfonamide_retro`, `diaryl_sulfone_retro`,
/// `boc_deprotection_retro`, `cbz_deprotection_retro` as of this writing).
///
/// Exists because these rules have no `RetroRule::smirks` string for
/// `bridge::forward`'s `declared_smirks` to reverse-apply, so any route
/// step using one of them can never reach `forward_validation: pass` via
/// self-audit today -- confirmed as a real, ~30%-of-default-rules gap, see
/// `docs/design/retro-rule-precision-gaps-v0.md` #5. This closes that gap
/// for the `bridge::route_graph::normalize_renkin_route` audit path
/// specifically, called lazily at audit time (not stored on `ReactionStep`
/// or emitted in `find_routes`'s own JSON output -- keeps this a pure
/// audit-layer concern, no search-output schema change).
///
/// Mechanism: re-runs the *same, unmodified* graph-based cleavage function
/// `apply_retro` would have used, on a fresh clone of `target` with every
/// atom given a sequential map number first (see
/// [`with_sequential_atom_maps`]) -- the map numbers propagate through the
/// existing fragment-extraction code for free, since `MoleculeBuilder`
/// clones each `Atom` (including `atom_map`) verbatim. A rule can have
/// multiple valid disconnections for one target (e.g. two ester bonds), so
/// every re-derived outcome's precursor set is compared (as an unmapped,
/// canonical multiset) against the real, already-known `precursors` this
/// step actually used -- only a genuine match is trusted, never assumed to
/// be "probably the first one." Newly-introduced leaving-group atoms (the
/// appended -OH/-Br/-Cl/-B(OH)2 etc.) are never mapped, matching how a
/// hand-crafted SMIRKS rule like `heck_retro`
/// (`"[c:1][CH:2]=[CH:3]>>[c:1][Br].[CH2:2]=[CH:3]"`) leaves its own
/// literal `Br` unmapped too.
///
/// Returns `None` when `rule_name` isn't a known graph-based default rule
/// (including: it's a real SMIRKS-based rule, which needs no help from
/// this function at all), when `target`/`precursors` fail to parse, or
/// when re-running the rule genuinely can't reproduce a matching outcome
/// (should not happen for a route this engine itself already found, but
/// never assumed -- always re-derived and verified, never guessed).
pub fn declared_forward_smirks(
    rule_name: &str,
    target_smiles: &str,
    precursor_smiles: &[String],
) -> Option<String> {
    let is_graph_based = default_rules()
        .iter()
        .any(|r| r.name == rule_name && r.smirks.is_empty());
    if !is_graph_based {
        return None;
    }

    let target_mol = mol_from_smiles(target_smiles).ok()?;
    let mapped_target = with_sequential_atom_maps(&target_mol);
    let rule = rr(rule_name, "");
    let outcomes = apply_retro(&mapped_target, &rule);

    // Canonicalize the caller's precursor SMILES before comparing -- callers
    // (e.g. a hand-authored route, or one parsed back in from external JSON)
    // aren't guaranteed to already hand us the exact canonical form, and an
    // exact-string mismatch here must never be mistaken for "not this rule's
    // outcome".
    let given: std::collections::BTreeSet<String> = precursor_smiles
        .iter()
        .map(|s| mol_from_smiles(s).map(|m| to_canonical(&m)))
        .collect::<Result<_, _>>()
        .ok()?;
    for outcome in &outcomes {
        let outcome_unmapped: std::collections::BTreeSet<String> = outcome
            .iter()
            .map(|p| to_canonical(&clear_atom_maps(&p.mol)))
            .collect();
        if outcome_unmapped == given {
            let target_side = to_canonical(&mapped_target);
            let precursor_side = outcome
                .iter()
                .map(|p| to_canonical(&p.mol))
                .collect::<Vec<_>>()
                .join(".");
            return Some(format!("{target_side}>>{precursor_side}"));
        }
    }
    None
}

/// A standardized precursor molecule with its canonical SMILES.
pub struct PrecursorMol {
    pub smiles: String,
    pub mol: Molecule,
}

/// Split a (possibly disconnected) molecule into standardized PrecursorMol fragments.
/// Filters out chemically invalid fragments (aromatic atoms outside any ring) that
/// arise from chematic's SMIRKS BFS leaking substituents across product templates.
pub(crate) fn split_fragments(mol: &Molecule) -> Vec<PrecursorMol> {
    canonical_smiles(mol)
        .split('.')
        .filter_map(|frag| {
            let m = parse(frag).ok()?;
            let std_mol = standardize(&m, &STANDARDIZE_OPTS);
            // Reject fragments that have aromatic atoms but no ring closure —
            // these are open-chain aromatic chains produced by BFS leakage (L4).
            //
            // We detect rings by the presence of SMILES ring-closure digits rather
            // than aromatic_ring_count(), because chematic's aromatic_ring_count does
            // not count heteroaromatic rings (e.g. pyridine → 0), which incorrectly
            // filtered valid fragments like 4-bromopyridine in biaryl cleavage.
            let smi = canonical_smiles(&std_mol);
            let has_aromatic = smi
                .chars()
                .any(|c| matches!(c, 'c' | 'n' | 'o' | 's' | 'p'));
            let has_ring = smi.chars().any(|c| c.is_ascii_digit());
            if has_aromatic && !has_ring {
                return None;
            }
            Some(PrecursorMol {
                smiles: smi,
                mol: std_mol,
            })
        })
        .collect()
}

/// Why a just-constructed product molecule failed the aromaticity-integrity
/// check (see [`aromaticity_integrity_violation`]). Kept distinct so callers
/// (search stats, diagnostics) can report which invariant broke rather than
/// one opaque "invalid" bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AromaticityIntegrityViolation {
    /// An atom is flagged aromatic but doesn't lie on any ring (cycle) in
    /// the molecular graph -- e.g. an acyclic `n` produced when a hash-atom
    /// variant's independently-chosen per-side aromaticity spelling doesn't
    /// match the real, matched atom's actual ring membership (Issue #90).
    AromaticAtomNotInRing,
    /// An atom is flagged aromatic and does lie on a ring, but none of its
    /// incident bonds carry `BondOrder::Aromatic` -- the ring exists but
    /// wasn't (re-)perceived as aromatic around this atom, so treating it
    /// as aromatic is unsupported by the actual bond orders present.
    AromaticAtomWithoutAromaticBond,
}

impl AromaticityIntegrityViolation {
    /// Machine-readable reason code, stable across releases (used in
    /// diagnostics output, not just human-facing text).
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::AromaticAtomNotInRing => "aromatic_atom_not_in_ring",
            Self::AromaticAtomWithoutAromaticBond => "aromatic_atom_without_aromatic_bond",
        }
    }
}

/// The first aromaticity-integrity violation found in `mol`, if any.
///
/// Checked against the raw molecule as constructed by `run_reactants` --
/// before `split_fragments`'s text round-trip (`canonical_smiles` ->
/// `parse` -> `standardize`) or any external tool's own re-parse -- because
/// that round-trip (or an external parser's sanitizer) can silently repair
/// or reject the exact defect this function exists to catch, hiding it from
/// RENKIN itself even when it's still semantically wrong (see Issue #90: a
/// piperazine-ring nitrogen wrongly flagged aromatic parses fine and gets
/// silently corrected by at least one external SMILES sanitizer, but an
/// acyclic wrongly-aromatic nitrogen elsewhere in the same corpus doesn't
/// parse at all -- both must be caught here, at the source, uniformly).
///
/// Used identically by `apply_retro` (this module), `renkin-forward`'s
/// product enumeration, and `ring_context.rs`'s match-level gate -- all
/// three reach hash-atom-expanded SMIRKS via the same
/// [`application_smirks_variants`] helper and are equally exposed.
///
/// Ring membership reuses the existing [`is_bridge_bond`] BFS (already
/// relied on by the graph-based cleavage rules above and by
/// `ring_context.rs`) rather than a second bridge-finding implementation:
/// an atom lies on a ring iff at least one of its incident bonds is not a
/// bridge.
pub fn aromaticity_integrity_violation(mol: &Molecule) -> Option<AromaticityIntegrityViolation> {
    for (idx, atom) in mol.atoms() {
        if !atom.aromatic {
            continue;
        }
        let mut in_ring = false;
        let mut has_aromatic_bond = false;
        for (neighbor, bidx) in mol.neighbors(idx) {
            if !is_bridge_bond(mol, idx, neighbor) {
                in_ring = true;
            }
            if mol.bond(bidx).order == BondOrder::Aromatic {
                has_aromatic_bond = true;
            }
        }
        if !in_ring {
            return Some(AromaticityIntegrityViolation::AromaticAtomNotInRing);
        }
        if !has_aromatic_bond {
            return Some(AromaticityIntegrityViolation::AromaticAtomWithoutAromaticBond);
        }
    }
    None
}

/// Compute a bitmask of atomic numbers that MUST appear in the target molecule
/// for `smirks` to have any chance of matching. Reads the reactant side of the
/// SMIRKS and extracts explicit element symbols from bracket atoms and bare atoms.
/// Returns 0 if the SMIRKS is empty (graph-based rule) or cannot be parsed.
fn required_elements_from_smirks(smirks: &str) -> u64 {
    let reactant = match smirks.split(">>").next() {
        Some(r) if !r.is_empty() => r,
        _ => return 0,
    };
    // Map element symbol → atomic number for elements common in organic chemistry.
    // Only symbols that unambiguously appear as bare uppercase tokens in SMIRKS.
    const ELEMENTS: &[(&str, u64)] = &[
        ("Cl", 17),
        ("Br", 35),
        ("Si", 14),
        ("Se", 34),
        ("Te", 52),
        ("Sn", 50),
        ("Zn", 30),
        ("Pd", 46),
        ("Cu", 29),
        ("Fe", 26),
        ("B", 5),
        ("C", 6),
        ("N", 7),
        ("O", 8),
        ("F", 9),
        ("P", 15),
        ("S", 16),
        ("I", 53),
    ];
    let mut mask: u64 = 0;
    // Scan bracket atoms like [N:1], [c:2], [Cl], [NH2:3]
    let bytes = reactant.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            i += 1;
            // Skip stereo / charge prefix chars
            while i < bytes.len() && matches!(bytes[i], b'@' | b'+' | b'-' | b'#') {
                i += 1;
            }
            // Read element (1-2 uppercase letters, possibly followed by lowercase)
            for (sym, an) in ELEMENTS {
                let end = i + sym.len();
                if end <= bytes.len() && bytes[i..end].eq_ignore_ascii_case(sym.as_bytes()) {
                    mask |= 1u64 << an;
                    break;
                }
            }
        }
        i += 1;
    }
    mask
}

// ── Hash-atom ([#N]) wildcard: application-time compatibility compiler ──
//
// `chematic::rxn::run_reactants`/`parse_reaction` parse a SMIRKS through
// chematic-smiles's *SMILES* grammar (confirmed by reading
// chematic-smiles-0.10.0's `parser.rs` and chematic-rxn-0.10.0's
// `reaction.rs`/`transform.rs`), not the SMARTS grammar `parse_smarts`
// above uses. SMILES bracket atoms require a concrete element symbol, so a
// bare atomic-number primitive like `[#7:2]` ("any isotope of nitrogen,
// aromaticity unspecified" -- a real SMARTS wildcard, not a mistake in the
// extracted template) fails to parse there, even though `parse_smarts`
// happily accepts it at load time. This silently breaks every such
// template at *apply* time, in both directions (`apply_retro` and, via the
// same `RetroRule`s, `renkin-forward`'s prediction), while load-time
// validation reports success -- see Issue #88's investigation for the
// empirical confirmation. This is not fixed in chematic 0.11.0 either
// (the relevant parser/reaction code is byte-identical to 0.10.0); RENKIN
// implements this as an immediate application-time compatibility compiler
// rather than waiting on it. Whether `chematic-rxn` should grow a
// query-aware SMIRKS application path natively is a separate upstream
// capability question, tracked independently of this fix.
//
// **This is a compatibility layer, not a new logical template.** The
// checked-in extracted-template corpus stays at exactly one `RetroRule`
// per raw SMIRKS line -- `load_rules_from_file`'s return count, every
// `RetroRule::template_id`/`name`/`weight`, the ONNX template scorer's
// `n_rules` contract, and `index_rules_by_template_id` (candidate-pool
// export) are all unaffected. The expansion below only decides which
// concrete-element SMIRKS string(s) `apply_retro`/`renkin-forward` try
// *when actually running* a `[#N]`-bearing rule against a real molecule,
// and is computed lazily, cached by SMIRKS string (`apply_retro` is a
// search hot path -- see `apply_retro_call_count` -- so this must not
// re-run `chematic::rxn::parse_reaction` validation on every call).
//
// `#N` genuinely doesn't say whether the atom is aromatic, so this never
// guesses a single answer: every distinct `[#N]`/`[#N:map]` atom expands
// into every aromatic/aliphatic reading, and only readings that
// independently pass `chematic::rxn::parse_reaction` are kept. The LHS and
// RHS of `>>` are allowed to choose *independently* for the same
// atom-map -- an aromatization/dearomatization step is a real reaction
// class, not an error -- but a `[#N:m]` occurring more than once on the
// *same* side must agree with itself (it's the same query atom instance),
// and the same atom-map must resolve to the same *element* on both sides
// (only its aromaticity may differ). Anything this module can't safely
// reason about (a combined primitive, an inconsistent element, or a
// combinatorial space this large) is reported as a distinct, machine-
// readable [`HashAtomUnsupportedReason`] -- never silently left as the
// original, still-broken SMIRKS for `apply_retro` to fail on later.

/// Upper bound on how many variant SMIRKS one template may expand into
/// before it's reported `Unsupported(VariantLimitExceeded)` instead of
/// expanded. Independent per-side aromaticity roughly doubles the group
/// count for a `[#N]` atom-map used on both sides of `>>` versus a
/// same-choice-both-sides design, so this is set well above the checked-in
/// 500-template corpus's real maximum (verified empirically before
/// picking this constant -- see the PR body for the measured
/// distribution) with headroom for the larger `_5000.smi` file. A
/// template that genuinely exceeds this is left fully unsupported, not
/// partially expanded -- see `HashAtomUnsupportedReason::VariantLimitExceeded`.
const MAX_HASH_ATOM_VARIANTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HashAtomSide {
    Lhs,
    Rhs,
}

/// One `[#N]`/`[#N:map]` bracket-atom occurrence in a raw SMIRKS string.
/// `byte_range` covers the whole bracket (`[` through `]` inclusive) so it
/// can be sliced out and replaced. `side` is which side of the SMIRKS's
/// single `>>` this occurrence is on.
struct HashAtomOccurrence {
    byte_range: std::ops::Range<usize>,
    atomic_number: u8,
    atom_map: Option<u32>,
    side: HashAtomSide,
}

/// Scans `smirks` for bracket atoms whose entire content is a bare
/// `#<digits>` or `#<digits>:<digits>` primitive. Returns `None` (meaning
/// "don't attempt expansion for this template") if a `#` primitive is
/// found combined with anything else in the same bracket (e.g. `[#7;+0]`)
/// or the bracket/atom-map syntax is malformed -- those are more complex
/// SMARTS shapes this module doesn't attempt to rewrite safely, not
/// something to guess about. Returns `Some(vec![])` if no `#` atoms exist
/// at all.
fn find_hash_atoms(smirks: &str) -> Option<Vec<HashAtomOccurrence>> {
    let arrow_pos = smirks.find(">>");
    let mut occurrences = Vec::new();
    let mut i = 0;
    while i < smirks.len() {
        if smirks.as_bytes()[i] != b'[' {
            i += 1;
            continue;
        }
        let start = i;
        let end = smirks[i..].find(']').map(|rel| i + rel + 1)?;
        let inner = &smirks[start + 1..end - 1];
        if let Some(hash_pos) = inner.find('#') {
            if hash_pos != 0 {
                // `#` combined with other content ahead of it in the
                // bracket (shouldn't normally happen -- `#` is always the
                // first primitive when present) -- bail defensively.
                return None;
            }
            let rest = &inner[1..];
            let (num_str, map_str) = match rest.split_once(':') {
                Some((n, m)) => (n, Some(m)),
                None => (rest, None),
            };
            if num_str.is_empty() || !num_str.bytes().all(|b| b.is_ascii_digit()) {
                // `#` not followed by a bare digit run (e.g. a combined
                // primitive like `#7;+0`) -- don't attempt to rewrite.
                return None;
            }
            let atomic_number: u32 = num_str.parse().ok()?;
            if atomic_number == 0 || atomic_number > 118 {
                return None;
            }
            let atom_map = match map_str {
                None => None,
                Some(m) if !m.is_empty() && m.bytes().all(|b| b.is_ascii_digit()) => {
                    Some(m.parse().ok()?)
                }
                Some(_) => return None, // malformed atom-map token
            };
            let side = match arrow_pos {
                Some(pos) if start < pos => HashAtomSide::Lhs,
                _ => HashAtomSide::Rhs,
            };
            occurrences.push(HashAtomOccurrence {
                byte_range: start..end,
                atomic_number: atomic_number as u8,
                atom_map,
                side,
            });
        }
        i = end;
    }
    Some(occurrences)
}

/// Candidate element-symbol spellings for a `[#N]` atom: the uppercase
/// (aliphatic) and lowercase (aromatic) readings. Both are proposed;
/// callers must still validate each candidate SMIRKS via
/// `chematic::rxn::parse_reaction` before trusting it -- this function
/// only enumerates possibilities, it doesn't decide which are valid
/// (e.g. some elements' lowercase form may not be accepted by chematic's
/// parser at all, in which case that candidate is dropped downstream).
fn hash_atom_candidate_symbols(atomic_number: u8) -> Vec<String> {
    match Element::from_atomic_number(atomic_number) {
        Some(elem) => {
            let upper = elem.symbol().to_string();
            let lower = upper.to_lowercase();
            if upper == lower {
                vec![upper]
            } else {
                vec![upper, lower]
            }
        }
        None => vec![],
    }
}

/// Why a `[#N]`-bearing template could not be expanded into any usable
/// concrete-element variant. Distinct reasons are kept distinct so a
/// caller (corpus stats, benchmark reports) can tell them apart rather
/// than lumping every failure into one opaque "unsupported" bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAtomUnsupportedReason {
    /// `find_hash_atoms` found a `#` primitive combined with other
    /// content in the same bracket, or malformed atom-map syntax --
    /// outside what this module attempts to rewrite safely.
    UnhandledSyntax,
    /// The same atom-map number resolves to two different *elements*
    /// (not just a different aromaticity reading) somewhere in the
    /// template -- internally inconsistent, not a real SMIRKS.
    InconsistentElement,
    /// The full independent-per-side combinatorial space exceeds
    /// `MAX_HASH_ATOM_VARIANTS`. The template is left fully unsupported
    /// rather than expanded to an arbitrary partial subset.
    VariantLimitExceeded { total_combinations: usize },
    /// Every combination in the (in-bounds) combinatorial space was tried
    /// and none independently re-parsed via `chematic::rxn::parse_reaction`.
    NoValidVariant,
}

/// Result of attempting to expand a raw SMIRKS's `[#N]` atoms into
/// concrete-element variants. See the module docs above.
#[derive(Debug, Clone)]
enum HashAtomExpansion {
    /// No `[#N]` atoms found -- caller should use the SMIRKS unchanged.
    NotApplicable,
    /// See [`HashAtomUnsupportedReason`].
    Unsupported(HashAtomUnsupportedReason),
    /// One or more variant SMIRKS strings, each independently confirmed
    /// parseable by `chematic::rxn::parse_reaction`. Always the *complete*
    /// set for this template -- never a truncated subset (see
    /// `HashAtomUnsupportedReason::VariantLimitExceeded` for the case
    /// where the full space was too large to attempt at all).
    Expanded { variants: Vec<String> },
}

/// The role of one atom-map number within a SMIRKS's `>>` transform,
/// relative to its own local bonded environment -- used to decide whether
/// a hash-atom map's aromaticity reading may be chosen independently per
/// side (see `expand_hash_atom_variants`'s grouping) or must be bound
/// together. See Issue #90: a spectator atom-map given independent
/// per-side aromaticity produced product SMILES with an aromatic-flagged
/// atom that has no ring/aromatic-bond backing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MappedAtomRole {
    /// The atom-map's local bonded environment (neighbor atom-maps and
    /// bond conditions) is structurally identical on both sides of `>>` --
    /// a genuine spectator, unchanged by the reaction. Its aromaticity
    /// reading must be the same choice on both sides.
    Spectator,
    /// The atom-map's local bonded environment differs between LHS and
    /// RHS (a bond was added/removed/changed condition) -- a real reaction
    /// center, where LHS and RHS aromaticity may legitimately differ
    /// (aromatization/dearomatization is a real reaction class).
    ReactionCenter,
    /// Couldn't confidently compare (one side failed to parse as SMARTS,
    /// or the atom-map appears more than once on one side). Fails safe to
    /// the same treatment as `Spectator` -- coverage is never guessed at
    /// the cost of correctness.
    Unknown,
}

/// Split `s` on top-level `.` (bracket-depth 0 only) -- a SMIRKS/SMARTS
/// component separator never appears inside `[...]` brackets, so tracking
/// bracket depth is sufficient (no need for a full grammar parse here).
fn split_top_level_dots(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'[' => depth += 1,
            b']' => depth -= 1,
            b'.' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parse one side of a SMIRKS (LHS or RHS text, `.`-separated components)
/// into one `QueryMolecule` per component via `chematic::smarts::parse_smarts`
/// -- the SMARTS grammar accepts `[#N]` natively, unlike the SMILES grammar
/// `chematic::rxn::parse_reaction` requires (see Issue #88's root cause).
/// `None` if any component fails to parse -- classification becomes
/// unavailable for the whole template rather than silently partial.
fn parse_side_as_query_fragments(side_text: &str) -> Option<Vec<chematic::smarts::QueryMolecule>> {
    split_top_level_dots(side_text)
        .into_iter()
        .map(|frag| parse_smarts(frag).ok())
        .collect()
}

/// This atom-map's (neighbor atom-map, bond condition) multiset within one
/// parsed side. `None` if the map doesn't appear exactly once across the
/// side's fragments (absent, or ambiguously repeated -- don't guess).
fn mapped_atom_signature(
    fragments: &[chematic::smarts::QueryMolecule],
    map: u16,
) -> Option<Vec<(Option<u16>, chematic::smarts::BondQuery)>> {
    let mut found = None;
    for qmol in fragments {
        for (atom_idx, qatom) in qmol.atoms.iter().enumerate() {
            if qatom.atom_map != Some(map) {
                continue;
            }
            if found.is_some() {
                return None; // repeated on this side -- don't guess
            }
            let sig = qmol.adj[atom_idx]
                .iter()
                .map(|&(bond_idx, neighbor_idx)| {
                    (
                        qmol.atoms[neighbor_idx].atom_map,
                        qmol.bonds[bond_idx].query.clone(),
                    )
                })
                .collect();
            found = Some(sig);
        }
    }
    found
}

/// Multiset equality (order-independent, respects duplicates) via O(n^2)
/// removal-matching -- fine at the tiny per-atom degree these signatures
/// have (organic valence caps this at single digits).
fn bond_signature_multiset_eq(
    a: &[(Option<u16>, chematic::smarts::BondQuery)],
    b: &[(Option<u16>, chematic::smarts::BondQuery)],
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut remaining: Vec<&(Option<u16>, chematic::smarts::BondQuery)> = b.iter().collect();
    for item in a {
        let Some(pos) = remaining
            .iter()
            .position(|r| r.0 == item.0 && r.1 == item.1)
        else {
            return false;
        };
        remaining.remove(pos);
    }
    true
}

/// Classify every atom-map appearing on BOTH sides of `smirks` as
/// `Spectator`, `ReactionCenter`, or `Unknown`. Atom-maps appearing on only
/// one side aren't classified here -- they already get exactly one
/// independent group under the existing `(side, atom_map)` grouping,
/// unaffected by this classification either way.
fn classify_mapped_atom_roles(smirks: &str) -> FxHashMap<u32, MappedAtomRole> {
    let mut roles = FxHashMap::default();
    let Some((lhs_text, rhs_text)) = smirks.split_once(">>") else {
        return roles;
    };
    let Some(lhs_frags) = parse_side_as_query_fragments(lhs_text) else {
        return roles;
    };
    let Some(rhs_frags) = parse_side_as_query_fragments(rhs_text) else {
        return roles;
    };
    let lhs_maps: FxHashSet<u16> = lhs_frags
        .iter()
        .flat_map(|q| q.atoms.iter().filter_map(|a| a.atom_map))
        .collect();
    let rhs_maps: FxHashSet<u16> = rhs_frags
        .iter()
        .flat_map(|q| q.atoms.iter().filter_map(|a| a.atom_map))
        .collect();

    for &map in lhs_maps.intersection(&rhs_maps) {
        let role = match (
            mapped_atom_signature(&lhs_frags, map),
            mapped_atom_signature(&rhs_frags, map),
        ) {
            (Some(l), Some(r)) => {
                if bond_signature_multiset_eq(&l, &r) {
                    MappedAtomRole::Spectator
                } else {
                    MappedAtomRole::ReactionCenter
                }
            }
            _ => MappedAtomRole::Unknown,
        };
        roles.insert(map as u32, role);
    }
    roles
}

fn expand_hash_atom_variants(smirks: &str) -> HashAtomExpansion {
    let occurrences = match find_hash_atoms(smirks) {
        Some(o) => o,
        None => return HashAtomExpansion::Unsupported(HashAtomUnsupportedReason::UnhandledSyntax),
    };
    if occurrences.is_empty() {
        return HashAtomExpansion::NotApplicable;
    }

    // Element-identity check, independent of grouping: the same atom-map
    // number must resolve to the same atomic number everywhere it
    // appears, on either side. Aromaticity may still differ per side --
    // that's handled by grouping below, not here.
    let mut element_by_map: Vec<(u32, u8)> = Vec::new();
    for occ in &occurrences {
        let Some(m) = occ.atom_map else { continue };
        match element_by_map.iter().find(|(map, _)| *map == m) {
            Some((_, an)) if *an != occ.atomic_number => {
                return HashAtomExpansion::Unsupported(
                    HashAtomUnsupportedReason::InconsistentElement,
                );
            }
            Some(_) => {}
            None => element_by_map.push((m, occ.atomic_number)),
        }
    }

    // Group by (side, atom-map) -- EXCEPT for atom-maps confirmed (or
    // fail-safe-assumed) `Spectator`/`Unknown` (see `MappedAtomRole`),
    // which bind their LHS and RHS occurrences into ONE group so both
    // sides always pick the same aromaticity reading together. Only a
    // `ReactionCenter` map -- whose local bonded environment genuinely
    // differs between LHS and RHS -- gets the old per-side independent
    // choice (aromatization/dearomatization is a real reaction class).
    // Unmapped atoms are always their own group, regardless of side. See
    // Issue #90: an unconditional per-side choice let a spectator map's
    // RHS spelling flip aromaticity while its LHS still matched the real,
    // unchanged atom -- producing an aromatic-flagged atom with no ring or
    // aromatic bond backing it.
    let mapped_atom_roles = classify_mapped_atom_roles(smirks);
    let is_reaction_center =
        |m: u32| mapped_atom_roles.get(&m) == Some(&MappedAtomRole::ReactionCenter);
    let mut group_key: Vec<(HashAtomSide, Option<u32>, usize)> = Vec::new(); // (side, map, disambiguator)
    let mut group_members: Vec<Vec<usize>> = Vec::new();
    let mut group_atomic_number: Vec<u8> = Vec::new();
    let mut next_disambiguator = 0usize;
    for (idx, occ) in occurrences.iter().enumerate() {
        // Non-reaction-center mapped atoms are keyed under a canonical
        // `Lhs` side so a RHS occurrence of the same map joins the SAME
        // group as its LHS counterpart, instead of getting its own.
        let key_side = match occ.atom_map {
            Some(m) if !is_reaction_center(m) => HashAtomSide::Lhs,
            _ => occ.side,
        };
        let existing = occ.atom_map.and_then(|m| {
            group_key
                .iter()
                .position(|(s, gm, _)| *s == key_side && *gm == Some(m))
        });
        match existing {
            Some(gi) => group_members[gi].push(idx),
            None => {
                let key = match occ.atom_map {
                    Some(m) => (key_side, Some(m), 0),
                    None => {
                        next_disambiguator += 1;
                        (occ.side, None, next_disambiguator)
                    }
                };
                group_key.push(key);
                group_members.push(vec![idx]);
                group_atomic_number.push(occ.atomic_number);
            }
        }
    }

    let mut group_candidates: Vec<Vec<String>> = Vec::with_capacity(group_key.len());
    for &an in &group_atomic_number {
        let candidates = hash_atom_candidate_symbols(an);
        if candidates.is_empty() {
            return HashAtomExpansion::Unsupported(HashAtomUnsupportedReason::UnhandledSyntax);
        }
        group_candidates.push(candidates);
    }

    let total_combinations: usize = group_candidates.iter().map(Vec::len).product();
    if total_combinations > MAX_HASH_ATOM_VARIANTS {
        // Fail closed: an arbitrary partial subset would silently discard
        // some of the template's real semantics. Report the template as
        // fully unsupported instead.
        return HashAtomExpansion::Unsupported(HashAtomUnsupportedReason::VariantLimitExceeded {
            total_combinations,
        });
    }

    // Deterministic mixed-radix enumeration ("odometer"): combo_indices[g]
    // selects group g's candidate symbol. Same order every run.
    let mut combo_indices = vec![0usize; group_candidates.len()];
    let mut variants = Vec::new();
    for _ in 0..total_combinations {
        let mut replacements: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for (gi, members) in group_members.iter().enumerate() {
            let symbol = &group_candidates[gi][combo_indices[gi]];
            for &occ_idx in members {
                let occ = &occurrences[occ_idx];
                let replacement = match occ.atom_map {
                    Some(m) => format!("[{symbol}:{m}]"),
                    None => format!("[{symbol}]"),
                };
                replacements.push((occ.byte_range.clone(), replacement));
            }
        }
        // Apply replacements back-to-front so earlier byte offsets stay valid.
        replacements.sort_by_key(|r| std::cmp::Reverse(r.0.start));
        let mut candidate = smirks.to_string();
        for (range, replacement) in replacements {
            candidate.replace_range(range, &replacement);
        }
        if chematic::rxn::parse_reaction(&candidate).is_ok() {
            variants.push(candidate);
        }

        // Advance the odometer.
        let mut gi = 0;
        while gi < combo_indices.len() {
            combo_indices[gi] += 1;
            if combo_indices[gi] < group_candidates[gi].len() {
                break;
            }
            combo_indices[gi] = 0;
            gi += 1;
        }
    }

    if variants.is_empty() {
        return HashAtomExpansion::Unsupported(HashAtomUnsupportedReason::NoValidVariant);
    }
    HashAtomExpansion::Expanded { variants }
}

/// Machine-readable summary of whether/how a `RetroRule`'s SMIRKS can be
/// concretely applied via `chematic::rxn::run_reactants` -- for corpus
/// audits and benchmark diagnostics, independent of actually running the
/// rule against any molecule. Does not use the (cached) application path
/// `apply_retro` uses internally; safe to call for reporting at any scale
/// without warming or contending that cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcreteApplicationStatus {
    /// No `[#N]` atoms -- `apply_retro` uses `rule.smirks` directly, byte
    /// for byte, exactly as it always has.
    Direct,
    /// `[#N]` atoms present; `apply_retro` tries every one of
    /// `variant_count` independently-validated concrete-element readings.
    HashAtomVariants { variant_count: usize },
    /// `[#N]` atoms present but no usable concrete reading exists --
    /// `apply_retro` always returns zero precursors for this rule.
    Unsupported { reason: HashAtomUnsupportedReason },
}

/// See [`ConcreteApplicationStatus`].
pub fn concrete_application_status(smirks: &str) -> ConcreteApplicationStatus {
    match expand_hash_atom_variants(smirks) {
        HashAtomExpansion::NotApplicable => ConcreteApplicationStatus::Direct,
        HashAtomExpansion::Unsupported(reason) => ConcreteApplicationStatus::Unsupported { reason },
        HashAtomExpansion::Expanded { variants } => ConcreteApplicationStatus::HashAtomVariants {
            variant_count: variants.len(),
        },
    }
}

/// Cache for `application_smirks_variants`, keyed by the exact raw SMIRKS
/// string. Only ever populated for `[#N]`-bearing SMIRKS (the caller
/// fast-paths everything else) -- at most a few hundred entries even for
/// the largest checked-in template file, computed once each.
fn hash_atom_variant_cache()
-> &'static std::sync::Mutex<FxHashMap<String, std::sync::Arc<Vec<String>>>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<FxHashMap<String, std::sync::Arc<Vec<String>>>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(FxHashMap::default()))
}

/// The concrete-element SMIRKS variant(s) to actually attempt when
/// applying `smirks`. Always safe to call, for any SMIRKS -- returns
/// `vec![smirks.to_string()]` unchanged if there's no real `[#N]` atom to
/// expand (including the case where `smirks` merely *contains* a `#`
/// character as an ordinary SMILES triple-bond symbol, e.g. `C#N` or
/// `C#C`, which is unrelated to the `[#N]` bracket-atom primitive this
/// module handles -- confirmed as a real, previously-shipped bug in an
/// earlier draft of this fix: a naive caller-side `smirks.contains('#')`
/// pre-check misrouted every triple-bond-bearing template, of which the
/// checked-in corpus has several, into treating them as fully
/// unsupported). Empty iff the template genuinely has `[#N]` atoms that
/// couldn't be expanded (see [`concrete_application_status`] for why).
///
/// Computed once per distinct SMIRKS string and cached: this is called
/// from `apply_retro`'s hot path (see `apply_retro_call_count`) for every
/// `smirks` containing a `#` character (a deliberately cheap, sound-in-one-
/// direction pre-filter kept purely as a performance optimization -- when
/// it correctly finds no `#` at all, `apply_retro` skips this function
/// entirely; when a `#` is present, for any reason, this function is the
/// single source of truth and is always safe to call).
pub fn application_smirks_variants(smirks: &str) -> std::sync::Arc<Vec<String>> {
    let cache = hash_atom_variant_cache();
    if let Some(hit) = cache.lock().unwrap().get(smirks) {
        return std::sync::Arc::clone(hit);
    }
    let computed = std::sync::Arc::new(match expand_hash_atom_variants(smirks) {
        HashAtomExpansion::Expanded { variants } => variants,
        HashAtomExpansion::NotApplicable => vec![smirks.to_string()],
        HashAtomExpansion::Unsupported(_) => Vec::new(),
    });
    cache
        .lock()
        .unwrap()
        .insert(smirks.to_string(), std::sync::Arc::clone(&computed));
    computed
}

fn rr(name: &str, smirks: &str) -> RetroRule {
    let required_elements = required_elements_from_smirks(smirks);
    RetroRule {
        name: name.into(),
        template_id: format!("rule:{name}"),
        smirks: smirks.into(),
        required_elements,
        ..Default::default()
    }
}

pub fn default_rules() -> Vec<RetroRule> {
    vec![
        // ── Acyl disconnections ──────────────────────────────────────────
        // Ester C(=O)-O → carboxylic acid + alcohol/phenol
        rr("ester_cleavage", ""), // graph-based: dispatched in apply_retro (avoids BFS-leakage)
        // Graph-based: dispatched in apply_retro (SMIRKS-based had BFS-leakage)
        rr("amide_cleavage", ""),
        // Ar-C(=O)R → Ar-H + R-C(=O)Cl (Friedel-Crafts retro)
        rr(
            "friedel_crafts_acylation_retro",
            "[c:1][C:2](=[O:3])>>[c:1].[C:2](=[O:3])Cl",
        ),
        // ── Aryl C-heteroatom disconnections ────────────────────────────
        // Ar-COOH → Ar-H + HCOOH (retro-Kolbe-Schmitt / decarboxylation).
        // The trailing atom is [OH], not bare O: a bare O also matches an ester
        // oxygen (Ar-C(=O)-O-R), and the R substituent isn't captured by this
        // 3-atom pattern — run_reactants then drops R entirely when building
        // the "free HCOOH" precursor fragment, silently losing atoms (e.g.
        // methyl benzoate COC(=O)c1ccccc1 → [benzene, formic acid], losing
        // the OMe). Requiring a terminal hydroxyl restricts the match to
        // genuine free carboxylic acids, where ester_cleavage doesn't apply.
        rr(
            "aryl_carboxylation_retro",
            "[c:1][C:2](=O)[OH]>>[c:1].[C:2](=O)O",
        ),
        // `aryl_amine_retro` ("[c:1][N:2]>>[c:1].[N:2]") was removed: on a
        // ring-fused nitrogen (N shared between the aromatic ring and a
        // fused saturated ring, e.g. an indoline-like bicyclic target), it
        // deletes the nitrogen outright instead of returning it as part of
        // a second precursor fragment — a genuine atom-loss defect, not an
        // intentional reagent omission (see `reagent_omission_template_allowlist`
        // in `synthesizability/schema.rs`, which already excluded this rule).
        // Confirmed on `uspto50k_test#L2263` (issue #77). Root cause not yet
        // isolated (SMIRKS-pattern vs. `split_fragments`/BFS-leakage pipeline
        // artifact); disabled per project policy pending that investigation,
        // the same policy applied to the halide rules above (31.11). See
        // `aryl_amine_retro_removed_from_default_rules` below.
        //
        // `buchwald_hartwig_retro` ("[c:1][N:2]>>[c:1]Br.[N:2]") was removed
        // for the same reason and mechanism as `aryl_amine_retro` above --
        // same root cause (both product templates' bare single-atom RHS
        // fragments let substituent-carry-through BFS sweep unchecked
        // across a ring-fusion boundary), confirmed on the same
        // `c12c(NCCC2)ccc(c1)Br`-shaped repro. Worse in practice: not only
        // does the amine fragment vanish the same way, the *surviving*
        // aryl fragment comes back corrupted too (carry-through loops the
        // other way around the same ring through the same fusion carbon,
        // producing a spurious extra `Br` plus a dangling alkyl chain that
        // don't belong on the real precursor) -- a chemically-wrong
        // "solved" route, not just a missing one. See
        // `buchwald_hartwig_retro_removed_from_default_rules` below.
        // Ar-O → Ar-OH + leaving fragment (retro-Ullmann ether synthesis).
        // Graph-based (empty smirks, dispatched in apply_retro below), not a
        // SMIRKS string: chematic-rxn's own reactant-template parser is
        // plain-SMILES-only (no SMARTS `;`/`!`/`$(...)` operators -- confirmed
        // empirically, see docs/design/retro-rule-precision-gaps-v0.md #1),
        // so excluding an ester oxygen (Ar-O-C(=O)-R, which the old bare-O
        // SMIRKS also matched, mislabeling an ester cleavage as an Ullmann
        // ether disconnection) needs a graph-based carbonyl-neighbor check,
        // mirroring ester_cleavage_graph's own pattern below.
        rr("aryl_ether_retro", ""),
        // ── Aryl C-halide disconnections ────────────────────────────────
        // `aryl_chloride_retro` ("[c:1][Cl]>>[c:1]"), `aryl_iodide_retro`
        // ("[c:1][I]>>[c:1]"), and `aryl_fluoride_snAr_retro`
        // ("[c:1][F]>>[c:1]") were removed (31.11): each deleted a halogen
        // with no tracked precursor for where it went. Real synthesis of
        // Ar-X from Ar-H needs an explicit halogenating reagent (Cl2/FeCl3,
        // NCS, I2/oxidant, Selectfluor, ...); hydrodehalogenation back to
        // Ar-H needs an explicit reducing reagent (H2/Pd, Bu3SnH, ...).
        // Neither is modeled, so retro-applying these rules silently
        // dropped atoms (target MW > precursor MW) and produced
        // chemically-invalid "solved" routes — confirmed 100% (F, I) and
        // 73%+ (Cl; the remainder was a validator false-positive, tracked
        // separately as 31.12) Invalid+imbalanced on sampled USPTO-50k
        // targets. `aryl_fluoride_snAr_retro`'s name additionally claimed
        // SNAr chemistry it didn't represent (a real SNAr retro keeps a
        // leaving group on the ring; this one deleted straight to Ar-H).
        // No atom-balanced disconnection for Ar-X <-> Ar-H exists without
        // inventing an untracked reagent, so per project policy (default to
        // remove/deprecate absent a real atom-balanced alternative) these
        // were deleted outright rather than tightened. See
        // `aryl_chloride_retro_removed_from_default_rules` below.
        //
        // Ar-Cl → Ar-Br (halogen exchange retro; Ar-Br is often a cheaper BB)
        // — atom-preserving halogen swap, NOT the same bug, kept unchanged.
        rr("aryl_chloride_to_bromide", "[c:1][Cl]>>[c:1][Br]"),
        // ── Aryl C-C disconnections ──────────────────────────────────────
        // Graph-based: find Ar-Ar bridge bonds and split into Ar-Br + Ar.
        rr("suzuki_retro", ""),
        // `heck_retro` ("[c:1][CH:2]=[CH:3]>>[c:1][Br].[CH2:2]=[CH:3]") was
        // removed (found 2026-08-29 during reaction-family-mislabel-
        // regression-v0.md's hand-inspection of §3's candidate list, not
        // the rule-safety census's own static screen -- this rule's 2-atom
        // RHS fragment doesn't match the census's "bare single-atom RHS"
        // shape, so it was never flagged by that tool): on an internal
        // alkene that's endocyclic and fused to the same aromatic ring the
        // leaving-group Br attaches to (e.g. indene -- the cyclopentene
        // ring shares its fusion bond with the benzo ring), the declared
        // 2-fragment product ([c:1][Br] + [CH2:2]=[CH:3]) collapses into a
        // single connected product, since [CH:3]'s real "far side"
        // (continuing around the fused ring) reconnects to the very
        // aromatic ring the other fragment already claims -- topologically
        // impossible to cleanly separate the way the acyclic SMIRKS
        // assumes. Confirmed by direct `apply_retro` reproduction on
        // indene (`C1=Cc2ccccc2C1`, 9 heavy atoms): the single outcome
        // produced is one 8-atom bromotoluene-shaped fragment (the mapped
        // [CH:2] atom survives only as an isolated methyl substituent,
        // its double-bond partner [CH:3] and the ring-closing carbon
        // beyond it vanish entirely) -- a chemically-wrong "solved" route,
        // same broader ring-fusion/naive-fragment-splitting defect family
        // as `aryl_amine_retro`/`buchwald_hartwig_retro`/
        // `n_benzylation_retro`/`michael_retro`/`negishi_retro` above, but
        // its own distinct signature (connectivity collapse + atom loss,
        // not the bare-fragment atom-duplication those five share) since
        // this SMIRKS's RHS fragment is 2 atoms, not 1. See
        // `heck_retro_removed_from_default_rules` below.
        //
        // `heck_retro_terminal` ("[c:1][CH:2]=[CH2:3]>>[c:1][Br].
        // [CH2:2]=[CH2:3]") does NOT share this defect and is kept: its
        // terminal [CH2:3] endpoint is fully saturated (2 H, no room for a
        // ring-closure bond) and its [CH:2] atom's valence (aromatic bond +
        // double bond + 1 H = 4) leaves no room for a second ring bond
        // either -- structurally, this SMIRKS's own matched atoms can
        // never be embedded in a ring at all, proven by valence counting,
        // not just untested.
        // Ar-CH=CH2 → Ar-Br + CH2=CH2 (retro-Heck, terminal alkene / styrene)
        rr(
            "heck_retro_terminal",
            "[c:1][CH:2]=[CH2:3]>>[c:1][Br].[CH2:2]=[CH2:3]",
        ),
        // `negishi_retro` ("[c:1][CH2:2]>>[c:1][Br].[CH3:2]") was removed:
        // v0.36.0's rule-safety census flagged it as the top candidate
        // (structurally near-identical LHS/RHS shape to the already-removed
        // `buchwald_hartwig_retro`), and direct `apply_retro` reproduction
        // confirmed a real defect on a real ring-fused target (deliberately
        // constructed, not corpus-derived -- this rule had zero findings in
        // the 15-target smoke sample) -- on an Ar-CH2 bond that's part of a
        // saturated ring fused to the aromatic ring (e.g. an
        // indane/tetralin-type substructure), a 25-atom target produced a
        // single outcome summing to 49 heavy atoms across its precursors,
        // against a chemically correct 26 (target + one new Br). Same
        // defect class as `n_benzylation_retro`/`michael_retro` above
        // (under-constrained LHS + bare RHS fragment + a cut bond that's
        // part of a ring) and the same excess-over-correct atom-count
        // shape, but the exact BFS carry-through path was not traced, so
        // "same mechanism" is inferred from the shared structural
        // precondition and signature, not directly observed -- the
        // magnitude here (49 vs. 26, ~88% excess) is well beyond round 1's
        // (~22-25% excess), which may reflect a different-sized
        // carry-through region rather than an identical failure path. See
        // `negishi_retro_removed_from_default_rules` below.
        //
        // ── Aliphatic C-C disconnections ─────────────────────────────────
        // Generic aliphatic C-C bond cleavage
        rr("cc_single_cleavage", "[C:1][C:2]>>[C:1].[C:2]"),
        // Alkene → two carbonyls (retro-Wittig / retro-HWE)
        rr("wittig_retro", "[C:1]=[C:2]>>[C:1]=O.[C:2]=O"),
        // ── C-N disconnections ───────────────────────────────────────────
        // C-N → C=O + amine (retro-reductive amination; aliphatic C only)
        rr("reductive_amination_retro", "[C:1][N:2]>>[C:1]=O.[N:2]"),
        // Generic aliphatic C-N bond cleavage (N-alkylation retro)
        rr("cn_aliphatic_cleavage", "[C:1][N:2]>>[C:1].[N:2]"),
        // ── C-O disconnections ───────────────────────────────────────────
        // Generic aliphatic C-O bond cleavage (ether / O-alkylation retro)
        rr("co_aliphatic_cleavage", "[C:1][O:2]>>[C:1].[O:2]"),
        // Alcohol → ketone/aldehyde (retro-reduction; converts C-OH to C=O)
        rr("alcohol_oxidation_retro", "[C:1][OH:2]>>[C:1]=O"),
        // ── Sonogashira coupling ─────────────────────────────────────────────
        // Ar-C≡C-R → Ar-Br + HC≡C-R (retro-Sonogashira, Pd/Cu catalysis)
        rr("sonogashira_retro", "[c:1][C:2]#[C:3]>>[c:1]Br.[C:2]#[C:3]"),
        // ── Sulfonamide / diaryl sulfone disconnections ──────────────────────
        // Ar-SO2-NHR → Ar-SO2Cl + HNR. Graph-based (avoids BFS-leakage).
        rr("sulfonamide_retro", ""),
        // Ar-SO2-Ar' → Ar-SO2Cl + Ar'H (graph-based; Friedel-Crafts sulfonylation retro)
        rr("diaryl_sulfone_retro", ""),
        // ── N-protection / deprotection ──────────────────────────────────────
        // N-Boc → N-H (deprotect: TFA removes Boc). Graph-based to avoid leakage.
        rr("boc_deprotection_retro", ""),
        // ── N-alkylation (more specific than cn_aliphatic_cleavage) ──────────
        // `n_benzylation_retro` ("[N:1][CH2:2][c:3]>>[N:1].[Br][CH2:2][c:3]")
        // was removed: v0.36.0's rule-safety census (issue #77-class screen,
        // docs/validation/rule-safety-census-2026-08-24.md) flagged it for
        // the same bare-single-atom-RHS shape as `aryl_amine_retro`/
        // `buchwald_hartwig_retro`, and direct `apply_retro` reproduction on
        // a real target confirmed it: on an N-CH2-Ar bond that's part of a
        // ring, substituent-carry-through BFS for the "bare" [N:1] fragment
        // loops the other way around the ring, producing a fragment that's
        // nearly the entire rest of the molecule, while the declared
        // Br-CH2-Ar leaving-group fragment comes back holding a piece of
        // the ring backbone that was never really a substituent -- a
        // chemically-wrong "solved" route, not a missing one, same failure
        // shape as the two already-removed rules. See
        // `n_benzylation_retro_removed_from_default_rules` below.
        //
        // ── Grignard / organolithium retro ───────────────────────────────────
        // `grignard_addition_retro`
        // ("[C:1]([OH:2])([C:3])[C:4]>>[C:1](=O)[C:3].[C:4]") was removed:
        // flagged by the v0.36.0 census and already carrying real
        // `SpectatorBondLoss` findings, confirmed by direct `apply_retro`
        // reproduction on a real ring-fused tertiary alcohol -- when the
        // atom-matcher binds the "bare" [C:4] leaving-group slot to a
        // ring-continuation bond rather than a genuine exocyclic
        // substituent (both are structurally valid matches for this
        // under-constrained LHS), cutting it doesn't actually separate
        // the molecule, and the carry-through duplicates ring atoms into
        // both declared fragments: an 11-atom target produced a single
        // outcome summing to 18 heavy atoms, against a chemically correct
        // 11 (this SMIRKS's RHS is atom-conserving) -- same defect class
        // (under-constrained LHS + bare RHS fragment + ring-membership at
        // the cut site) as `negishi_retro`/`n_benzylation_retro`/
        // `michael_retro` above, and the same excess-over-correct
        // signature, but the two output fragments here each contain a
        // *whole* extra benzene ring the 11-atom target only has one of --
        // wholesale ring duplication, not the more modest ~22-25% excess
        // round 1's two rules showed. Whether that's the identical BFS
        // failure path or a related-but-distinct one wasn't traced; treat
        // "same mechanism" claims across these four as inferred from the
        // shared precondition and signature shape, not directly observed.
        // Not every match is wrong (a genuine exocyclic substituent, e.g.
        // a real methyl-Grignard product, still disconnects correctly --
        // see the still-passing acyclic case this rule's removed
        // `SubstituentPreservationCase` used to cover), but an
        // under-constrained LHS with no ring/degree check can't tell the
        // two apart, so the rule is disabled entirely rather than kept
        // for the subset of inputs it handles correctly, matching this
        // codebase's established policy for this whole defect class. See
        // `grignard_addition_retro_removed_from_default_rules` below.
        //
        // ── Claisen / Dieckmann condensation ────────────────────────────────
        // β-ketoester → ester + ester (retro-Claisen condensation)
        rr(
            "claisen_retro",
            "[C:1](=O)[CH2:2][C:3](=O)[O:4]>>[C:1](=O)O.[C:2]=[C:3][O:4]",
        ),
        // ── Michael addition retro ───────────────────────────────────────────
        // `michael_retro`
        // ("[C:1][CH2:2][C:3]=[O:4]>>[C:1].[CH2:2]=[C:3][OH:4]") was removed:
        // same v0.36.0 rule-safety census flag and same confirmed mechanism
        // as `n_benzylation_retro` above -- on a C-CH2-C=O bond that's part
        // of a ring (e.g. a glutarimide), the "bare" [C:1] fragment's
        // carry-through loops around the ring the other way, producing
        // nearly the whole molecule as one fragment while the declared
        // enol fragment comes back as an unreal, garbled piece. Confirmed
        // by direct `apply_retro` reproduction on a real target. See
        // `michael_retro_removed_from_default_rules` below.
        //
        // ── Acyl chloride as electrophile source ─────────────────────────────
        // Acid chloride → carboxylic acid (SOCl2 activation retro)
        rr("acyl_chloride_from_acid", "[C:1](=[O:2])Cl>>[C:1](=[O:2])O"),
        // ── N-formylation / N-acylation (Cbz retro) ─────────────────────────
        // N-Cbz → N-H (hydrogenolysis retro, graph-based)
        rr("cbz_deprotection_retro", ""),
    ]
}

/// Extract (elem1, elem2) bond-pair signatures from a SMIRKS reactant pattern.
///
/// Parses bracket atoms and the bond topology of the SMIRKS left-hand side to
/// determine which element-pair bonds the template can break.  Returns sorted,
/// deduplicated `(min_atomic_num, max_atomic_num)` pairs.
pub fn bond_pairs_from_smirks(smirks: &str) -> Vec<(u8, u8)> {
    let reactant = match smirks.split_once(">>") {
        Some((lhs, _)) => lhs,
        None => return vec![],
    };
    // Same element table used in required_elements_from_smirks.
    const ELEMENTS: &[(&str, u8)] = &[
        ("Cl", 17),
        ("Br", 35),
        ("Si", 14),
        ("Se", 34),
        ("Te", 52),
        ("Sn", 50),
        ("Zn", 30),
        ("Pd", 46),
        ("Cu", 29),
        ("Fe", 26),
        ("B", 5),
        ("C", 6),
        ("N", 7),
        ("O", 8),
        ("F", 9),
        ("P", 15),
        ("S", 16),
        ("I", 53),
    ];
    fn elem_at(bytes: &[u8], mut j: usize) -> Option<u8> {
        while j < bytes.len() && matches!(bytes[j], b'@' | b'+' | b'-' | b'#') {
            j += 1;
        }
        for (sym, an) in ELEMENTS {
            let end = j + sym.len();
            if end <= bytes.len() && bytes[j..end].eq_ignore_ascii_case(sym.as_bytes()) {
                return Some(*an);
            }
        }
        None
    }
    let bytes = reactant.as_bytes();
    let mut pairs: Vec<(u8, u8)> = Vec::new();
    let mut stack: Vec<Option<u8>> = Vec::new(); // branch context atom
    let mut prev: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => {
                if let Some(elem) = elem_at(bytes, i + 1) {
                    if let Some(p) = prev {
                        let pair = if p <= elem { (p, elem) } else { (elem, p) };
                        pairs.push(pair);
                    }
                    prev = Some(elem);
                }
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
            }
            b'(' => stack.push(prev),
            b')' => prev = stack.pop().flatten(),
            b'.' => {
                // Top-level component separator (never appears inside a
                // bracket atom -- those bytes are already skipped above).
                // Without this reset, the last atom of one disconnected
                // fragment and the first atom of the next would be
                // recorded as a bonded pair, which they are not --
                // `TemplateBondIndex`'s AND/subset retrieval treats every
                // returned pair as a hard requirement, so a spurious
                // cross-fragment pair here would wrongly exclude targets
                // that lack that nonexistent bond (a false negative).
                prev = None;
                stack.clear();
            }
            _ => {}
        }
        i += 1;
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

/// Bond-center template index (RetroKNN-inspired).
///
/// Indexes templates by the element-pair bonds their SMIRKS patterns can break.
/// At search time, only templates relevant to bonds present in the target molecule
/// are retrieved, avoiding unnecessary SMARTS matching for incompatible templates.
///
/// OR/union retrieval: a rule is retrieved if the target shares *any one*
/// element-pair bond with it. Phase B.1 (2026-08-12) tried an AND/subset
/// variant (require *every* pair present, not just one) to get more
/// exclusion power at 5,000+ templates -- it worked (94.8-99.3% ->
/// ~81% average retained) but not nearly enough to clear that program's
/// speed gate (1.14x -> 1.18x, needed >=1.5x), and wasn't independently
/// validated against this OR baseline at this feature's actual shipped
/// usage point (500-template default, route search via `--bond-index`).
/// Reverted rather than shipped as an unvalidated behavior change to an
/// already-production flag -- see `data/phase_b1_frontier/findings.md`
/// for the full negative-result writeup and cost attribution.
pub struct TemplateBondIndex {
    index: FxHashMap<(u8, u8), Vec<usize>>,
    /// Graph-based rules (empty SMIRKS) — always included.
    graph_indices: Vec<usize>,
    /// Rules with unparseable / empty bond pairs — included as fallback.
    fallback_indices: Vec<usize>,
}

/// Result of a bond-index lookup, including the candidate counts needed to
/// measure the element-presence filter's exclusion power.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateBondRetrieval {
    /// Unique indexed rules collected before required-element filtering.
    pub candidates_before_element_filter: usize,
    /// Unique indexed rules remaining after required-element filtering.
    pub candidates_after_element_filter: usize,
    /// Rule indices returned after optional top-k trimming.
    pub indices: Vec<usize>,
}

impl TemplateBondIndex {
    pub fn build(rules: &[RetroRule]) -> Self {
        let mut index: FxHashMap<(u8, u8), Vec<usize>> = FxHashMap::default();
        let mut graph_indices = Vec::new();
        let mut fallback_indices = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            if rule.smirks.is_empty() {
                graph_indices.push(i);
                continue;
            }
            let pairs = bond_pairs_from_smirks(&rule.smirks);
            if pairs.is_empty() {
                fallback_indices.push(i);
            } else {
                for pair in pairs {
                    index.entry(pair).or_default().push(i);
                }
            }
        }
        Self {
            index,
            graph_indices,
            fallback_indices,
        }
    }

    /// Return indices (into the original `rules` slice) of templates relevant to `mol`.
    /// Includes graph-based rules and fallback rules unconditionally.
    /// If `top_k > 0`, the SMIRKS-matched candidates are trimmed to the top-K by weight.
    pub fn retrieve(&self, mol: &Molecule, top_k: usize, rules: &[RetroRule]) -> Vec<usize> {
        self.retrieve_with_diagnostics(mol, top_k, rules).indices
    }

    /// Retrieve rules and expose the pre/post element-filter counts for
    /// search diagnostics. The returned rule indices and ordering are exactly
    /// the same as [`Self::retrieve`].
    pub fn retrieve_with_diagnostics(
        &self,
        mol: &Molecule,
        top_k: usize,
        rules: &[RetroRule],
    ) -> TemplateBondRetrieval {
        let target_elements = mol.atoms().fold(0u64, |mask, (atom_idx, _)| {
            let atomic_number = mol.atom(atom_idx).element.atomic_number();
            if atomic_number < 64 {
                mask | (1u64 << atomic_number)
            } else {
                mask
            }
        });
        let is_eligible = |idx: usize| {
            rules
                .get(idx)
                .is_some_and(|rule| rule.required_elements & !target_elements == 0)
        };
        let mut seen: FxHashSet<usize> = FxHashSet::default();
        let mut candidates: Vec<usize> = Vec::new();

        // Always include graph-based and fallback rules.
        for &idx in &self.graph_indices {
            if seen.insert(idx) {
                candidates.push(idx);
            }
        }
        for &idx in &self.fallback_indices {
            if seen.insert(idx) {
                candidates.push(idx);
            }
        }

        let fixed_before_filter = candidates.len();

        // Retrieve SMIRKS rules matching bonds present in the target.
        for (atom_idx, _) in mol.atoms() {
            let e1 = mol.atom(atom_idx).element.atomic_number();
            for (nb_idx, _bond_idx) in mol.neighbors(atom_idx) {
                // Only process each bond once (lower-index atom first).
                if nb_idx <= atom_idx {
                    continue;
                }
                let e2 = mol.atom(nb_idx).element.atomic_number();
                let pair = if e1 <= e2 { (e1, e2) } else { (e2, e1) };
                if let Some(indices) = self.index.get(&pair) {
                    for &idx in indices {
                        if seen.insert(idx) {
                            candidates.push(idx);
                        }
                    }
                }
            }
        }

        let candidates_before_element_filter = candidates.len();
        candidates.retain(|&idx| is_eligible(idx));
        let fixed = candidates
            .iter()
            .take(fixed_before_filter)
            .filter(|&&idx| is_eligible(idx))
            .count();
        let candidates_after_element_filter = candidates.len();

        if top_k > 0 && candidates.len() > top_k {
            // Sort SMIRKS portion by weight desc, keep top_k total.
            candidates[fixed..].sort_unstable_by(|&a, &b| {
                rules[b]
                    .weight
                    .partial_cmp(&rules[a].weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            candidates.truncate(fixed + top_k);
        }
        TemplateBondRetrieval {
            candidates_before_element_filter,
            candidates_after_element_filter,
            indices: candidates,
        }
    }
}

/// Map comma-separated element symbols (e.g. `"Br,I"`) to the same bitmask
/// format as `RetroRule::required_elements`.  Unknown symbols are silently skipped.
pub fn elem_symbols_to_mask(csv: &str) -> u64 {
    let mut mask = 0u64;
    for sym in csv.split(',') {
        let n: Option<u32> = match sym.trim() {
            "H" => Some(1),
            "B" => Some(5),
            "C" => Some(6),
            "N" => Some(7),
            "O" => Some(8),
            "F" => Some(9),
            "Si" => Some(14),
            "P" => Some(15),
            "S" => Some(16),
            "Cl" => Some(17),
            "Br" => Some(35),
            "I" => Some(53),
            _ => None,
        };
        if let Some(n) = n {
            mask |= 1u64 << n;
        }
    }
    mask
}

/// Keep only the `k` highest-weight (most frequent) templates.
/// Used by `--top-templates N` to trade a little recall for speed and less noise.
/// Hand-crafted rules are loaded separately and are never passed here.
pub fn top_templates_by_weight(mut rules: Vec<RetroRule>, k: usize) -> Vec<RetroRule> {
    if rules.len() <= k {
        return rules;
    }
    rules.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rules.truncate(k);
    rules
}

/// Load additional SMIRKS templates from a file (tab-separated: SMIRKS\tcount).
/// Lines starting with '#' are treated as comments and skipped.
/// Validates each template by running it against a probe molecule; only templates
/// that chematic's run_reactants can handle (even if they produce no matches) are kept.
pub fn load_rules_from_file(path: &str) -> Vec<RetroRule> {
    // Validate each template by parsing the reactant side with parse_smarts.
    // chematic 0.4.14 fixed issue #19: parse_smarts now accepts atom-map notation (:N),
    // so we can validate SMIRKS reactant patterns directly instead of running them
    // against a probe molecule.
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: could not read template file {path}: {e}");
            return vec![];
        }
    };
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .enumerate()
        .filter_map(|(i, line)| {
            // Format is exactly 2 tab-separated columns today: SMIRKS, count.
            // `splitn(2, '\t')`'s second half is everything after the first tab,
            // including any further tab-separated content -- so a naive 3rd column
            // (e.g. a template ID or DOI for provenance metadata) added later without
            // a format-version bump won't error here. It'll just make `count.parse()`
            // fail on the combined string and silently fall back to `weight = 1.0`
            // via `.unwrap_or(1.0)` below, corrupting the frequency weight for every
            // such line. Whoever adds a 3rd column needs to change this split first.
            let mut cols = line.splitn(2, '\t');
            let smirks = cols.next()?.trim();
            let count: f64 = cols
                .next()
                .and_then(|c| c.trim().parse().ok())
                .unwrap_or(1.0);
            let weight = (count + 1.0).ln();
            let reactant = smirks.split(">>").next()?;
            // Validate that chematic can parse the reactant SMARTS pattern.
            parse_smarts(reactant).ok()?;
            // Exactly one logical RetroRule per raw line -- unchanged from
            // this function's original contract. Templates containing a
            // `[#N]` bare-atomic-number SMARTS primitive keep their SMIRKS
            // exactly as written here; `apply_retro`/`renkin-forward`
            // transparently try independently-validated concrete-element
            // readings at *apply* time (see
            // `chem_env::{concrete_application_status,
            // application_smirks_variants}`, Issue #88) without changing
            // template identity, count, weight, or scorer/candidate-pool
            // indexing -- none of which this loader touches.
            let required_elements = required_elements_from_smirks(smirks);
            Some(RetroRule {
                name: format!("extracted_{i}"),
                template_id: template_id_for_smirks(smirks),
                smirks: smirks.to_string(),
                weight,
                required_elements,
            })
        })
        .collect()
}

/// Graph-based Boc deprotection retro:
/// N-C(=O)-O-C(C)(C)C → N-H  (removes Boc group, "protected amine" retro synthesis)
fn boc_deprotection(mol: &Molecule) -> Vec<Vec<PrecursorMol>> {
    // Find N–C(=O)–O–C(C)(C)C substructure via SMARTS and remove the Boc group.
    // This is modelled as: cut the N–C bond of the carbamate.
    let boc_smarts = "[N;!$(N=*)]C(=O)OC(C)(C)C";
    let Ok(query) = chematic::smarts::parse_smarts(boc_smarts) else {
        return vec![];
    };
    let matches = chematic::smarts::find_matches(&query, mol);
    if matches.is_empty() {
        return vec![];
    }

    let mut results = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    for m in matches {
        // m[0] = N, m[1] = carbonyl C
        if m.len() < 2 {
            continue;
        }
        let Some(&n_idx) = m.get(&0) else { continue };
        let Some(&c_idx) = m.get(&1) else { continue };

        if !is_bridge_bond(mol, n_idx, c_idx) {
            continue;
        }

        let comp_n = get_component(mol, n_idx, n_idx, c_idx);
        let Some(frag_n) = build_sub_molecule(mol, &comp_n) else {
            continue;
        };

        let precs = split_fragments(&frag_n);
        if precs.is_empty() {
            continue;
        }

        let key = precs
            .iter()
            .map(|p| p.smiles.as_str())
            .collect::<Vec<_>>()
            .join("|");
        if !seen.insert(key) {
            continue;
        }
        results.push(precs);
    }
    results
}

/// Graph-based Cbz deprotection retro:
/// N-C(=O)-O-CH2-Ph → N-H  (hydrogenolysis removes Cbz group)
fn cbz_deprotection(mol: &Molecule) -> Vec<Vec<PrecursorMol>> {
    let cbz_smarts = "[N;!$(N=*)]C(=O)OCc1ccccc1";
    let Ok(query) = chematic::smarts::parse_smarts(cbz_smarts) else {
        return vec![];
    };
    let matches = chematic::smarts::find_matches(&query, mol);
    if matches.is_empty() {
        return vec![];
    }

    let mut results = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    for m in matches {
        if m.len() < 2 {
            continue;
        }
        let Some(&n_idx) = m.get(&0) else { continue };
        let Some(&c_idx) = m.get(&1) else { continue };

        if !is_bridge_bond(mol, n_idx, c_idx) {
            continue;
        }

        let comp_n = get_component(mol, n_idx, n_idx, c_idx);
        let Some(frag_n) = build_sub_molecule(mol, &comp_n) else {
            continue;
        };

        let precs = split_fragments(&frag_n);
        if precs.is_empty() {
            continue;
        }

        let key = precs
            .iter()
            .map(|p| p.smiles.as_str())
            .collect::<Vec<_>>()
            .join("|");
        if !seen.insert(key) {
            continue;
        }
        results.push(precs);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_aspirin_bbs() -> ChemEnv {
        ChemEnv::in_memory(&["CC(=O)O", "Oc1ccccc1C(=O)O", "c1ccccc1C(=O)O", "C", "O"])
    }

    #[test]
    fn content_sha256_is_order_independent_and_detects_content_change() {
        let a = ChemEnv::in_memory(&["CCO", "CC(=O)O", "C"]);
        let b = ChemEnv::in_memory(&["C", "CC(=O)O", "CCO"]);
        assert_eq!(
            a.content_sha256(),
            b.content_sha256(),
            "hashing must not depend on input order"
        );

        let c = ChemEnv::in_memory(&["CCO", "CC(=O)O"]);
        assert_ne!(
            a.content_sha256(),
            c.content_sha256(),
            "a different BB set must hash differently even under the same caller-supplied label"
        );
    }

    #[test]
    fn bond_pairs_from_smirks_resets_at_component_boundary() {
        // Two disconnected LHS fragments (`.`-separated) must not produce
        // a spurious bonded pair between the last atom of one fragment
        // and the first atom of the next -- see the `.` handling in
        // `bond_pairs_from_smirks`. Before that fix, `prev` carried across
        // the `.` unchanged, so this SMIRKS would have wrongly recorded a
        // Cl-N pair spanning the two disconnected fragments.
        let pairs = bond_pairs_from_smirks("[C:1][Cl:2].[N:3][O:4]>>[C:1][N:3]");
        assert!(pairs.contains(&(6, 17)), "C-Cl pair missing: {pairs:?}");
        assert!(pairs.contains(&(7, 8)), "N-O pair missing: {pairs:?}");
        assert!(
            !pairs.contains(&(7, 17)),
            "spurious Cl-N pair across '.' boundary: {pairs:?}"
        );
    }

    #[test]
    fn bond_index_filters_templates_missing_required_target_elements() {
        let matching = "[C:1][Cl:2]>>[C:1].[Cl:2]";
        let missing_oxygen = "[C:1][Cl:2][O:3]>>[C:1].[Cl:2][O:3]";
        let rules = vec![
            RetroRule {
                smirks: matching.to_string(),
                required_elements: required_elements_from_smirks(matching),
                ..RetroRule::default()
            },
            RetroRule {
                smirks: missing_oxygen.to_string(),
                required_elements: required_elements_from_smirks(missing_oxygen),
                ..RetroRule::default()
            },
        ];
        let index = TemplateBondIndex::build(&rules);
        let target = parse("CCCl").unwrap();

        let retrieval = index.retrieve_with_diagnostics(&target, 0, &rules);
        assert_eq!(retrieval.candidates_before_element_filter, 2);
        assert_eq!(retrieval.candidates_after_element_filter, 1);
        assert_eq!(retrieval.indices, vec![0]);
        assert_eq!(index.retrieve(&target, 0, &rules), vec![0]);
    }

    #[test]
    fn bond_index_top_k_counts_only_eligible_fixed_rules() {
        let graph_rule = RetroRule {
            name: "graph".to_string(),
            required_elements: 1u64 << 8,
            ..RetroRule::default()
        };
        let bond_rule = RetroRule {
            smirks: "[C:1][Cl:2]>>[C:1].[Cl:2]".to_string(),
            required_elements: required_elements_from_smirks("[C:1][Cl:2]>>[C:1].[Cl:2]"),
            weight: 2.0,
            ..RetroRule::default()
        };
        let rules = vec![graph_rule, bond_rule];
        let index = TemplateBondIndex::build(&rules);
        let target = parse("CCCl").unwrap();

        assert_eq!(index.retrieve(&target, 1, &rules), vec![1]);
    }

    #[test]
    fn parse_aspirin_roundtrip() {
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        assert_eq!(mol.atom_count(), 13);
    }

    // ── Spectator N/P-oxide charge repair (Issue #106 / L4703) ──────────
    //
    // Root cause: `hash_atom_candidate_symbols` only offers "N"/"n" for
    // atomic number 7 -- expanding a spectator `[#7:m]` always produces a
    // literal, always-neutral output atom, even when the real matched
    // substrate atom is charged (e.g. pyridine N-oxide's `[n+]`).
    // `run_reactants` builds the output atom from that literal spelling,
    // not from the real atom it stood in for, and its output atoms never
    // carry `atom_map` (confirmed via a direct dump), so there is no way
    // to recover the real charge from `run_reactants`'s public API. The
    // repair instead restores the one narrow, ring-size-independent
    // invariant that was broken: an aromatic N/P bonded (non-aromatically)
    // to a negatively-charged exocyclic O/S must itself be positive.

    #[test]
    fn repair_spectator_oxide_charge_fixes_broken_n_oxide_fragment() {
        // The exact defective precursor fragment `run_reactants` produced
        // for uspto50k_test#L4703 before this fix: a pyridine-N-oxide ring
        // written with a neutral `n` despite still carrying its `[O-]`
        // substituent -- unkekulizable, rejected by RDKit
        // (`compare_route_graph.py`'s validator) as `unparseable_smiles_in_route`.
        let mut mol = mol_from_smiles("n1(cc(C(=O)OC)ccc1)[O-]").unwrap();
        repair_spectator_oxide_charge(&mut mol);
        let fixed = canonical_smiles(&mol);
        assert!(
            mol_from_smiles(&fixed).is_ok(),
            "repaired fragment must itself round-trip through chematic's own parser: {fixed}"
        );
        let n_charge = mol
            .atoms()
            .find(|(_, a)| a.element.atomic_number() == 7)
            .map(|(_, a)| a.charge);
        assert_eq!(n_charge, Some(1), "ring N must now carry +1: {fixed}");
    }

    #[test]
    fn repair_spectator_oxide_charge_is_idempotent_on_already_correct_n_oxide() {
        let mut mol = mol_from_smiles("COC(=O)c1ccc[n+]([O-])c1-c1ccc(F)cc1").unwrap();
        let before = canonical_smiles(&mol);
        repair_spectator_oxide_charge(&mut mol);
        assert_eq!(
            canonical_smiles(&mol),
            before,
            "an already-correctly-charged N-oxide must be left unchanged"
        );
    }

    #[test]
    fn repair_spectator_oxide_charge_ignores_n_methylpyrrole() {
        // N-alkyl substitution on a genuine lone-pair-donor ring nitrogen
        // (5-membered azole) must NOT be touched -- its exocyclic
        // substituent is carbon, not a negatively-charged O/S, so the
        // repair's neighbor check never fires.
        let mut mol = mol_from_smiles("Cn1cccc1").unwrap();
        let before = canonical_smiles(&mol);
        repair_spectator_oxide_charge(&mut mol);
        assert_eq!(
            canonical_smiles(&mol),
            before,
            "N-methylpyrrole must stay neutral"
        );
    }

    #[test]
    fn repair_spectator_oxide_charge_ignores_n_methylindole() {
        let mut mol = mol_from_smiles("Cn1ccc2ccccc21").unwrap();
        let before = canonical_smiles(&mol);
        repair_spectator_oxide_charge(&mut mol);
        assert_eq!(
            canonical_smiles(&mol),
            before,
            "N-methylindole must stay neutral"
        );
    }

    #[test]
    fn repair_spectator_oxide_charge_ignores_n_arylpyrrole() {
        let mut mol = mol_from_smiles("c1ccccc1n1cccc1").unwrap();
        let before = canonical_smiles(&mol);
        repair_spectator_oxide_charge(&mut mol);
        assert_eq!(
            canonical_smiles(&mol),
            before,
            "N-phenylpyrrole must stay neutral"
        );
    }

    #[test]
    fn apply_retro_l4703_biaryl_stille_disconnection_produces_parseable_n_oxide_precursor() {
        // End-to-end regression for uspto50k_test#L4703: the exact SMIRKS
        // (templates_2000.smi entry `extracted_1312`) applied to the exact
        // target must now produce a precursor set whose N-oxide fragment
        // round-trips, instead of the pre-fix `n1(...)[O-]` defect.
        let smirks = "[#7:5]:[c:4](-[c:1](:[c:2]):[c:3]):[c:6]>>Br-[c:1](:[c:2]):[c:3].C-C-C-[CH2]-[Sn](-[CH2]-C-C-C)(-[CH2]-C-C-C)-[c:4](:[#7:5]):[c:6]";
        let rule = rr("extracted_1312", smirks);
        let mol = mol_from_smiles("COC(=O)c1ccc[n+]([O-])c1-c1ccc(F)cc1").unwrap();
        let outcomes = apply_retro(&mol, &rule);
        assert!(
            !outcomes.is_empty(),
            "the Stille disconnection must still be found"
        );
        let n_oxide_precursor = outcomes
            .iter()
            .flatten()
            .find(|p| p.smiles.contains("[n+]") || p.smiles.contains("[Sn]"))
            .unwrap_or_else(|| {
                panic!(
                    "no N-oxide/Sn precursor among outcomes: {outcomes:?}",
                    outcomes = outcomes
                        .iter()
                        .flatten()
                        .map(|p| &p.smiles)
                        .collect::<Vec<_>>()
                )
            });
        assert!(
            mol_from_smiles(&n_oxide_precursor.smiles).is_ok(),
            "precursor must round-trip: {}",
            n_oxide_precursor.smiles
        );
    }

    // ── Hash-atom ([#N]) wildcard: application-time compatibility ──────

    #[test]
    fn hash_atom_not_applicable_for_plain_smirks() {
        let smirks = "[N:1][CH2:2][c:3]>>[N:1].[Br][CH2:2][c:3]";
        assert!(matches!(
            expand_hash_atom_variants(smirks),
            HashAtomExpansion::NotApplicable
        ));
        assert_eq!(
            concrete_application_status(smirks),
            ConcreteApplicationStatus::Direct
        );
    }

    #[test]
    fn hash_atom_expands_bare_nitrogen_wildcard_into_validated_variants() {
        // Real extracted template (2-anilinopyrimidine-class retro):
        // "any nitrogen" on both ring positions, aromaticity unspecified.
        let smirks = "[#7:2]:[c:1](-[NH:4]-[c:5]):[#7:3]>>Cl-[c:1](:[#7:2]):[#7:3].[NH2:4]-[c:5]";
        match expand_hash_atom_variants(smirks) {
            HashAtomExpansion::Expanded { variants } => {
                assert!(!variants.is_empty());
                for v in &variants {
                    assert!(
                        chematic::rxn::parse_reaction(v).is_ok(),
                        "every returned variant must independently re-parse: {v}"
                    );
                    assert!(
                        !v.contains('#'),
                        "no variant may still contain a hash atom: {v}"
                    );
                }
                // The all-aromatic reading must be among the survivors --
                // this is the chemically-correct one for a pyrimidine ring.
                assert!(
                    variants
                        .iter()
                        .any(|v| v.contains("[n:2]") && v.contains("[n:3]")),
                    "expected an all-aromatic variant among {variants:?}"
                );
            }
            other => panic!("expected Expanded, got {other:?}"),
        }
    }

    #[test]
    fn hash_atom_allows_independent_aromaticity_choice_per_side() {
        // Same atom-map (2) appears on both LHS and RHS. Aromatization /
        // dearomatization is a real reaction class, so the two sides must
        // be allowed to pick *different* readings -- not forced to agree.
        let smirks = "[#7:2]:[c:1]:[c:3]>>Cl-[c:1](=[#7:2])-[c:3]";
        let HashAtomExpansion::Expanded { variants } = expand_hash_atom_variants(smirks) else {
            panic!("expected an Expanded outcome");
        };
        let has_mixed_reading = variants.iter().any(|v| {
            let lhs = v.split(">>").next().unwrap();
            let rhs = v.split(">>").nth(1).unwrap();
            lhs.contains("[n:2]") != rhs.contains("[n:2]")
        });
        assert!(
            has_mixed_reading,
            "expected at least one variant where the two sides disagree on aromaticity \
             for atom-map 2, among {variants:?}"
        );
    }

    #[test]
    fn hash_atom_same_side_same_atom_map_must_agree_with_itself() {
        // Atom-map 2 appears twice, both on the LHS -- same query atom
        // instance, so both occurrences must resolve to the same reading
        // within any one variant (this is a within-side constraint, not
        // the now-relaxed cross-side one).
        let smirks = "[#7:2]-[C:1]-[#7:2]>>[N:1]";
        let HashAtomExpansion::Expanded { variants } = expand_hash_atom_variants(smirks) else {
            panic!("expected an Expanded outcome");
        };
        for v in &variants {
            let lhs = v.split(">>").next().unwrap();
            let upper_count = lhs.matches("[N:2]").count();
            let lower_count = lhs.matches("[n:2]").count();
            assert!(
                upper_count == 0 || lower_count == 0,
                "both LHS occurrences of atom-map 2 must share one reading within a variant: {v}"
            );
        }
    }

    #[test]
    fn hash_atom_bails_on_inconsistent_element_for_same_atom_map() {
        // Synthetic: atom-map 2 is #7 (N) on one side, #8 (O) on the other --
        // internally inconsistent, must not guess a choice.
        let smirks = "[#7:2]-[C:1]>>[#8:2]-[C:1]";
        assert_eq!(
            concrete_application_status(smirks),
            ConcreteApplicationStatus::Unsupported {
                reason: HashAtomUnsupportedReason::InconsistentElement
            }
        );
    }

    #[test]
    fn hash_atom_bails_on_combined_primitive() {
        // `#7` combined with another primitive in the same bracket --
        // outside what this expansion attempts to rewrite safely.
        let smirks = "[#7;+0:2]-[C:1]>>[N:2]-[C:1]";
        assert_eq!(
            concrete_application_status(smirks),
            ConcreteApplicationStatus::Unsupported {
                reason: HashAtomUnsupportedReason::UnhandledSyntax
            }
        );
    }

    #[test]
    fn hash_atom_expansion_fails_closed_when_combinatorial_space_exceeds_cap() {
        // 10 distinct unmapped hash atoms (5 per side, no atom-map to
        // share a choice across) -> 2^10 = 1024 combinations, over the
        // 64-variant cap. Must be reported fully unsupported -- never a
        // silently truncated partial subset.
        let smirks = "[#7]-[#8]-[#16]-[#7]-[#8]>>[#7]-[#8]-[#16]-[#7]-[#8]";
        assert_eq!(
            concrete_application_status(smirks),
            ConcreteApplicationStatus::Unsupported {
                reason: HashAtomUnsupportedReason::VariantLimitExceeded {
                    total_combinations: 1024
                }
            }
        );
        assert!(application_smirks_variants(smirks).is_empty());
    }

    #[test]
    fn triple_bond_hash_character_is_not_mistaken_for_a_hash_atom() {
        // `#` is also the ordinary SMILES/SMARTS triple-bond symbol
        // (unrelated to the `[#N]` atomic-number primitive). A real
        // checked-in template using it: nitrile hydrolysis retro
        // (`extracted_166` in `data/templates_extracted.smi`).
        let smirks = "[C:2]-[C:1]#[N:3]>>O=[C:1](-[C:2])-[NH2:3]";
        assert_eq!(
            concrete_application_status(smirks),
            ConcreteApplicationStatus::Direct,
            "a triple bond outside any bracket must never be treated as a [#N] atom"
        );
        // The real bug this guards: a caller-side `smirks.contains('#')`
        // pre-check alone would misroute this into the hash-atom path;
        // `application_smirks_variants` must still be a safe pass-through
        // even if a caller's pre-check is that naive.
        assert_eq!(
            application_smirks_variants(smirks).as_slice(),
            &[smirks.to_string()]
        );
    }

    #[test]
    fn load_rules_from_file_keeps_one_logical_rule_per_line_hash_atom_smirks_unchanged() {
        // Root invariant this redesign restores: the loader must not know
        // or care about hash-atom expansion at all -- exactly one
        // RetroRule per raw line, `smirks` byte-identical to the file,
        // `#` and all. Downstream consumers (ONNX template scorer,
        // candidate-pool export) depend on this count and this identity.
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "renkin_hash_atom_loader_test_{}.smi",
            std::process::id()
        ));
        let plain = "[N:1][CH2:2][c:3]>>[N:1].[Br][CH2:2][c:3]";
        let hash_atom =
            "[#7:2]:[c:1](-[NH:4]-[c:5]):[#7:3]>>Cl-[c:1](:[#7:2]):[#7:3].[NH2:4]-[c:5]";
        std::fs::write(&path, format!("{plain}\t10\n{hash_atom}\t167\n")).unwrap();

        let rules = load_rules_from_file(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();

        assert_eq!(rules.len(), 2, "exactly one RetroRule per raw line");
        assert_eq!(rules[0].smirks, plain);
        assert_eq!(rules[0].name, "extracted_0");
        assert_eq!(
            rules[1].smirks, hash_atom,
            "hash-atom SMIRKS must be stored unchanged"
        );
        assert_eq!(rules[1].name, "extracted_1");
        assert_eq!(rules[1].template_id, template_id_for_smirks(hash_atom));

        // The candidate-pool exporter's indexer relies on this: same
        // template_id must always imply the same name/smirks/weight,
        // trivially true here since there is exactly one rule per
        // template_id.
        let mut by_id = std::collections::HashMap::new();
        for r in &rules {
            assert!(
                by_id
                    .insert(r.template_id.clone(), (&r.name, &r.smirks))
                    .is_none(),
                "template_id must be unique across the loaded rule set"
            );
        }
    }

    #[test]
    fn apply_retro_succeeds_end_to_end_on_hash_atom_template_without_changing_rule_identity() {
        // Regression test for the real bug (Issue #88): applying the
        // *unmodified* `[#7:2]`-bearing RetroRule -- exactly what
        // `load_rules_from_file` now produces, byte-for-byte -- must
        // transparently succeed via the internal variant path, without
        // the caller ever seeing a different template_id/name/smirks.
        let hash_atom_retro =
            "[#7:2]:[c:1](-[NH:4]-[c:5]):[#7:3]>>Cl-[c:1](:[#7:2]):[#7:3].[NH2:4]-[c:5]";
        let rule = RetroRule {
            name: "extracted_21".to_string(),
            template_id: template_id_for_smirks(hash_atom_retro),
            smirks: hash_atom_retro.to_string(),
            weight: 1.0,
            required_elements: required_elements_from_smirks(hash_atom_retro),
        };

        // Atom-maps 2 and 3 each appear on both sides of `>>`, bonded to
        // map 1 by the same aromatic (`:`) bond on both sides -- both are
        // genuine spectators (Issue #90's `MappedAtomRole::Spectator`), not
        // reaction centers, so each now gets ONE combined LHS/RHS group
        // instead of two independent ones (2 groups x 2 readings = 4
        // combinations, not the pre-#90 16 -- the previous 16 included
        // spurious cross-side N/n combinations for these same spectator
        // maps that this template's own test simply never happened to
        // exercise as invalid output).
        assert_eq!(
            concrete_application_status(&rule.smirks),
            ConcreteApplicationStatus::HashAtomVariants { variant_count: 4 }
        );

        let target = mol_from_smiles("c1ccc(Nc2ncccn2)cc1").unwrap(); // 2-anilinopyrimidine
        let outcomes = apply_retro(&target, &rule);
        assert!(
            !outcomes.is_empty(),
            "apply_retro must succeed on the unmodified rule via the internal variant path"
        );
        let mut found_expected = false;
        for outcome in &outcomes {
            let smiles: Vec<&str> = outcome.iter().map(|p| p.smiles.as_str()).collect();
            let has_chloropyrimidine = smiles
                .iter()
                .any(|s| s.contains("Cl") && (s.contains('n') || s.contains('N')));
            let has_aniline = smiles.iter().any(|s| {
                mol_from_smiles(s)
                    .map(|m| m.atom_count() == 7) // aniline: C6H5NH2 -- 6 C + 1 N heavy atoms
                    .unwrap_or(false)
            });
            if has_chloropyrimidine && has_aniline {
                found_expected = true;
            }
        }
        assert!(
            found_expected,
            "expected 2-chloropyrimidine + aniline among outcomes: {:?}",
            outcomes
                .iter()
                .map(|o| o.iter().map(|p| &p.smiles).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );

        // `rule` itself must be untouched by applying it.
        assert_eq!(rule.smirks, hash_atom_retro);
        assert_eq!(rule.name, "extracted_21");
    }

    #[test]
    fn apply_retro_dedupes_identical_outcomes_across_hash_atom_variants() {
        // If two variants happen to produce the exact same precursor set
        // for a given molecule, apply_retro must report it once, not
        // once per matching variant.
        let smirks = "[#7:1]-[CH3:2]>>[#7:1]-[H].[CH3:2]-Cl";
        let target = mol_from_smiles("CNC").unwrap(); // dimethylamine, matches both N-CH3 bonds
        let rule = RetroRule {
            name: "extracted_test".to_string(),
            template_id: template_id_for_smirks(smirks),
            smirks: smirks.to_string(),
            weight: 1.0,
            required_elements: 0,
        };
        let outcomes = apply_retro(&target, &rule);
        let mut signatures: Vec<Vec<String>> = outcomes
            .iter()
            .map(|o| {
                let mut s: Vec<String> = o.iter().map(|p| p.smiles.clone()).collect();
                s.sort_unstable();
                s
            })
            .collect();
        signatures.sort();
        let mut deduped = signatures.clone();
        deduped.dedup();
        assert_eq!(
            signatures, deduped,
            "apply_retro must not report the same precursor set twice: {signatures:?}"
        );
    }

    #[test]
    fn apply_retro_rejects_spectator_atom_aromaticity_flip_with_unrelated_ring() {
        // Issue #90 minimal reproducer. `extracted_45` (real corpus
        // template, data/templates_extracted_500.smi line 50): atom-map 2
        // is a pure spectator, same position on both sides of `>>`. Before
        // the fix, `expand_hash_atom_variants` gave each side an
        // independent aromaticity choice for it, so a variant like
        // `[N:2]-[CH2:1]-[C:3]>>O=[C:1](-[n:2])-[C:3]` would match a real,
        // non-aromatic nitrogen on the LHS (correct) and spell that same
        // atom as aromatic `n` on the RHS (wrong) -- confirmed to produce
        // acyclic aromatic-`n` outcomes (`c1c(cccc1)CCC(=O)nCC` and
        // `c1c(CCCnC(=O)C)cccc1`) when run directly, against real master
        // pre-fix. The pre-existing whole-fragment `has_aromatic &&
        // !has_ring` guard in `split_fragments` didn't catch it because the
        // *fragment* (not the corrupted atom) does contain a real ring --
        // the phenyl below -- which is exactly why this needed an
        // atom-level fix, not a better text heuristic.
        let smirks = "[#7:2]-[CH2:1]-[C:3]>>O=[C:1](-[#7:2])-[C:3]";
        let rule = RetroRule {
            name: "extracted_45".to_string(),
            template_id: template_id_for_smirks(smirks),
            smirks: smirks.to_string(),
            weight: 1.0,
            required_elements: required_elements_from_smirks(smirks),
        };
        let target = mol_from_smiles("c1ccccc1CCCNCC").unwrap();
        let outcomes = apply_retro(&target, &rule);
        assert!(
            !outcomes.is_empty(),
            "the real (non-spectator-corrupted) N->N reading must still succeed"
        );
        for outcome in &outcomes {
            for p in outcome {
                assert_eq!(
                    aromaticity_integrity_violation(&p.mol),
                    None,
                    "outcome {:?} must not contain an aromaticity-integrity violation",
                    p.smiles
                );
            }
        }
    }

    #[test]
    fn apply_retro_rejects_spectator_atom_aromaticity_flip_on_real_ring() {
        // Same root cause as above, but the spectator nitrogen this time
        // sits on a *real* ring (a Boc-piperazine), not acyclically. Before
        // the fix this was confirmed to still be flagged internally as
        // `aromatic=true` with zero incident `BondOrder::Aromatic` bonds
        // (i.e. it fails the same atom-level check), even though at least
        // one external SMILES sanitizer (a downstream tool re-parsing the
        // canonical SMILES text) silently repairs `n`->`N` for this
        // specific ring shape instead of rejecting it outright -- unlike
        // the acyclic case above, which such a re-parse rejects outright.
        // A check that only looked at "does this re-parse" would miss this
        // case; this test locks in that it's caught at generation time
        // regardless of whether a downstream parser would happen to notice.
        let smirks = "[#7:2]-[CH2:1]-[C:3]>>O=[C:1](-[#7:2])-[C:3]";
        let rule = RetroRule {
            name: "extracted_45".to_string(),
            template_id: template_id_for_smirks(smirks),
            smirks: smirks.to_string(),
            weight: 1.0,
            required_elements: required_elements_from_smirks(smirks),
        };
        let target = mol_from_smiles("O=C(OC)C1CN(CCN1)C(=O)OC(C)(C)C").unwrap();
        let outcomes = apply_retro(&target, &rule);
        for outcome in &outcomes {
            for p in outcome {
                assert_eq!(
                    aromaticity_integrity_violation(&p.mol),
                    None,
                    "outcome {:?} must not contain an aromaticity-integrity violation",
                    p.smiles
                );
            }
        }
    }

    #[test]
    fn aromaticity_integrity_accepts_valid_acetanilide() {
        let mol = mol_from_smiles("CC(=O)Nc1ccccc1").unwrap(); // acetanilide, real, valid
        assert_eq!(aromaticity_integrity_violation(&mol), None);
    }

    #[test]
    fn aromaticity_integrity_accepts_real_heteroaromatic_ring() {
        // Pyridine: a real aromatic ring where every aromatic atom has both
        // ring membership and an incident aromatic bond -- must not be
        // flagged, mirroring the pre-existing 4-bromopyridine concern
        // documented on `split_fragments`.
        let mol = mol_from_smiles("c1ccncc1").unwrap();
        assert_eq!(aromaticity_integrity_violation(&mol), None);
    }

    #[test]
    fn aromaticity_integrity_violation_detects_acyclic_aromatic_atom() {
        // Issue #90's exact known-bad hash-atom variant, applied directly
        // via `run_reactants` -- bypassing `expand_hash_atom_variants`'
        // now-fixed spectator grouping entirely -- to prove the atom-level
        // integrity check (the second, independent layer of defense) really
        // does detect this defect on its own. Without a test like this, the
        // grouping fix alone could make every reproducer above pass for the
        // wrong reason: because the bad variant is never generated, not
        // because this check would catch it if it somehow were.
        let bad_variant = "[N:2]-[CH2:1]-[C:3]>>O=[C:1](-[n:2])-[C:3]";
        let target = mol_from_smiles("c1ccccc1CCCNCC").unwrap();
        let results = run_reactants(bad_variant, &[&target]).unwrap_or_default();
        assert!(
            !results.is_empty(),
            "the bad variant must still match an acyclic amine (that's what makes it dangerous)"
        );
        for group in &results {
            for product in group {
                assert_eq!(
                    aromaticity_integrity_violation(product),
                    Some(AromaticityIntegrityViolation::AromaticAtomNotInRing),
                    "product {:?} must be flagged AromaticAtomNotInRing",
                    canonical_smiles(product)
                );
            }
        }
    }

    #[test]
    fn aromaticity_integrity_violation_detects_ring_atom_without_aromatic_bond() {
        // Same known-bad variant, but the spectator N it corrupts is a real
        // piperazine ring atom this time -- confirms the check catches both
        // failure shapes uniformly (acyclic above, on-ring here), the same
        // target as `apply_retro_rejects_spectator_atom_aromaticity_flip_on_real_ring`
        // but exercising the raw defect directly rather than relying on the
        // grouping fix to have already prevented it from ever being generated.
        let bad_variant = "[N:2]-[CH2:1]-[C:3]>>O=[C:1](-[n:2])-[C:3]";
        let target = mol_from_smiles("O=C(OC)C1CN(CCN1)C(=O)OC(C)(C)C").unwrap();
        let results = run_reactants(bad_variant, &[&target]).unwrap_or_default();
        assert!(
            !results.is_empty(),
            "the bad variant must still match a piperazine ring nitrogen"
        );
        for group in &results {
            for product in group {
                assert_eq!(
                    aromaticity_integrity_violation(product),
                    Some(AromaticityIntegrityViolation::AromaticAtomWithoutAromaticBond),
                    "product {:?} must be flagged AromaticAtomWithoutAromaticBond",
                    canonical_smiles(product)
                );
            }
        }
    }

    #[test]
    fn building_block_recognized_by_exact_match() {
        let env = env_aspirin_bbs();
        let mol = mol_from_smiles("CC(=O)O").unwrap();
        assert!(
            env.is_building_block(&mol),
            "acetic acid should be a building block"
        );
    }

    #[test]
    fn non_building_block_rejected() {
        let env = env_aspirin_bbs();
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        assert!(
            !env.is_building_block(&mol),
            "aspirin should not be a building block"
        );
    }

    #[test]
    fn building_block_canonical_form_variant() {
        // Canonical-SMILES lookup must match regardless of input notation
        // (same molecule, different SMILES string) — no substructure
        // matching involved, just canonicalization (L2 in lessons.md).
        let env = ChemEnv::in_memory(&["CC(=O)O"]);
        let mol = mol_from_smiles("OC(C)=O").unwrap(); // different SMILES, same molecule
        assert!(
            env.is_building_block(&mol),
            "OC(C)=O is the same as CC(=O)O"
        );
    }

    #[test]
    fn benzoic_acid_variant_matches() {
        // Different SMILES representations of the *same* benzoic acid
        // molecule must match via canonicalization (L2 in lessons.md).
        let env = ChemEnv::in_memory(&["c1ccccc1C(=O)O"]);
        let mol = mol_from_smiles("c1c(C(=O)O)cccc1").unwrap();
        assert!(
            env.is_building_block(&mol),
            "c1c(C(=O)O)cccc1 is benzoic acid"
        );
    }

    #[test]
    fn stock_membership_requires_exact_identity_not_substructure() {
        // A stock containing a larger molecule must not make a genuine
        // substructure of it register as a stock hit. Guards against
        // reintroducing subgraph/VF2-style matching for stock identity.
        let env = ChemEnv::in_memory(&["Cc1ccccc1"]); // toluene only, no benzene
        let benzene = mol_from_smiles("c1ccccc1").unwrap();
        assert!(
            !env.is_building_block(&benzene),
            "benzene is a substructure of toluene but is not the same molecule"
        );
    }

    #[test]
    #[ignore = "one-off diagnostic for issue #71: sweeps the full 402-compound \
        stock against every one-step retro-fragment of the real 4,903-target \
        corpus. Run explicitly with `cargo test --lib -- --ignored --nocapture \
        issue_71_before_after_stock_identity_diff`."]
    fn issue_71_before_after_stock_identity_diff() {
        use chematic::smarts::{QueryMolecule, find_matches, parse_smarts};
        use std::collections::HashSet;

        /// Recreates the pre-fix VF2-inclusive membership check, for
        /// before/after comparison only. Not used anywhere outside this test.
        struct OldEnv {
            canon_set: HashSet<String>,
            vf2_index: FxHashMap<(usize, usize), Vec<QueryMolecule>>,
        }
        impl OldEnv {
            fn load(path: &str) -> Self {
                let content = std::fs::read_to_string(path).unwrap();
                let mut canon_set = HashSet::new();
                let mut vf2_raw = Vec::new();
                for line in content
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                {
                    let Some(smiles) = line.split_whitespace().next() else {
                        continue;
                    };
                    let Ok(mol) = parse(smiles) else { continue };
                    let canon = canonical_smiles(&mol);
                    if !canon_set.insert(canon) {
                        continue;
                    }
                    if let Ok(query) = parse_smarts(smiles) {
                        vf2_raw.push((mol.atom_count(), mol.bonds().count(), query));
                    }
                }
                let mut vf2_index: FxHashMap<(usize, usize), Vec<QueryMolecule>> =
                    FxHashMap::default();
                for (a, b, q) in vf2_raw {
                    vf2_index.entry((a, b)).or_default().push(q);
                }
                Self {
                    canon_set,
                    vf2_index,
                }
            }
            fn is_building_block(&self, mol: &Molecule) -> bool {
                let canon = canonical_smiles(mol);
                if self.canon_set.contains(&canon) {
                    return true;
                }
                let key = (mol.atom_count(), mol.bonds().count());
                if let Some(cands) = self.vf2_index.get(&key) {
                    let n = mol.atom_count();
                    return cands
                        .iter()
                        .any(|q| find_matches(q, mol).iter().any(|m| m.len() == n));
                }
                false
            }
        }

        let old_env = OldEnv::load("data/building_blocks.smi");
        let new_env = ChemEnv::load("data/building_blocks.smi").unwrap();
        let rules = default_rules();

        // Probe set: every distinct one-step retro-fragment SMILES produced
        // by applying every default rule to every real target in the
        // 4,903-target corpus (data/comparison/sample_full_sorted.jsonl).
        // Single-level apply_retro only (no multi-step search), so this is
        // fast despite the corpus size.
        let corpus = std::fs::read_to_string("data/comparison/sample_full_sorted.jsonl").unwrap();
        let mut probe_smiles: HashSet<String> = HashSet::new();
        for line in corpus.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(smi) = v.get("canonical_smiles").and_then(|s| s.as_str()) else {
                continue;
            };
            let Ok(mol) = mol_from_smiles(smi) else {
                continue;
            };
            for rule in &rules {
                for prec_set in apply_retro(&mol, rule) {
                    for p in prec_set {
                        probe_smiles.insert(p.smiles);
                    }
                }
            }
        }

        let mut only_old: Vec<&str> = Vec::new();
        let mut only_new: Vec<&str> = Vec::new();
        let mut both = 0usize;
        let mut neither = 0usize;
        for smi in &probe_smiles {
            let Ok(mol) = mol_from_smiles(smi) else {
                continue;
            };
            match (
                old_env.is_building_block(&mol),
                new_env.is_building_block(&mol),
            ) {
                (true, true) => both += 1,
                (true, false) => only_old.push(smi),
                (false, true) => only_new.push(smi),
                (false, false) => neither += 1,
            }
        }
        only_old.sort_unstable();
        only_new.sort_unstable();

        println!("probe set size: {}", probe_smiles.len());
        println!("both accept (real matches, unaffected): {both}");
        println!("neither accepts: {neither}");
        println!(
            "OLD-only accepts (VF2 false positives fixed by this PR): {}",
            only_old.len()
        );
        for s in &only_old {
            println!("  FIXED FALSE POSITIVE: {s}");
        }
        println!(
            "NEW-only accepts (previously-missed true matches, now correct): {}",
            only_new.len()
        );
        for s in &only_new {
            println!("  NEWLY CORRECT MATCH: {s}");
        }
    }

    #[test]
    fn vf2_false_positive_regressions_from_issue_71() {
        // Real false positives found during the #66 benchmark's per-target
        // audit (data/comparison/results_100/per_target_audit.md in
        // renkin-compare-66, "3 common-stock-fail targets"). Before this
        // fix, ChemEnv::is_building_block's VF2 subgraph fallback accepted
        // each of these as a stock hit even though none is present in
        // data/building_blocks.smi under any SMILES notation or
        // canonicalization (independently re-verified via RDKit over the
        // full stock file during the audit).
        let env = ChemEnv::load("data/building_blocks.smi").unwrap();
        let false_positives = [
            (
                "C=C/C/C=C",
                "1,4-pentadiene, via cc_single_cleavage (#L679)",
            ),
            ("O=C/C(=O)O", "glyoxylic acid, via wittig_retro (#L1640)"),
            (
                "c1ccc(cc1)CC=O",
                "phenylacetaldehyde, via co_aliphatic_cleavage (#L4575)",
            ),
        ];
        for (smiles, label) in false_positives {
            let mol = mol_from_smiles(smiles).unwrap();
            assert!(
                !env.is_building_block(&mol),
                "{label} ({smiles}) must not be a stock hit — it is not in data/building_blocks.smi"
            );
        }
    }

    #[test]
    fn ester_cleavage_fires_on_aspirin() {
        // Graph-based ester cleavage: aspirin → acetic acid + salicylic acid
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        let rule = rr("ester_cleavage", ""); // graph-based (empty smirks)
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty(), "ester_cleavage must match aspirin");
        // All precursor SMILES must parse cleanly.
        for prec_set in &results {
            for p in prec_set {
                assert!(
                    mol_from_smiles(&p.smiles).is_ok(),
                    "invalid precursor: {}",
                    p.smiles
                );
            }
        }
    }

    #[test]
    fn ester_cleavage_skips_free_acid() {
        // Free carboxylic acids should not be split at their terminal OH.
        let mol = mol_from_smiles("CC(=O)O").unwrap(); // acetic acid
        let rule = rr("ester_cleavage", "");
        let results = apply_retro(&mol, &rule);
        // No meaningful split: the only C-O single bond is the terminal OH
        assert!(
            results.is_empty(),
            "free carboxylic acid should not be cleaved"
        );
    }

    #[test]
    #[cfg(not(feature = "perf-instrumentation"))]
    fn apply_retro_call_count_is_zero_without_perf_instrumentation() {
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        let rule = rr("ester_cleavage", "");
        apply_retro(&mol, &rule);
        assert_eq!(
            apply_retro_call_count(),
            0,
            "without perf-instrumentation the counter must never move"
        );
    }

    #[test]
    #[cfg(feature = "perf-instrumentation")]
    fn apply_retro_call_count_tracks_calls_with_perf_instrumentation() {
        // Tests in this module run concurrently and share the same global
        // counter, so this asserts a lower bound after our own known number
        // of calls (>=), not exact equality -- other tests' calls landing in
        // the same window can only push the count higher, never lower.
        reset_apply_retro_call_count();
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        let rule = rr("ester_cleavage", "");
        apply_retro(&mol, &rule);
        apply_retro(&mol, &rule);
        assert!(
            apply_retro_call_count() >= 2,
            "two apply_retro calls after reset must be reflected in the counter"
        );
    }

    #[test]
    fn ester_cleavage_ethyl_benzoate() {
        // Ethyl benzoate → benzoic acid + ethanol
        let mol = mol_from_smiles("CCOC(=O)c1ccccc1").unwrap();
        let rule = rr("ester_cleavage", "");
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "ethyl benzoate ester cleavage must fire"
        );
    }

    #[test]
    fn sulfonamide_cleavage_fires_on_aryl_sulfonamide() {
        // N-phenyl benzenesulfonamide → benzenesulfonyl chloride + aniline
        let mol = mol_from_smiles("O=S(=O)(c1ccccc1)Nc1ccccc1").unwrap();
        let rule = rr("sulfonamide_retro", ""); // graph-based
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty(), "aryl sulfonamide cleavage must fire");
        // All precursors must parse cleanly (no invalid fragments).
        for prec_set in &results {
            for p in prec_set {
                assert!(
                    mol_from_smiles(&p.smiles).is_ok(),
                    "invalid precursor: {}",
                    p.smiles
                );
            }
        }
    }

    #[test]
    fn sulfonamide_cleavage_skips_non_sulfonyl() {
        // A sulfoxide (one =O) or amine without sulfonyl must not be cleaved as sulfonamide.
        // Sulfanilamide's N-S? Use a plain sulfenamide-like S-N without two =O.
        let mol = mol_from_smiles("CSNc1ccccc1").unwrap(); // S has no =O → not a sulfonyl
        let rule = rr("sulfonamide_retro", "");
        let results = apply_retro(&mol, &rule);
        assert!(
            results.is_empty(),
            "non-sulfonyl S-N must not be cleaved as sulfonamide"
        );
    }

    #[test]
    fn aromatic_ring_fragment_filter() {
        use chematic::chem::aromatic_ring_count;
        // Open-chain aromatic fragments (BFS leakage, L4) must be discarded.
        let mol = mol_from_smiles("c1ccc(N)cc1C(=O)O").unwrap();
        let rule = rr(
            "aryl_carboxylation_retro",
            "[c:1][C:2](=O)O>>[c:1].[C:2](=O)O",
        );
        let results = apply_retro(&mol, &rule);
        // All returned fragments must have rings if they contain aromatic atoms.
        for precursor_set in &results {
            for p in precursor_set {
                let smi = &p.smiles;
                let has_lowercase = smi
                    .chars()
                    .any(|c| matches!(c, 'c' | 'n' | 'o' | 's' | 'p'));
                if has_lowercase {
                    let m = mol_from_smiles(smi).unwrap();
                    assert!(
                        aromatic_ring_count(&m) > 0,
                        "fragment '{smi}' has aromatic atoms but no ring"
                    );
                }
            }
        }
    }

    #[test]
    fn suzuki_retro_4_phenylpyridine_solvable() {
        // 4-Phenylpyridine was returning 0 routes because aromatic_ring_count()
        // returned 0 for pyridine (heteroaromatic), causing the BFS-leakage filter
        // to incorrectly discard the 4-bromopyridine fragment.
        use crate::search::{SearchConfig, find_routes};
        let bbs = [
            "Brc1ccccc1",
            "c1ccccc1",
            "Brc1ccncc1",
            "c1ccncc1",
            "OB(O)c1ccccc1",
            "OB(O)c1ccncc1",
        ];
        let env = ChemEnv::in_memory(&bbs);
        let rules = crate::chem_env::default_rules();
        let config = SearchConfig {
            max_depth: 3,
            max_routes: 5,
            beam_width: 0,
            ..Default::default()
        };
        let (routes, _) = find_routes("c1ccc(-c2ccncc2)cc1", &env, &rules, &config)
            .expect("find_routes must not error");
        assert!(
            !routes.is_empty(),
            "4-phenylpyridine must be solvable via suzuki_retro"
        );
    }

    #[test]
    fn degenerate_route_not_in_precursors() {
        // apply_retro itself does not filter self-referencing; the search does.
        // This test just verifies that for anthranilic acid the aryl_carboxylation
        // rule returns aniline-like and acid-like fragments without crashing.
        let mol = mol_from_smiles("c1ccc(N)cc1C(=O)O").unwrap();
        let rule = rr(
            "aryl_carboxylation_retro",
            "[c:1][C:2](=O)O>>[c:1].[C:2](=O)O",
        );
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty());
    }

    // ── aryl_carboxylation_retro: [OH] restricts to free acids, excludes esters ──
    //
    // Regression coverage for the ester-overmatch atom-loss bug: the old pattern
    // "[c:1][C:2](=O)O" (bare O, no H constraint) matched ester oxygens too, and
    // apply_retro then discarded the ester's real alkyl leaving group entirely,
    // fabricating free HCOOH instead — e.g. methyl benzoate COC(=O)c1ccccc1
    // produced precursors [benzene, formic acid], losing OCH3 without a trace.

    fn aryl_carboxylation_rule() -> RetroRule {
        default_rules()
            .into_iter()
            .find(|r| r.name == "aryl_carboxylation_retro")
            .expect("aryl_carboxylation_retro must be in default_rules()")
    }

    #[test]
    fn aryl_carboxylation_fires_on_benzoic_acid() {
        let mol = mol_from_smiles("OC(=O)c1ccccc1").unwrap();
        let results = apply_retro(&mol, &aryl_carboxylation_rule());
        assert!(
            !results.is_empty(),
            "free benzoic acid must still disconnect via aryl_carboxylation_retro"
        );
    }

    #[test]
    fn aryl_carboxylation_fires_on_substituted_benzoic_acid() {
        let mol = mol_from_smiles("OC(=O)c1ccc(Cl)cc1").unwrap(); // 4-chlorobenzoic acid
        let results = apply_retro(&mol, &aryl_carboxylation_rule());
        assert!(
            !results.is_empty(),
            "substituted free acid must still disconnect via aryl_carboxylation_retro"
        );
    }

    #[test]
    fn aryl_carboxylation_skips_methyl_ester() {
        let mol = mol_from_smiles("COC(=O)c1ccccc1").unwrap(); // methyl benzoate
        let results = apply_retro(&mol, &aryl_carboxylation_rule());
        assert!(
            results.is_empty(),
            "methyl benzoate must NOT disconnect via aryl_carboxylation_retro \
             (that would silently drop the OMe group — ester_cleavage is the correct rule)"
        );
    }

    #[test]
    fn aryl_carboxylation_skips_ethyl_ester() {
        let mol = mol_from_smiles("CCOC(=O)c1ccccc1").unwrap(); // ethyl benzoate
        let results = apply_retro(&mol, &aryl_carboxylation_rule());
        assert!(
            results.is_empty(),
            "ethyl benzoate must NOT disconnect via aryl_carboxylation_retro"
        );
    }

    #[test]
    fn aryl_carboxylation_skips_amide() {
        let mol = mol_from_smiles("NC(=O)c1ccccc1").unwrap(); // benzamide
        let results = apply_retro(&mol, &aryl_carboxylation_rule());
        assert!(
            results.is_empty(),
            "benzamide (N, not O) must not match the carboxylation pattern"
        );
    }

    #[test]
    fn aryl_carboxylation_skips_carboxylate_anion() {
        // Documented expected behavior: this rule targets free acids only. A
        // deprotonated carboxylate has 0 H on that oxygen (not 1), so [OH]
        // correctly excludes it — firing here would need a distinct salt-aware
        // rule/step (protonation), not silently treated as equivalent to the
        // free acid.
        let mol = mol_from_smiles("[O-]C(=O)c1ccccc1").unwrap(); // benzoate anion
        let results = apply_retro(&mol, &aryl_carboxylation_rule());
        assert!(
            results.is_empty(),
            "carboxylate anion must not fire aryl_carboxylation_retro (free-acid-only by design)"
        );
    }

    // ── aryl_ether_retro: [O;!$(OC=O)] restricts to genuine ethers, excludes esters ──
    //
    // Regression coverage for the ester-mislabeling bug: the old pattern
    // "[c:1][O:2]" (bare O, no exclusion) also matched an ester's
    // Ar-O-C(=O)-R oxygen, mislabeling an ester cleavage as a retro-Ullmann
    // ether disconnection (wrong reaction_family/conditions/procedure_hint
    // on a route ester_cleavage already found correctly) -- e.g. aspirin
    // produced a spurious duplicate route. See
    // docs/design/retro-rule-precision-gaps-v0.md #1.

    fn aryl_ether_rule() -> RetroRule {
        default_rules()
            .into_iter()
            .find(|r| r.name == "aryl_ether_retro")
            .expect("aryl_ether_retro must be in default_rules()")
    }

    #[test]
    fn aryl_ether_retro_fires_on_diaryl_ether() {
        let mol = mol_from_smiles("c1ccc(Oc2ccccc2)cc1").unwrap(); // diphenyl ether
        let results = apply_retro(&mol, &aryl_ether_rule());
        assert!(
            !results.is_empty(),
            "genuine diaryl ether must still disconnect via aryl_ether_retro"
        );
    }

    #[test]
    fn aryl_ether_retro_fires_on_aryl_alkyl_ether() {
        let mol = mol_from_smiles("COc1ccccc1").unwrap(); // anisole
        let results = apply_retro(&mol, &aryl_ether_rule());
        assert!(
            !results.is_empty(),
            "genuine aryl alkyl ether must still disconnect via aryl_ether_retro"
        );
    }

    #[test]
    fn aryl_ether_retro_skips_aryl_ester_oxygen() {
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1").unwrap(); // phenyl acetate
        let results = apply_retro(&mol, &aryl_ether_rule());
        assert!(
            results.is_empty(),
            "phenyl acetate's ester oxygen must NOT disconnect via aryl_ether_retro \
             (that mislabels an ester cleavage as an Ullmann ether disconnection -- \
             ester_cleavage is the correct rule)"
        );
    }

    #[test]
    fn aryl_ether_retro_skips_aspirin_ester_oxygen() {
        // The exact repro from docs/design/retro-rule-precision-gaps-v0.md #1:
        // aspirin's ester bond was being double-counted as both ester_cleavage
        // (correct) and aryl_ether_retro (wrong reaction_family/conditions).
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap(); // aspirin
        let results = apply_retro(&mol, &aryl_ether_rule());
        assert!(
            results.is_empty(),
            "aspirin must NOT disconnect via aryl_ether_retro -- its only aromatic \
             C-O bond is the ester oxygen, which ester_cleavage already handles correctly"
        );
    }

    #[test]
    fn methyl_benzoate_ester_cleavage_gives_correct_precursors() {
        // Proves this isn't just "candidate removed" but "routed to the correct
        // rule": ester_cleavage must produce benzoic acid + methanol.
        let mol = mol_from_smiles("COC(=O)c1ccccc1").unwrap();
        let rule = rr("ester_cleavage", "");
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "ester_cleavage must fire on methyl benzoate"
        );
        let found_correct_split = results.iter().any(|set| {
            let smiles: Vec<String> = set.iter().map(|p| p.smiles.clone()).collect();
            let has_acid = smiles.iter().any(|s| {
                mol_from_smiles(s)
                    .map(|m| {
                        canonical_smiles(&m)
                            == canonical_smiles(&mol_from_smiles("OC(=O)c1ccccc1").unwrap())
                    })
                    .unwrap_or(false)
            });
            let has_methanol = smiles.iter().any(|s| {
                mol_from_smiles(s)
                    .map(|m| {
                        canonical_smiles(&m) == canonical_smiles(&mol_from_smiles("CO").unwrap())
                    })
                    .unwrap_or(false)
            });
            has_acid && has_methanol
        });
        assert!(
            found_correct_split,
            "ester_cleavage must split methyl benzoate into benzoic acid + methanol, got: {:?}",
            results
                .iter()
                .map(|set| set.iter().map(|p| p.smiles.clone()).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );
    }

    // ── Substituent-preservation regression suite ────────────────────────────
    //
    // The aryl_carboxylation_retro bug happened because an UNMAPPED atom
    // matched a real target atom but was "recreated fresh" (implicit-H-filled)
    // in the precursor fragment, discarding whatever that real atom was really
    // bonded to. The audit that found and fixed that bug also empirically
    // checked every other rule with a mapped "leaving" atom whose H-count gets
    // re-declared between the target and precursor SMIRKS templates (the
    // mechanism that could, in principle, hit the same failure mode). All
    // checked clean: chematic's reaction engine correctly carries a MAPPED
    // atom's real substituents across the reaction. These cases pin that
    // finding down as regression coverage — if a future chematic upgrade or
    // rule edit breaks substituent preservation, this table catches it without
    // requiring the same manual audit to be repeated by hand.
    struct SubstituentPreservationCase {
        rule_name: &'static str,
        target: &'static str,
        /// A precursor fragment that must appear (by canonical SMILES) in the
        /// result, proving the target's real substituent beyond the rule's
        /// textbook pattern survived instead of being silently dropped.
        expected_preserved_fragment: &'static str,
    }

    const SUBSTITUENT_PRESERVATION_CASES: &[SubstituentPreservationCase] = &[
        SubstituentPreservationCase {
            // Ester oxygen (OMe) must survive as part of the acyl fragment,
            // not be replaced outright by the rule's hardcoded Cl.
            rule_name: "friedel_crafts_acylation_retro",
            target: "COC(=O)c1ccccc1",                // methyl benzoate
            expected_preserved_fragment: "COC(=O)Cl", // methyl chloroformate
        },
        // negishi_retro's case was removed here along with the rule itself
        // (v0.36.0 rule-safety census) -- it passed for this plain
        // acyclic substrate (benzyl alcohol -> bromobenzene + methanol),
        // but the rule produces a duplicated split on ring-fused
        // substrates, so it was disabled entirely rather than kept for
        // the subset of inputs it handles correctly. See
        // `negishi_retro_removed_from_default_rules` below.
        SubstituentPreservationCase {
            // C:1's extra branch (isopropyl) must survive into the acid fragment.
            rule_name: "claisen_retro",
            target: "CC(C)C(=O)CC(=O)OCC", // ethyl 4-methyl-3-oxopentanoate
            expected_preserved_fragment: "CC(C)C(=O)O", // isobutyric acid
        },
        // michael_retro's case was removed here along with the rule itself
        // (v0.36.0 rule-safety census) -- it passed for this plain acyclic
        // substrate, but the rule produces a corrupted split on ring-fused
        // targets, so it was disabled entirely rather than kept for the
        // subset of inputs it handles correctly. See
        // `michael_retro_removed_from_default_rules` below.
        SubstituentPreservationCase {
            // C:1's bulky tert-butyl substituent must survive, not vanish
            // when C:1 becomes a carbonyl.
            rule_name: "reductive_amination_retro",
            target: "CC(C)(C)NCC",                    // N-ethyl-tert-butylamine
            expected_preserved_fragment: "CC(C)(C)N", // tert-butylamine
        },
        SubstituentPreservationCase {
            // Both alkene substituents (extra methyls) must survive as two
            // distinct, fully-substituted carbonyl fragments.
            rule_name: "wittig_retro",
            target: "CC(C)=CC",                     // 2-methyl-2-butene
            expected_preserved_fragment: "CC(C)=O", // acetone
        },
        // grignard_addition_retro's case was removed here along with the
        // rule itself (v0.36.0 rule-safety census) -- it passed for this
        // plain acyclic substrate (3-methylpentan-3-ol -> butan-2-one +
        // ethyl), but the rule produces a duplicated split on ring-fused
        // tertiary alcohols, so it was disabled entirely rather than kept
        // for the subset of inputs it handles correctly. See
        // `grignard_addition_retro_removed_from_default_rules` below.
        // aryl_amine_retro's case was removed here along with the rule
        // itself (issue #77) — it passed for this plain acyclic substrate,
        // but the rule loses the nitrogen outright on ring-fused targets,
        // so it was disabled entirely rather than kept for the subset of
        // inputs it handles correctly. See
        // `aryl_amine_retro_removed_from_default_rules` below.
    ];

    /// Molecular formula fingerprint (element -> count, including implicit H).
    /// Used instead of canonical-SMILES string equality: chematic's canonical_smiles
    /// preserves whether an atom was written with explicit brackets (e.g. `[CH3]`,
    /// as the reaction engine emits) vs organic-subset notation (`C`, as a plain
    /// reference SMILES parses to) — chemically identical molecules can render as
    /// different canonical strings purely from that notational difference.
    fn formula_fingerprint(mol: &Molecule) -> std::collections::BTreeMap<Element, i64> {
        let mut counts = std::collections::BTreeMap::new();
        for (_, atom) in mol.atoms() {
            *counts.entry(atom.element).or_insert(0) += 1;
        }
        for h in chematic::chem::implicit_hcount_per_atom(mol) {
            if h > 0 {
                *counts.entry(Element::H).or_insert(0) += h as i64;
            }
        }
        counts
    }

    #[test]
    fn substituent_preservation_regression_suite() {
        let rules = default_rules();
        for case in SUBSTITUENT_PRESERVATION_CASES {
            let rule = rules
                .iter()
                .find(|r| r.name == case.rule_name)
                .unwrap_or_else(|| panic!("{} must be in default_rules()", case.rule_name));
            let mol = mol_from_smiles(case.target)
                .unwrap_or_else(|_| panic!("target must parse: {}", case.target));
            let results = apply_retro(&mol, rule);
            let expected_formula = formula_fingerprint(
                &mol_from_smiles(case.expected_preserved_fragment).unwrap_or_else(|_| {
                    panic!(
                        "expected_preserved_fragment must parse: {}",
                        case.expected_preserved_fragment
                    )
                }),
            );
            let found = results.iter().any(|set| {
                set.iter()
                    .any(|p| formula_fingerprint(&p.mol) == expected_formula)
            });
            assert!(
                found,
                "{}: expected precursor fragment '{}' (preserving the target's real \
                 substituent) not found for target '{}'. Got: {:?}",
                case.rule_name,
                case.expected_preserved_fragment,
                case.target,
                results
                    .iter()
                    .map(|set| set.iter().map(|p| p.smiles.clone()).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn suzuki_retro_biphenyl_gives_bromobenzene_and_phenylboronic_acid() {
        let mol = mol_from_smiles("c1ccc(-c2ccccc2)cc1").unwrap();
        let rule = rr("suzuki_retro", "");
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "suzuki_retro must find at least one biaryl disconnection"
        );

        let all_smiles: Vec<String> = results
            .iter()
            .flat_map(|set| set.iter().map(|p| p.smiles.clone()))
            .collect();

        // Expect exactly bromobenzene and phenylboronic acid -- a real
        // retro-Suzuki disconnection needs one aryl-halide partner and one
        // boron-containing partner, not "aryl halide + plain arene" (the
        // previous, buggy behavior this test used to pin; see
        // docs/design/retro-rule-precision-gaps-v0.md #2). Compare against
        // canonical forms computed at test time (not a hardcoded string) —
        // the exact canonical SMILES chematic emits for a given molecule is
        // an implementation detail that can change between chematic
        // versions (e.g. 0.4.25 wrote "Brc1ccccc1", 0.4.30 writes
        // "c1ccc(cc1)Br" for the same molecule); what must hold is chemical
        // identity, not a specific string layout.
        let bromobenzene_canon = canonical_smiles(&mol_from_smiles("Brc1ccccc1").unwrap());
        let boronic_acid_canon = canonical_smiles(&mol_from_smiles("OB(O)c1ccccc1").unwrap());
        let has_bromobenzene = all_smiles.contains(&bromobenzene_canon);
        let has_boronic_acid = all_smiles.contains(&boronic_acid_canon);
        assert!(
            has_bromobenzene,
            "expected bromobenzene fragment ({bromobenzene_canon:?}); got {all_smiles:?}"
        );
        assert!(
            has_boronic_acid,
            "expected phenylboronic acid fragment ({boronic_acid_canon:?}); got {all_smiles:?}"
        );
    }

    #[test]
    fn suzuki_retro_biphenyl_solvable_with_bb() {
        // End-to-end: the engine must resolve biphenyl given bromobenzene + phenylboronic acid as BBs.
        use crate::search::{SearchConfig, find_routes};
        let env = ChemEnv::in_memory(&["Brc1ccccc1", "OB(O)c1ccccc1"]);
        let rules = default_rules();
        let cfg = SearchConfig {
            max_depth: 2,
            max_routes: 3,
            beam_width: 0,
            ..Default::default()
        };
        let routes = find_routes("c1ccc(-c2ccccc2)cc1", &env, &rules, &cfg)
            .unwrap()
            .0;
        assert!(
            !routes.is_empty(),
            "biphenyl must be solvable with Br-PhH + PhH BBs"
        );
        assert!(
            routes.iter().any(|r| r.depth == 1),
            "should need only 1 step"
        );
    }

    #[test]
    fn suzuki_retro_4_fluorobiphenyl_solvable() {
        use crate::search::{SearchConfig, find_routes};
        let env = ChemEnv::load("data/building_blocks.smi")
            .unwrap_or_else(|_| ChemEnv::in_memory(&["Brc1ccccc1", "Brc1ccc(F)cc1", "c1ccccc1"]));
        let rules = default_rules();
        let cfg = SearchConfig {
            max_depth: 2,
            max_routes: 3,
            beam_width: 0,
            ..Default::default()
        };
        let routes = find_routes("Fc1ccc(-c2ccccc2)cc1", &env, &rules, &cfg)
            .unwrap()
            .0;
        assert!(!routes.is_empty(), "4-fluorobiphenyl must be solvable");
    }

    #[test]
    fn default_bbs_solve_biphenyl() {
        // Verify that DEFAULT_BUILDING_BLOCKS (the actual WASM runtime set) contains
        // the BBs needed for the Biphenyl (Suzuki) playground preset.
        use crate::search::{SearchConfig, find_routes};
        let env = ChemEnv::in_memory(crate::DEFAULT_BUILDING_BLOCKS);

        // First confirm bromobenzene and benzene are recognized as BBs.
        let bromobenzene = mol_from_smiles("Brc1ccccc1").unwrap();
        let benzene = mol_from_smiles("c1ccccc1").unwrap();
        assert!(
            env.is_building_block(&bromobenzene),
            "DEFAULT_BUILDING_BLOCKS must contain bromobenzene"
        );
        assert!(
            env.is_building_block(&benzene),
            "DEFAULT_BUILDING_BLOCKS must contain benzene"
        );

        let rules = default_rules();
        let cfg = SearchConfig {
            max_depth: 3,
            max_routes: 5,
            beam_width: 0,
            ..Default::default()
        };
        let routes = find_routes("c1ccc(-c2ccccc2)cc1", &env, &rules, &cfg)
            .unwrap()
            .0;
        assert!(
            !routes.is_empty(),
            "biphenyl must be solvable with DEFAULT_BUILDING_BLOCKS"
        );
    }

    #[test]
    fn amide_cleavage_paracetamol() {
        // Verify amide_cleavage rule fires on paracetamol.
        let mol = mol_from_smiles("CC(=O)Nc1ccc(O)cc1").unwrap();
        let rule = rr("amide_cleavage", "[C:1](=[O:2])[N:3]>>[C:1](=[O:2])O.[N:3]");
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "amide_cleavage must fire on paracetamol"
        );
    }

    #[test]
    fn default_bbs_solve_playground_presets() {
        // Smoke-test: every playground preset must find at least 1 route
        // using DEFAULT_BUILDING_BLOCKS. Add missing BBs to lib.rs when this fails.
        use crate::search::{SearchConfig, find_routes};
        let env = ChemEnv::in_memory(crate::DEFAULT_BUILDING_BLOCKS);
        let rules = default_rules();
        let cfg = SearchConfig {
            max_depth: 3,
            max_routes: 3,
            beam_width: 0,
            ..Default::default()
        };

        let presets = [
            ("CC(=O)Oc1ccccc1C(=O)O", "Aspirin"),
            ("CC(=O)Nc1ccc(O)cc1", "Paracetamol"),
            ("CC(=O)Nc1ccccc1", "Acetanilide"),
            ("c1ccc(-c2ccccc2)cc1", "Biphenyl"),
            ("c1ccc(-c2ccncc2)cc1", "4-Phenylpyridine"),
            ("Fc1ccc(-c2ccccc2)cc1", "4-Fluorobiphenyl"),
            ("O=Cc1ccc(-c2ccco2)nc1", "Pyridine-furan biaryl"),
            ("C=Cc1ccccc1", "Styrene"),
            ("CCOC(=O)c1ccccc1", "Ethyl benzoate"),
        ];

        for (smiles, name) in presets {
            let routes = find_routes(smiles, &env, &rules, &cfg).unwrap().0;
            assert!(
                !routes.is_empty(),
                "{name} ({smiles}) must be solvable with DEFAULT_BUILDING_BLOCKS"
            );
        }
    }

    #[test]
    fn wittig_retro_cleaves_alkene() {
        let mol = mol_from_smiles("C=C").unwrap(); // ethylene
        let rule = rr("wittig_retro", "[C:1]=[C:2]>>[C:1]=O.[C:2]=O");
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty(), "wittig_retro must match ethylene");
        // Products must contain oxygen atoms (carbonyls — canonical form may be C=O or O=C).
        let smiles: Vec<_> = results[0].iter().map(|p| p.smiles.as_str()).collect();
        assert!(
            smiles.iter().any(|s| s.contains('O')),
            "products should contain oxygen; got {smiles:?}"
        );
    }

    // ── Layer 2: graph function unit tests ───────────────────────────────────

    fn all_bond_pairs(mol: &Molecule) -> Vec<(AtomIdx, AtomIdx)> {
        mol.bonds().map(|(_, b)| (b.atom1, b.atom2)).collect()
    }

    #[test]
    fn is_bridge_bond_linear_chain() {
        // CCC: both C-C bonds are bridges (removing either disconnects the chain).
        let mol = mol_from_smiles("CCC").unwrap();
        for (a, b) in all_bond_pairs(&mol) {
            assert!(
                is_bridge_bond(&mol, a, b),
                "every bond in CCC must be a bridge"
            );
        }
    }

    #[test]
    fn is_bridge_bond_ring_is_not_bridge() {
        // Benzene: removing any single bond still leaves a path through the ring.
        let mol = mol_from_smiles("c1ccccc1").unwrap();
        for (a, b) in all_bond_pairs(&mol) {
            assert!(!is_bridge_bond(&mol, a, b), "benzene has no bridge bonds");
        }
    }

    #[test]
    fn is_bridge_bond_biphenyl_inter_ring() {
        // Biphenyl: exactly ONE inter-ring bond is a bridge; ring-internal bonds are not.
        let mol = mol_from_smiles("c1ccc(-c2ccccc2)cc1").unwrap();
        let bridges: Vec<_> = all_bond_pairs(&mol)
            .into_iter()
            .filter(|&(a, b)| is_bridge_bond(&mol, a, b))
            .collect();
        assert_eq!(bridges.len(), 1, "biphenyl must have exactly 1 bridge bond");
    }

    #[test]
    fn build_sub_molecule_with_br_gives_bromobenzene() {
        // Split biphenyl at the inter-ring bond; the phenyl component + Br should
        // produce a molecule whose canonical SMILES matches bromobenzene.
        let mol = mol_from_smiles("c1ccc(-c2ccccc2)cc1").unwrap();
        let (a, b) = all_bond_pairs(&mol)
            .into_iter()
            .find(|&(a, b)| is_bridge_bond(&mol, a, b))
            .expect("biphenyl must have a bridge bond");
        let comp = get_component(&mol, a, a, b);
        let frag = build_sub_molecule_with_br(&mol, &comp, a).unwrap();
        let smi = canonical_smiles(&frag);
        // chematic's canonical form for bromobenzene
        let expected = canonical_smiles(&mol_from_smiles("Brc1ccccc1").unwrap());
        assert_eq!(
            smi, expected,
            "phenyl + Br should give bromobenzene; got {smi}"
        );
    }

    #[test]
    fn build_sub_molecule_with_oh_gives_acetic_acid() {
        // Amide cleavage of acetanilide (CC(=O)Nc1ccccc1): C side + OH → acetic acid.
        let mol = mol_from_smiles("CC(=O)Nc1ccccc1").unwrap();
        // Find the amide C-N bond (bridge).
        let (c_idx, n_idx) = all_bond_pairs(&mol)
            .into_iter()
            .find(|&(a, b)| {
                mol.atom(a).element == Element::C
                    && mol.atom(b).element == Element::N
                    && is_bridge_bond(&mol, a, b)
                    && mol.neighbors(a).any(|(nb, bi)| {
                        mol.atom(nb).element == Element::O
                            && mol.bond(bi).order == BondOrder::Double
                    })
            })
            .or_else(|| {
                all_bond_pairs(&mol)
                    .into_iter()
                    .find(|&(a, b)| {
                        mol.atom(b).element == Element::C
                            && mol.atom(a).element == Element::N
                            && is_bridge_bond(&mol, a, b)
                            && mol.neighbors(b).any(|(nb, bi)| {
                                mol.atom(nb).element == Element::O
                                    && mol.bond(bi).order == BondOrder::Double
                            })
                    })
                    .map(|(a, b)| (b, a))
            })
            .expect("acetanilide must have an amide C-N bridge bond");
        let comp_c = get_component(&mol, c_idx, c_idx, n_idx);
        let frag = build_sub_molecule_with_oh(&mol, &comp_c, c_idx).unwrap();
        let smi = canonical_smiles(&frag);
        let expected = canonical_smiles(&mol_from_smiles("CC(=O)O").unwrap());
        assert_eq!(
            smi, expected,
            "acetyl + OH should give acetic acid; got {smi}"
        );
    }

    // ── Layer 1: retro rule unit tests ───────────────────────────────────────

    #[test]
    fn friedel_crafts_retro_on_acetophenone() {
        let mol = mol_from_smiles("CC(=O)c1ccccc1").unwrap();
        let rule = rr(
            "friedel_crafts_acylation_retro",
            "[c:1][C:2](=[O:3])>>[c:1].[C:2](=[O:3])Cl",
        );
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "friedel_crafts_retro must fire on acetophenone"
        );
        let flat: Vec<_> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.as_str()))
            .collect();
        assert!(
            flat.iter().any(|s| s.contains("Cl")),
            "products must include acyl chloride; got {flat:?}"
        );
    }

    #[test]
    fn heck_retro_terminal_on_styrene() {
        let mol = mol_from_smiles("C=Cc1ccccc1").unwrap();
        let rule = rr(
            "heck_retro_terminal",
            "[c:1][CH:2]=[CH2:3]>>[c:1][Br].[CH2:2]=[CH2:3]",
        );
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "heck_retro_terminal must fire on styrene"
        );
        let flat: Vec<String> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.clone()))
            .collect();
        assert!(
            flat.iter().any(|s| s.contains("Br")),
            "products must include aryl bromide; got {flat:?}"
        );
        // Note: chematic may serialise ethylene as "C=C" or "[CH2]=[CH2]" depending on
        // internal H-count representation; both are correct for this test.
        assert!(
            flat.iter().any(|s| s == "C=C" || s == "[CH2]=[CH2]"),
            "products must include ethylene; got {flat:?}"
        );
    }

    #[test]
    fn heck_retro_removed_from_default_rules() {
        // Old version of this test ("heck_retro_internal_on_stilbene")
        // asserted the rule's behavior on stilbene by building it directly
        // via `rr(...)` rather than routing through `default_rules()` --
        // exactly the mistake `aryl_chloride_retro_removed_from_default_rules`'s
        // own comment warns about: it would have kept passing even after
        // this removal. Replaced with the same removal-assertion +
        // frozen-fixture pattern every other ring-fusion defect removal
        // uses.
        let rules = default_rules();
        assert!(
            rules.iter().all(|r| r.name != "heck_retro"),
            "heck_retro must not be present in default_rules(): ring-fusion \
             connectivity-collapse defect on internal alkenes fused to the \
             same aromatic ring the leaving-group Br attaches to (indene-type)"
        );
        assert!(
            rules.iter().any(|r| r.name == "heck_retro_terminal"),
            "heck_retro_terminal must stay active -- its terminal CH2 endpoint \
             is structurally immune to this defect (proven by valence \
             counting, see the removal comment in default_rules())"
        );
    }

    #[test]
    fn heck_retro_would_corrupt_a_ring_fused_target_if_re_enabled() {
        // Indene: cyclopentene fused to benzene, its endocyclic C=C
        // directly attached to the aromatic ring -- the exact shape the
        // removal comment in default_rules() describes.
        let target_smiles = "C1=Cc2ccccc2C1";
        let target = mol_from_smiles(target_smiles).unwrap();
        let rule = rr("heck_retro", "[c:1][CH:2]=[CH:3]>>[c:1][Br].[CH2:2]=[CH:3]");
        let outcomes = apply_retro(&target, &rule);
        let outcome_smiles: Vec<Vec<&str>> = outcomes
            .iter()
            .map(|o| o.iter().map(|p| p.smiles.as_str()).collect())
            .collect();
        assert!(
            !outcomes.is_empty(),
            "expected at least one outcome on this real ring-fused (indene) target: \
             {outcome_smiles:?}"
        );
        // Correct chemistry: two separate fragments ([Ar-Br], [CH2=CH-...])
        // whose heavy atoms sum to target_atoms + 1 (the new Br). The
        // confirmed defect (verified 2026-08-29 against this exact target:
        // indene has 9 heavy atoms) instead produces a *single* fused
        // fragment of 8 heavy atoms -- fewer fragments than expected AND
        // fewer atoms than the target itself, the opposite direction from
        // negishi_retro/grignard_addition_retro's atom-duplication
        // signature, but the same root cause: a cut bond that's part of a
        // ring, where the SMIRKS's naive 2-fragment assumption can't
        // represent the real cyclic connectivity.
        let target_atoms = target.atom_count();
        let corrupted = outcomes.iter().any(|o| {
            let precursor_atoms: usize = o
                .iter()
                .map(|p| mol_from_smiles(&p.smiles).unwrap().atom_count())
                .sum();
            o.len() < 2 || precursor_atoms < target_atoms
        });
        assert!(
            corrupted,
            "expected at least one outcome with fewer than 2 fragments or fewer heavy atoms \
             than the target itself (connectivity collapse), got only well-formed \
             2-fragment, atom-conserving outcomes -- rule may have been fixed upstream, \
             re-check whether it's still correctly disabled: {outcome_smiles:?}"
        );
    }

    // v0.36.0 rule-safety census (docs/validation/rule-safety-census-2026-08-24.md):
    // same bare-single-atom-RHS shape as aryl_amine_retro/
    // buchwald_hartwig_retro/n_benzylation_retro/michael_retro, confirmed
    // by direct apply_retro reproduction on a real target with an Ar-CH2
    // bond inside a saturated ring fused to the aromatic ring
    // (indane/tetralin-type). negishi_retro used to have a simple-case
    // test on ethylbenzene (a plain, non-ring-fused benzylic CH2) that
    // passed cleanly -- removed along with the rule itself, since the
    // defect only shows up on ring-fused substrates and the rule is
    // disabled entirely rather than kept for the inputs it handles
    // correctly, matching this codebase's established policy for this
    // defect class.
    #[test]
    fn negishi_retro_removed_from_default_rules() {
        let rules = default_rules();
        assert!(
            rules.iter().all(|r| r.name != "negishi_retro"),
            "negishi_retro must not be present in default_rules(): same ring-fused \
             atom-duplication defect as aryl_amine_retro/buchwald_hartwig_retro/\
             n_benzylation_retro/michael_retro"
        );
    }

    #[test]
    fn negishi_retro_would_corrupt_a_ring_fused_target_if_re_enabled() {
        let target_smiles = "c1ccc2c(c1)CCC2C(=O)N3CCN(CC3)C(=O)c4ccccc4";
        let target = mol_from_smiles(target_smiles).unwrap();
        let rule = rr("negishi_retro", "[c:1][CH2:2]>>[c:1][Br].[CH3:2]");
        let outcomes = apply_retro(&target, &rule);
        let outcome_smiles: Vec<Vec<&str>> = outcomes
            .iter()
            .map(|o| o.iter().map(|p| p.smiles.as_str()).collect())
            .collect();
        assert!(
            !outcomes.is_empty(),
            "expected at least one outcome on this real ring-fused (indane-type) target: \
             {outcome_smiles:?}"
        );
        // Correct chemistry for this SMIRKS is atom-conserving except for
        // one new heavy atom (the appended leaving-group Br). The
        // confirmed defect's specific signature (verified 2026-08-24
        // against this exact target: target has 25 heavy atoms, the
        // broken outcome's precursors sum to 49 -- 23 atoms more than
        // the chemically correct 26, an ~88% excess well beyond round 1's
        // ~22-25%) is duplication, same direction and defect class as
        // n_benzylation_retro/michael_retro (under-constrained LHS + bare
        // RHS fragment + ring-membership at the cut site) -- consistent
        // with, but not directly traced to, the same BFS carry-through
        // path.
        let target_atoms = target.atom_count();
        let corrupted = outcomes.iter().any(|o| {
            let precursor_atoms: usize = o
                .iter()
                .map(|p| mol_from_smiles(&p.smiles).unwrap().atom_count())
                .sum();
            precursor_atoms > target_atoms + 1
        });
        assert!(
            corrupted,
            "expected at least one outcome with atom-duplicating ring carry-through (more \
             heavy atoms across precursors than the target's own count plus one new Br), got \
             only atom-conserving outcomes -- rule may have been fixed upstream, re-check \
             whether it's still correctly disabled: {outcome_smiles:?}"
        );
    }

    // Same v0.36.0 census flag and defect class as negishi_retro above,
    // confirmed by direct apply_retro reproduction on a real target with a
    // tertiary alcohol carbon that's part of a saturated ring fused to an
    // aromatic ring (a 2-substituted indanol). This SMIRKS's RHS declares
    // no new atoms (pure cleavage, no leaving-group atom appended), so a
    // correct outcome's precursors must sum to exactly the target's own
    // heavy-atom count; the atom-conservation violation here is even
    // larger in relative terms (~64% excess) than negishi_retro's, and
    // whether it's the identical BFS path or a related-but-distinct one
    // wasn't traced -- see the removed-rule comment above for detail.
    #[test]
    fn grignard_addition_retro_removed_from_default_rules() {
        let rules = default_rules();
        assert!(
            rules.iter().all(|r| r.name != "grignard_addition_retro"),
            "grignard_addition_retro must not be present in default_rules(): same ring-fused \
             atom-duplication defect as negishi_retro/n_benzylation_retro/michael_retro"
        );
    }

    #[test]
    fn grignard_addition_retro_would_corrupt_a_ring_fused_target_if_re_enabled() {
        let target_smiles = "c1ccc2c(c1)C(C)(O)CC2";
        let target = mol_from_smiles(target_smiles).unwrap();
        let rule = rr(
            "grignard_addition_retro",
            "[C:1]([OH:2])([C:3])[C:4]>>[C:1](=O)[C:3].[C:4]",
        );
        let outcomes = apply_retro(&target, &rule);
        let outcome_smiles: Vec<Vec<&str>> = outcomes
            .iter()
            .map(|o| o.iter().map(|p| p.smiles.as_str()).collect())
            .collect();
        assert!(
            !outcomes.is_empty(),
            "expected at least one outcome on this real ring-fused (2-substituted indanol) \
             target: {outcome_smiles:?}"
        );
        // Confirmed defect's specific signature (verified 2026-08-24
        // against this exact target: target has 11 heavy atoms, the
        // broken outcome's precursors sum to 18 -- 7 atoms more than the
        // chemically correct 11) is duplication: the atom-matcher bound
        // the "bare" [C:4] leaving-group slot to a ring-continuation bond
        // instead of a genuine exocyclic substituent, and cutting it
        // doesn't actually separate the molecule.
        let target_atoms = target.atom_count();
        let corrupted = outcomes.iter().any(|o| {
            let precursor_atoms: usize = o
                .iter()
                .map(|p| mol_from_smiles(&p.smiles).unwrap().atom_count())
                .sum();
            precursor_atoms > target_atoms
        });
        assert!(
            corrupted,
            "expected at least one outcome with atom-duplicating ring carry-through (more \
             heavy atoms across precursors than the target's own count), got only \
             atom-conserving outcomes -- rule may have been fixed upstream, re-check whether \
             it's still correctly disabled: {outcome_smiles:?}"
        );
    }

    #[test]
    fn alcohol_oxidation_retro_on_ethanol() {
        let mol = mol_from_smiles("CCO").unwrap();
        let rule = rr("alcohol_oxidation_retro", "[C:1][OH:2]>>[C:1]=O");
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "alcohol_oxidation_retro must fire on ethanol"
        );
        let flat: Vec<_> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.as_str()))
            .collect();
        assert!(
            flat.iter().any(|s| s.contains("=O") || s.contains("O=")),
            "products must include a carbonyl; got {flat:?}"
        );
    }

    // 31.11: aryl_chloride_retro, aryl_iodide_retro, and aryl_fluoride_snAr_retro
    // were removed from default_rules() — each deleted a halogen from the
    // product with no tracked precursor accounting for where it went
    // (target MW > precursor MW). The old version of this test asserted the
    // buggy behavior (rule fires, benzene is the sole "precursor"); it built
    // the rule directly via `rr(...)` rather than routing through
    // `default_rules()`, so it would have kept passing even after the rule
    // was deleted from the default set. These replacements route through
    // `default_rules()` so they fail if the removal is ever reverted.
    #[test]
    fn aryl_chloride_retro_removed_from_default_rules() {
        let rules = default_rules();
        for removed in [
            "aryl_chloride_retro",
            "aryl_iodide_retro",
            "aryl_fluoride_snAr_retro",
        ] {
            assert!(
                rules.iter().all(|r| r.name != removed),
                "{removed} must not be present in default_rules() (31.11: atom-loss, no tracked reagent)"
            );
        }
    }

    // Issue #77: aryl_amine_retro deletes a ring-fused nitrogen outright
    // instead of returning it as part of a second (amine) precursor
    // fragment — a genuine target-atom-loss defect, not the accepted
    // reagent-omission class covered by `reagent_omission_template_allowlist`
    // (synthesizability/schema.rs). Disabled pending root-cause, same policy
    // as the 31.11 halide-rule removals above.
    #[test]
    fn aryl_amine_retro_removed_from_default_rules() {
        let rules = default_rules();
        assert!(
            rules.iter().all(|r| r.name != "aryl_amine_retro"),
            "aryl_amine_retro must not be present in default_rules() (issue #77: \
             deletes ring-fused nitrogen with no tracked second precursor)"
        );
    }

    // Same root cause and mechanism as aryl_amine_retro above, confirmed by
    // direct reproduction, not just by analogy: both rules' bare
    // single-atom RHS fragments let substituent-carry-through BFS sweep
    // unchecked across a ring-fusion boundary. Worse in practice --
    // buchwald_hartwig_retro's surviving "aryl" fragment comes back
    // corrupted (a spurious extra Br plus a dangling alkyl chain), not
    // just missing the amine fragment.
    #[test]
    fn buchwald_hartwig_retro_removed_from_default_rules() {
        let rules = default_rules();
        assert!(
            rules.iter().all(|r| r.name != "buchwald_hartwig_retro"),
            "buchwald_hartwig_retro must not be present in default_rules(): same \
             ring-fused-nitrogen atom-loss defect as aryl_amine_retro (issue #77), \
             plus a corrupted surviving aryl fragment"
        );
    }

    // Direct reproduction of the removed rule's failure on a real,
    // deliberately re-added instance -- documents *why* it's gone, not
    // just *that* it's gone. Same target shape as the issue #77 repro
    // (uspto50k_test#L2263): a fused aromatic/saturated-ring nitrogen.
    #[test]
    fn buchwald_hartwig_retro_would_corrupt_a_ring_fused_target_if_re_enabled() {
        let target = mol_from_smiles("c12c(NCCC2)ccc(c1)Br").unwrap();
        let rule = rr("buchwald_hartwig_retro", "[c:1][N:2]>>[c:1]Br.[N:2]");
        let outcomes = apply_retro(&target, &rule);
        let outcome_smiles: Vec<Vec<&str>> = outcomes
            .iter()
            .map(|o| o.iter().map(|p| p.smiles.as_str()).collect())
            .collect();
        assert_eq!(
            outcomes.len(),
            1,
            "expected exactly one (broken) outcome: {outcome_smiles:?}",
        );
        let precursor_smiles: Vec<&str> = outcomes[0].iter().map(|p| p.smiles.as_str()).collect();
        assert_eq!(
            precursor_smiles.len(),
            1,
            "the amine fragment must be silently dropped (split_fragments' aromaticity \
             filter rejects the leaked-ring-remainder fragment): {precursor_smiles:?}"
        );
        // The lone surviving fragment is the corrupted one: it must retain
        // the alkyl chain that leaked in from the far side of the fused
        // ring (real precursor loss) and carry two bromines (the target's
        // own plus the rule's own appended leaving-group Br) rather than
        // one -- proof this is not just a dropped fragment but a wrong one.
        let precursor_mol = mol_from_smiles(precursor_smiles[0]).unwrap();
        let br_count = precursor_mol
            .atoms()
            .filter(|(_, a)| a.element == Element::BR)
            .count();
        assert_eq!(
            br_count, 2,
            "corrupted fragment must carry two bromines, not the real product's one: {}",
            precursor_smiles[0]
        );
    }

    // v0.36.0 rule-safety census (docs/validation/rule-safety-census-2026-08-24.md):
    // same bare-single-atom-RHS shape as aryl_amine_retro/
    // buchwald_hartwig_retro, confirmed by direct apply_retro reproduction
    // on a real target with a ring N-CH2-Ar bond. Unlike buchwald_hartwig_retro
    // (which drops a fragment), the broken output here is atom-accounting
    // wrong: the "bare" [N:1] fragment carries through nearly the whole
    // molecule via the ring's other path, and the declared Br-CH2-Ar leaving
    // group ends up holding an extra piece of the ring backbone that was
    // never a real substituent of it -- total heavy atoms across both
    // precursors comes out wrong for a bond-cleavage-plus-one-new-Br.
    #[test]
    fn n_benzylation_retro_removed_from_default_rules() {
        let rules = default_rules();
        assert!(
            rules.iter().all(|r| r.name != "n_benzylation_retro"),
            "n_benzylation_retro must not be present in default_rules(): same ring-fused \
             atom-accounting defect as aryl_amine_retro/buchwald_hartwig_retro"
        );
    }

    #[test]
    fn n_benzylation_retro_would_corrupt_a_ring_fused_target_if_re_enabled() {
        let target_smiles =
            "C(CC(NC(=O)C(NC(=O)C(N)Cc4ccccc4)C)Cc3c(F)cc(c(F)c3Cl)F)(=O)N1Cc2n(c(C(F)(F)F)nn2)CC1";
        let target = mol_from_smiles(target_smiles).unwrap();
        let rule = rr(
            "n_benzylation_retro",
            "[N:1][CH2:2][c:3]>>[N:1].[Br][CH2:2][c:3]",
        );
        let outcomes = apply_retro(&target, &rule);
        let outcome_smiles: Vec<Vec<&str>> = outcomes
            .iter()
            .map(|o| o.iter().map(|p| p.smiles.as_str()).collect())
            .collect();
        assert!(
            !outcomes.is_empty(),
            "expected at least one outcome on this real ring-fused target: {outcome_smiles:?}"
        );
        // Correct chemistry for this SMIRKS is atom-conserving except for
        // one new heavy atom (the appended leaving-group Br): total heavy
        // atoms across the outcome's precursors should equal the target's
        // own heavy-atom count plus exactly one. The confirmed defect's
        // specific signature (verified 2026-08-24 against this exact
        // target: target has 45 heavy atoms, the broken outcome's two
        // precursors sum to 56 -- 10 atoms *more* than the chemically
        // correct 46) is duplication, not loss: the "bare" [N:1]
        // fragment's carry-through sweeps around the ring the *other*
        // way and re-collects atoms the declared Br-CH2-Ar fragment
        // already legitimately claims. A dropped-fragment defect (the
        // buchwald_hartwig_retro shape) would show as too *few* atoms
        // instead; asserting a strict excess pins this to the specific
        // duplication mechanism rather than any atom-count mismatch.
        let target_atoms = target.atom_count();
        let corrupted = outcomes.iter().any(|o| {
            let precursor_atoms: usize = o
                .iter()
                .map(|p| mol_from_smiles(&p.smiles).unwrap().atom_count())
                .sum();
            precursor_atoms > target_atoms + 1
        });
        assert!(
            corrupted,
            "expected at least one outcome with atom-duplicating ring carry-through (more \
             heavy atoms across precursors than the target's own count plus one new Br), got \
             only atom-conserving outcomes -- rule may have been fixed upstream, re-check \
             whether it's still correctly disabled: {outcome_smiles:?}"
        );
    }

    // Same v0.36.0 census flag and mechanism as n_benzylation_retro above,
    // confirmed by direct apply_retro reproduction on a real target with a
    // ring C-CH2-C=O bond (a glutarimide). This SMIRKS's RHS declares no
    // new atoms (pure cleavage + tautomerization), so a correct outcome's
    // precursors must have exactly the target's own heavy-atom count; the
    // ring carry-through defect breaks that conservation.
    #[test]
    fn michael_retro_removed_from_default_rules() {
        let rules = default_rules();
        assert!(
            rules.iter().all(|r| r.name != "michael_retro"),
            "michael_retro must not be present in default_rules(): same ring-fused \
             atom-accounting defect as aryl_amine_retro/buchwald_hartwig_retro"
        );
    }

    #[test]
    fn michael_retro_would_corrupt_a_ring_fused_target_if_re_enabled() {
        let target_smiles = "C(=O)N1C(=O)CC(N2C(C)c3c(c(C#N)cc(F)c3)C2=O)CC1=O";
        let target = mol_from_smiles(target_smiles).unwrap();
        let rule = rr(
            "michael_retro",
            "[C:1][CH2:2][C:3]=[O:4]>>[C:1].[CH2:2]=[C:3][OH:4]",
        );
        let outcomes = apply_retro(&target, &rule);
        let outcome_smiles: Vec<Vec<&str>> = outcomes
            .iter()
            .map(|o| o.iter().map(|p| p.smiles.as_str()).collect())
            .collect();
        assert!(
            !outcomes.is_empty(),
            "expected at least one outcome on this real ring-fused (glutarimide) target: \
             {outcome_smiles:?}"
        );
        // This SMIRKS's RHS declares no new atoms (pure cleavage +
        // tautomerization), so a correct outcome's precursors must sum to
        // exactly the target's own heavy-atom count. The confirmed
        // defect's specific signature (verified 2026-08-24 against this
        // exact target: target has 24 heavy atoms, both broken outcomes'
        // precursors sum to 30 -- 6 atoms *more* than the correct 24) is
        // duplication, same direction and same defect class as
        // n_benzylation_retro above (similar magnitude too: both ~25%
        // excess): the "bare" [C:1] fragment's carry-through sweeps
        // around the ring the other way and re-collects atoms the
        // declared enol fragment already claims. Asserting a strict
        // excess (not just any mismatch) pins this to that duplication
        // direction, though the exact BFS path wasn't traced atom-by-atom.
        let target_atoms = target.atom_count();
        let corrupted = outcomes.iter().any(|o| {
            let precursor_atoms: usize = o
                .iter()
                .map(|p| mol_from_smiles(&p.smiles).unwrap().atom_count())
                .sum();
            precursor_atoms > target_atoms
        });
        assert!(
            corrupted,
            "expected at least one outcome with atom-duplicating ring carry-through (more \
             heavy atoms across precursors than the target's own count), got only \
             atom-conserving outcomes -- rule may have been fixed upstream, re-check whether \
             it's still correctly disabled: {outcome_smiles:?}"
        );
    }

    // Guards the invariant `search::is_extracted_template` depends on: it
    // discriminates hand-crafted rules from extracted templates purely by
    // checking for an `"extracted_"` name prefix. (`RetroRule.template_id` is
    // a reliable hand-crafted/extracted discriminator too -- `rule:` vs
    // `smirks-sha256:` -- but `metadata_source`/`metadata_scope` tagging keeps
    // using the name-prefix check unchanged, matching pre-template_id behavior.)
    // If a hand-crafted rule ever used the `extracted_` prefix, it would be
    // silently mis-tagged as having no metadata provenance.
    #[test]
    fn default_rule_names_never_use_extracted_prefix() {
        let rules = default_rules();
        for rule in &rules {
            assert!(
                !rule.name.starts_with("extracted_"),
                "hand-crafted rule {:?} must not use the extracted_ name prefix \
                 reserved for load_rules_from_file",
                rule.name
            );
        }
    }

    fn write_templates_file(dir: &std::path::Path, name: &str, content: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn template_id_stable_across_file_reordering() {
        let dir = std::env::temp_dir();
        let a = "[O:3]=[C:2]-[OH:1]>>C-[O:1]-[C:2]=[O:3]";
        let b = "[NH2:1]-[c:2]>>O=[N+:1](-[O-])-[c:2]";
        let path1 = write_templates_file(
            &dir,
            "renkin_tid_order1.smi",
            &format!("{a}\t10\n{b}\t20\n"),
        );
        let path2 = write_templates_file(
            &dir,
            "renkin_tid_order2.smi",
            &format!("{b}\t20\n{a}\t10\n"),
        );
        let rules1 = load_rules_from_file(&path1);
        let rules2 = load_rules_from_file(&path2);
        let id_a_1 = rules1
            .iter()
            .find(|r| r.smirks == a)
            .unwrap()
            .template_id
            .clone();
        let id_a_2 = rules2
            .iter()
            .find(|r| r.smirks == a)
            .unwrap()
            .template_id
            .clone();
        assert_eq!(id_a_1, id_a_2, "template_id must not depend on line order");
        std::fs::remove_file(&path1).ok();
        std::fs::remove_file(&path2).ok();
    }

    #[test]
    fn template_id_stable_when_count_changes() {
        let dir = std::env::temp_dir();
        let smirks = "[O:3]=[C:2]-[OH:1]>>C-[O:1]-[C:2]=[O:3]";
        let path1 = write_templates_file(&dir, "renkin_tid_count1.smi", &format!("{smirks}\t1\n"));
        let path2 = write_templates_file(
            &dir,
            "renkin_tid_count2.smi",
            &format!("{smirks}\t999999\n"),
        );
        let id1 = load_rules_from_file(&path1)[0].template_id.clone();
        let id2 = load_rules_from_file(&path2)[0].template_id.clone();
        assert_eq!(id1, id2, "template_id must not depend on count");
        std::fs::remove_file(&path1).ok();
        std::fs::remove_file(&path2).ok();
    }

    #[test]
    fn different_smirks_give_different_template_id() {
        let dir = std::env::temp_dir();
        let path = write_templates_file(
            &dir,
            "renkin_tid_distinct.smi",
            "[O:3]=[C:2]-[OH:1]>>C-[O:1]-[C:2]=[O:3]\t1\n[NH2:1]-[c:2]>>O=[N+:1](-[O-])-[c:2]\t1\n",
        );
        let rules = load_rules_from_file(&path);
        assert_eq!(rules.len(), 2);
        assert_ne!(
            rules[0].template_id, rules[1].template_id,
            "different SMIRKS must produce different template_id"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn extracted_template_id_uses_smirks_sha256_prefix() {
        let dir = std::env::temp_dir();
        let path = write_templates_file(
            &dir,
            "renkin_tid_prefix.smi",
            "[O:3]=[C:2]-[OH:1]>>C-[O:1]-[C:2]=[O:3]\t1\n",
        );
        let rules = load_rules_from_file(&path);
        assert!(rules[0].template_id.starts_with("smirks-sha256:"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn hand_crafted_rule_template_id_is_stable_rule_prefix() {
        let rules = default_rules();
        for rule in &rules {
            assert_eq!(
                rule.template_id,
                format!("rule:{}", rule.name),
                "hand-crafted rule {:?} must have template_id `rule:<name>`",
                rule.name
            );
        }
        // Stable across repeated calls (no hidden nondeterminism, e.g. hashing order).
        let rules_again = default_rules();
        for (r1, r2) in rules.iter().zip(rules_again.iter()) {
            assert_eq!(r1.template_id, r2.template_id);
        }
    }

    #[test]
    fn default_rules_never_reduce_halobenzene_to_bare_benzene() {
        // Cl/I/F atoms present in a target must never be silently dropped
        // without a tracked reagent: no rule in default_rules() may turn
        // chlorobenzene/iodobenzene/fluorobenzene into a single-fragment
        // "benzene" precursor set — that's exactly the atom-loss bug these
        // three rules had (retro-applying them "explained" Ar-X as coming
        // from Ar-H with the halogen vanishing into nothing).
        let benzene_smi = canonical_smiles(&mol_from_smiles("c1ccccc1").unwrap());
        let rules = default_rules();
        for (name, smi) in [
            ("chlorobenzene", "Clc1ccccc1"),
            ("iodobenzene", "Ic1ccccc1"),
            ("fluorobenzene", "Fc1ccccc1"),
        ] {
            let mol = mol_from_smiles(smi).unwrap();
            for rule in &rules {
                for set in apply_retro(&mol, rule) {
                    let is_bare_benzene = set.len() == 1
                        && canonical_smiles(&mol_from_smiles(&set[0].smiles).unwrap())
                            == benzene_smi;
                    assert!(
                        !is_bare_benzene,
                        "{name}: rule '{}' must not reduce it to bare benzene with no tracked halogen precursor",
                        rule.name
                    );
                }
            }
        }
    }

    #[test]
    fn aryl_chloride_to_bromide_unaffected_by_halide_rule_removal() {
        // aryl_chloride_to_bromide is a different, atom-preserving rule
        // (halogen-for-halogen swap) and must keep firing exactly as before.
        let rules = default_rules();
        let rule = rules
            .iter()
            .find(|r| r.name == "aryl_chloride_to_bromide")
            .expect("aryl_chloride_to_bromide must still be in default_rules()");
        let mol = mol_from_smiles("Clc1ccccc1").unwrap();
        let results = apply_retro(&mol, rule);
        assert!(
            !results.is_empty(),
            "aryl_chloride_to_bromide must still fire on chlorobenzene"
        );
        let bromobenzene_smi = canonical_smiles(&mol_from_smiles("Brc1ccccc1").unwrap());
        let flat: Vec<_> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.as_str()))
            .collect();
        assert!(
            flat.iter().any(|s| *s == bromobenzene_smi),
            "products must include bromobenzene; got {flat:?}"
        );
    }

    #[test]
    fn amide_cleavage_graph_gives_clean_two_fragments() {
        // Graph-based amide_cleavage must not produce BFS-leaked extra fragments.
        // Acetanilide: CC(=O)Nc1ccccc1 → acetic acid + aniline (exactly 2 fragments).
        let mol = mol_from_smiles("CC(=O)Nc1ccccc1").unwrap();
        let rule = rr("amide_cleavage", "");
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "amide_cleavage must fire on acetanilide"
        );
        // Every candidate precursor set must contain exactly 2 fragments.
        for set in &results {
            assert_eq!(
                set.len(),
                2,
                "amide cleavage must yield exactly 2 fragments (no BFS leakage); got {:?}",
                set.iter().map(|p| p.smiles.as_str()).collect::<Vec<_>>()
            );
        }
        let acetic = canonical_smiles(&mol_from_smiles("CC(=O)O").unwrap());
        let aniline = canonical_smiles(&mol_from_smiles("Nc1ccccc1").unwrap());
        let flat: Vec<_> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.clone()))
            .collect();
        assert!(
            flat.contains(&acetic),
            "must include acetic acid; got {flat:?}"
        );
        assert!(
            flat.contains(&aniline),
            "must include aniline; got {flat:?}"
        );
    }

    #[test]
    fn reductive_amination_retro_on_benzylamine() {
        let mol = mol_from_smiles("NCc1ccccc1").unwrap();
        let rule = rr("reductive_amination_retro", "[C:1][N:2]>>[C:1]=O.[N:2]");
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "reductive_amination_retro must fire on benzylamine"
        );
        let flat: Vec<_> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.as_str()))
            .collect();
        assert!(
            flat.iter().any(|s| s.contains("=O") || s.contains("O=")),
            "products must include aldehyde/ketone; got {flat:?}"
        );
    }
}

#[test]
fn canonical_smiles_is_deterministic() {
    // Regression test for chematic Bug #14 (fixed in 0.4.12):
    // canonical_smiles() must return the same string for the same molecule
    // regardless of how the SMILES was written.
    // Note: aromatic vs Kekulé (c1ccccc1 vs C1=CC=CC=C1) are treated as
    // different representations by chematic and intentionally excluded here.
    let pairs = [
        ("Nc1ccccc1", "c1ccc(N)cc1", "aniline"),
        ("Oc1ccccc1", "c1ccc(O)cc1", "phenol"),
        ("Brc1ccccc1", "c1ccc(Br)cc1", "bromobenzene"),
        ("CC(=O)O", "OC(C)=O", "acetic acid"),
    ];
    for (s1, s2, name) in pairs {
        let c1 = canonical_smiles(&parse(s1).unwrap());
        let c2 = canonical_smiles(&parse(s2).unwrap());
        assert_eq!(
            c1, c2,
            "{name}: '{s1}' and '{s2}' should have the same canonical SMILES"
        );
    }
}

#[cfg(test)]
mod bug13_regression {
    use super::*;

    /// Regression test for chematic Bug #13 (fixed in 0.4.12):
    /// run_reactants must not leak BFS across product templates.
    /// Amide cleavage of acetanilide must give exactly 2 clean products.
    #[test]
    fn smirks_amide_cleavage_no_bfs_leakage() {
        let mol = parse("CC(=O)Nc1ccccc1").unwrap();
        let smirks = "[C:1](=[O:2])[N:3]>>[C:1](=[O:2])O.[N:3]";
        let results = run_reactants(smirks, &[&mol]).unwrap_or_default();
        assert!(!results.is_empty(), "expected at least one result set");
        for group in &results {
            assert_eq!(
                group.len(),
                2,
                "expected exactly 2 products, got {}: {:?}",
                group.len(),
                group.iter().map(canonical_smiles).collect::<Vec<_>>()
            );
        }
    }
}

#[cfg(test)]
mod chematic_regression {
    use super::*;

    /// Frozen fixture, `extracted_4255` on `C2c1cc(c(F)cc1C(O2)=O)F` (a
    /// difluoro-substituted isobenzofuran-1(3H)-one / phthalide, benzo-fused
    /// gamma-butyrolactone) <- `O=C.C(=O)O.c1(F)ccccc1F` (formaldehyde +
    /// formic acid + 1,2-difluorobenzene, three independent fragments): a
    /// third individually-investigated step from Finding #4's pilot
    /// (`docs/validation/finding4-validator-pilot-2026-08-23.md`), per that
    /// doc's own protocol -- not by re-running search.
    ///
    /// Classified as **`genuine_template_error`**, the same failure class as
    /// `extracted_112_indanone_is_genuine_template_error` above: a
    /// two-or-more-fragment SMIRKS retro-template that models what is
    /// actually an intramolecular ring bond as an intermolecular one.
    ///
    /// `extracted_4255`'s SMIRKS
    /// (`[C:2]-[O:3]-[CH2:1]-[c:5](:[c:4]):[c:6]>>O=[CH2:1].[C:2]-[OH:3].[c:4]:[cH:5]:[c:6]`)
    /// breaks the phthalide's two ring-closing bonds (benzylic CH2-O and
    /// benzylic CH2-aromatic) *and additionally* severs the carbonyl
    /// carbon's own bond to its aromatic ring neighbor -- a bond the LHS
    /// pattern never even examines (that aromatic atom isn't matched at
    /// all), so the template has no way to know it's discarding a real,
    /// present bond rather than a genuinely absent one. The result is 3
    /// mutually independent fragments instead of the 2 that a correct retro
    /// of this bicyclic lactone would produce (an open aryl fragment still
    /// bearing both substituents, plus formaldehyde) -- let alone
    /// recognizing this disconnection is intramolecular to begin with.
    ///
    /// Both directions confirm this empirically:
    /// - `apply_retro` on the real target reproduces the exact same
    ///   3-fragment disconnection fresh -- `["O=C", "C(=O)O",
    ///   "c1(F)ccccc1F"]`, the fixture's own precursors, matching the
    ///   pilot harness's raw output exactly.
    /// - Forward-replaying those 3 precursors through the reversed SMIRKS
    ///   can only ever form ONE new bond (benzylic CH2 to one aromatic
    ///   ring), leaving the carbonyl fragment dangling as an open ester
    ///   chain (`Ar-CH2-O-C(=O)H`) rather than closing the second bond
    ///   back onto the SAME ring to re-form the fused 5-membered lactone --
    ///   structurally impossible from 3 independent molecules via this
    ///   rule. Neither of the 2 resulting open-chain regiochemical products
    ///   is or can be the bicyclic target.
    ///
    /// Same likely-general-failure-class note as `extracted_112`: any
    /// multi-fragment SMIRKS whose RHS declares a mapped atom's spectator
    /// bond (one never examined by the LHS pattern) as absent will make
    /// this same mistake whenever the target happens to have that
    /// substituent tethered elsewhere. Not investigated further here.
    #[test]
    fn extracted_4255_difluorophthalide_is_genuine_template_error() {
        let target = "C2c1cc(c(F)cc1C(O2)=O)F";
        let precursors = [
            "O=C".to_string(),
            "C(=O)O".to_string(),
            "c1(F)ccccc1F".to_string(),
        ];
        let smirks =
            "[C:2]-[O:3]-[CH2:1]-[c:5](:[c:4]):[c:6]>>O=[CH2:1].[C:2]-[OH:3].[c:4]:[cH:5]:[c:6]";
        let rule = rr("extracted_4255", smirks);
        let target_mol = mol_from_smiles(target).unwrap();
        let target_canon = to_canonical(&target_mol);

        // apply_retro reproduces the exact 3-fragment disconnection fresh,
        // from the real target -- confirms this isn't a harness-only
        // artifact.
        let retro_outcomes = apply_retro(&target_mol, &rule);
        let retro_smiles: Vec<Vec<String>> = retro_outcomes
            .iter()
            .map(|outcome| outcome.iter().map(|p| to_canonical(&p.mol)).collect())
            .collect();
        let expected_precursors: Vec<String> = precursors
            .iter()
            .map(|s| to_canonical(&mol_from_smiles(s).unwrap()))
            .collect();
        assert!(
            retro_smiles.iter().any(|outcome| outcome
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                == expected_precursors.iter().collect()),
            "apply_retro must still reproduce this exact 3-fragment disconnection: {retro_smiles:?}"
        );

        // Exhaustive forward replay of the declared precursors: every
        // possible product is a plain open-chain formate-ester regioisomer,
        // never the fused bicyclic target.
        let reactant_mols: Vec<Molecule> = precursors
            .iter()
            .map(|s| mol_from_smiles(s).unwrap())
            .collect();
        let reactant_refs: Vec<&Molecule> = reactant_mols.iter().collect();
        let (lhs, rhs) = smirks.split_once(">>").unwrap();
        let fwd = format!("{rhs}>>{lhs}");
        let products: std::collections::BTreeSet<String> = run_reactants(&fwd, &reactant_refs)
            .unwrap()
            .into_iter()
            .flatten()
            .map(|m| to_canonical(&m))
            .collect();
        // Product count (currently 2 open-chain formate-ester regioisomers)
        // is supplementary evidence, not the classification's essential
        // claim -- a future chematic regiochemistry-enumeration/dedup
        // change could shift the count without changing the underlying
        // chemical conclusion. The essential claims are: at least one
        // product exists, and none of them is the target. Canonical-SMILES
        // inequality is a sufficient stand-in for "never reconstructs the
        // fused ring" here since this target has no stereocenters -- a
        // constitutional match would necessarily canonicalize identically.
        assert!(
            !products.is_empty(),
            "forward replay of the declared precursors must produce at least one candidate \
             product: {products:?}"
        );
        assert!(
            !products.contains(&target_canon),
            "formaldehyde + formic acid + 1,2-difluorobenzene as three separate molecules must \
             never be able to forward-produce the fused bicyclic target -- if this starts \
             passing, the underlying chematic/apply_retro fragment-tether-tracking behavior \
             changed and this classification needs re-checking, not just the assertion updated: \
             {products:?}"
        );
    }

    /// Regression test for chematic issue #19 (fixed in 0.4.14):
    /// parse_smarts must accept atom-map notation (:N).
    #[test]
    fn parse_smarts_accepts_atom_maps() {
        assert!(parse_smarts("[C:1](=[O:2])[N:3]").is_ok());
        assert!(parse_smarts("[NH2:1]-[c:2]").is_ok());
        assert!(parse_smarts("[O:1]=[C:2]").is_ok());
        // Phase 15: @/@@ stereo + atom-map (chematic #20 fixed in 0.4.13)
        assert!(parse_smarts("[C@:1]").is_ok(), "@ + atom-map must parse");
        assert!(
            parse_smarts("[C@@H:2]").is_ok(),
            "@@ + H + atom-map must parse"
        );
        assert!(
            parse_smarts("[C@H:1]-[c:2]").is_ok(),
            "stereo SMIRKS reactant must parse"
        );
    }

    /// Phase 15 regression: tetrahedral @/@@ in run_reactants (chematic #20, fixed in v0.4.13).
    /// A stereo-specific SMIRKS must only match the correct enantiomer.
    #[test]
    fn tetrahedral_stereo_filter_rejects_wrong_enantiomer() {
        // Retro-oxidation: chiral alcohol → ketone.
        // [C:1]-[C@H:2](-[OH:3])-[c:4] should match only the R-enantiomer.
        let smirks = "[C:1]-[C@H:2](-[OH:3])-[c:4]>>[C:1]-[C:2](=[O:3])-[c:4]";
        let r_alcohol = parse("C[C@H](O)c1ccccc1").unwrap(); // (R) — should match
        let s_alcohol = parse("C[C@@H](O)c1ccccc1").unwrap(); // (S) — must NOT match

        let r_results = run_reactants(smirks, &[&r_alcohol]).unwrap_or_default();
        let s_results = run_reactants(smirks, &[&s_alcohol]).unwrap_or_default();

        assert!(
            !r_results.is_empty(),
            "R-alcohol must match @-SMIRKS (chematic #20 regression)"
        );
        assert!(
            s_results.is_empty(),
            "S-alcohol must NOT match @-SMIRKS (chematic #20 regression); got {} result(s)",
            s_results.len()
        );
    }

    /// Regression test for chematic issue #18 (fixed in 0.4.14):
    /// run_reactants products must not have unnecessary bracket atoms.
    #[test]
    fn run_reactants_products_no_bracket_atoms() {
        let mol = parse("CC(=O)Nc1ccccc1").unwrap();
        let smirks = "[C:1](=[O:2])[N:3]>>[C:1](=[O:2])O.[N:3]";
        let results = run_reactants(smirks, &[&mol]).unwrap_or_default();
        assert!(!results.is_empty());
        for group in &results {
            for product in group {
                let canon = canonical_smiles(product);
                assert!(
                    !canon.starts_with('['),
                    "product has unexpected bracket atom: {canon}"
                );
            }
        }
    }

    /// Regression test for chematic issue #21 (fixed in 0.4.15):
    /// run_reactants must filter reactants by E/Z geometry when SMIRKS specifies /\.
    /// Using the retro-Wittig example from the issue: Z-specific SMIRKS must not match E-alkene.
    #[test]
    fn ez_stereo_filter_rejects_wrong_geometry() {
        // Z-selective SMIRKS: [C:1]/[C:2]=[C:3]\[C:4] matches only Z-alkenes
        let smirks = "[C:1]/[C:2]=[C:3]\\[C:4]>>[C:1][C:2]=O.[O:3]=[C:4]";
        let z_hexene = parse("CC/C=C\\CC").unwrap(); // (Z)-3-hexene — should match
        let e_hexene = parse("CC/C=C/CC").unwrap(); // (E)-3-hexene — must NOT match

        let z_results = run_reactants(smirks, &[&z_hexene]).unwrap_or_default();
        let e_results = run_reactants(smirks, &[&e_hexene]).unwrap_or_default();

        assert!(
            !z_results.is_empty(),
            "Z-alkene must match Z-SMIRKS (chematic #21 regression)"
        );
        assert!(
            e_results.is_empty(),
            "E-alkene must NOT match Z-SMIRKS (chematic #21 regression); got {} result set(s)",
            e_results.len()
        );
    }

    /// diaryl_sulfone_retro: diphenyl sulfone → benzenesulfonyl chloride + benzene.
    #[test]
    fn diaryl_sulfone_retro_diphenyl_sulfone() {
        let mol = mol_from_smiles("O=S(=O)(c1ccccc1)c1ccccc1").unwrap(); // diphenyl sulfone
        let rule = rr("diaryl_sulfone_retro", "");
        let results = apply_retro(&mol, &rule);

        assert!(
            !results.is_empty(),
            "diaryl_sulfone_retro must fire on diphenyl sulfone"
        );
        // Must produce benzenesulfonyl chloride (PhSO2Cl) and benzene (PhH)
        let flat: Vec<_> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.as_str()))
            .collect();
        // canonical SMILES for PhSO2Cl is "O=S(c1ccccc1)(Cl)=O"
        let has_so2cl = flat.iter().any(|s| s.contains("Cl") && s.contains('S'));
        assert!(has_so2cl, "must produce ArSO2Cl; got {flat:?}");
        let has_benzene = flat.contains(&"c1ccccc1");
        assert!(has_benzene, "must produce benzene; got {flat:?}");
    }

    /// diaryl_sulfone_retro: asymmetric sulfone gives two distinct disconnections.
    #[test]
    fn diaryl_sulfone_retro_asymmetric() {
        // 4-methylphenyl phenyl sulfone
        let mol = mol_from_smiles("O=S(=O)(c1ccc(C)cc1)c1ccccc1").unwrap();
        let rule = rr("diaryl_sulfone_retro", "");
        let results = apply_retro(&mol, &rule);

        assert!(
            results.len() >= 2,
            "asymmetric diaryl sulfone must give ≥2 disconnections; got {}",
            results.len()
        );
    }

    /// diaryl_sulfone_retro must NOT fire on a simple thioether (no =O on S).
    #[test]
    fn diaryl_sulfone_retro_no_fire_on_thioether() {
        let mol = mol_from_smiles("c1ccccc1Sc1ccccc1").unwrap(); // diphenyl thioether
        let rule = rr("diaryl_sulfone_retro", "");
        let results = apply_retro(&mol, &rule);
        assert!(
            results.is_empty(),
            "diaryl_sulfone_retro must NOT fire on thioether; got {} result set(s)",
            results.len()
        );
    }

    /// Symmetric counterpart: E-selective SMIRKS must match E-alkene and reject Z-alkene.
    #[test]
    fn ez_stereo_e_selective_smirks() {
        // E-selective SMIRKS: [C:1]/[C:2]=[C:3]/[C:4] matches only E-alkenes
        let smirks = "[C:1]/[C:2]=[C:3]/[C:4]>>[C:1][C:2]=O.[O:3]=[C:4]";
        let e_hexene = parse("CC/C=C/CC").unwrap(); // (E)-3-hexene — should match
        let z_hexene = parse("CC/C=C\\CC").unwrap(); // (Z)-3-hexene — must NOT match

        let e_results = run_reactants(smirks, &[&e_hexene]).unwrap_or_default();
        let z_results = run_reactants(smirks, &[&z_hexene]).unwrap_or_default();

        assert!(!e_results.is_empty(), "E-alkene must match E-SMIRKS");
        assert!(
            z_results.is_empty(),
            "Z-alkene must NOT match E-SMIRKS; got {} result set(s)",
            z_results.len()
        );
    }

    /// Stereo-unspecified SMIRKS must match both E- and Z-alkenes.
    #[test]
    fn ez_stereo_unspecified_smirks_matches_both_geometries() {
        // No /\ in SMIRKS → geometry-agnostic
        let smirks = "[C:1][C:2]=[C:3][C:4]>>[C:1][C:2]=O.[O:3]=[C:4]";
        let e_hexene = parse("CC/C=C/CC").unwrap();
        let z_hexene = parse("CC/C=C\\CC").unwrap();

        let e_results = run_reactants(smirks, &[&e_hexene]).unwrap_or_default();
        let z_results = run_reactants(smirks, &[&z_hexene]).unwrap_or_default();

        assert!(
            !e_results.is_empty(),
            "non-stereo SMIRKS must match E-alkene"
        );
        assert!(
            !z_results.is_empty(),
            "non-stereo SMIRKS must match Z-alkene"
        );
    }

    /// Real-world example: retro-Wittig on (E)-stilbene vs (Z)-stilbene.
    /// E-selective SMIRKS (Ph/C=C/Ph pattern) must discriminate between isomers.
    #[test]
    fn ez_stereo_stilbene_wittig_discrimination() {
        // E-selective retro-Wittig: splits E-stilbene into two benzaldehyde equivalents
        let smirks = "[c:1]/[C:2]=[C:3]/[c:4]>>[c:1][C:2]=O.[O:3]=[C:4][c:4]";
        let e_stilbene = parse("c1ccccc1/C=C/c1ccccc1").unwrap(); // (E)-stilbene
        let z_stilbene = parse("c1ccccc1/C=C\\c1ccccc1").unwrap(); // (Z)-stilbene

        let e_results = run_reactants(smirks, &[&e_stilbene]).unwrap_or_default();
        let z_results = run_reactants(smirks, &[&z_stilbene]).unwrap_or_default();

        assert!(
            !e_results.is_empty(),
            "E-selective SMIRKS must fire on (E)-stilbene"
        );
        assert!(
            z_results.is_empty(),
            "E-selective SMIRKS must NOT fire on (Z)-stilbene; got {} result set(s)",
            z_results.len()
        );
    }

    /// Frozen fixture, `extracted_109` on `C1CCCC[C@H](N1)C` <-
    /// `C(C)(CCCC)=O.C(CCN)C` (a methyl-butyl ketone + n-butylamine): a
    /// third individually-investigated step from Finding #4's pilot
    /// (`docs/validation/finding4-validator-pilot-2026-08-23.md`), per
    /// that doc's own protocol -- not by re-running search.
    ///
    /// Note: the pilot's preliminary visual note called the target
    /// "2-methylpiperidine". That was wrong -- `target_mol.atom_count()`
    /// is 8 (6 ring carbons + 1 ring nitrogen + 1 exocyclic methyl),
    /// confirming a 7-membered ring: this is **2-methylazepane**
    /// (hexahydro-2-methyl-1H-azepine), not the 6-membered piperidine.
    /// Doesn't change the classification below, but corrects the record.
    ///
    /// Classified as **`genuine_template_error`**, and a second concrete
    /// instance of the same failure class documented on `extracted_112`
    /// (`extracted_112_indanone_is_genuine_template_error` above): a
    /// two-fragment retro-SMIRKS (RHS = `ketone . amine`, `.`-separated)
    /// applied to a target where the two "fragments" are not actually
    /// separate molecules -- they're tethered to each other via the rest
    /// of a ring the LHS pattern never examines.
    ///
    /// `extracted_109`'s SMIRKS
    /// (`[C:2]-[CH:1](-[NH:5]-[C:4])-[C:3]>>O=[C:1](-[C:2])-[C:3].[C:4]-[NH2:5]`)
    /// is a standard retro-reductive-amination template, correct in
    /// general for genuinely intermolecular cases (ketone + amine ->
    /// secondary amine). In the target, `[CH:1]` and `[NH:5]` are two
    /// adjacent atoms of the SAME 7-membered ring; `[C:4]` (`NH:5`'s other
    /// neighbor) is reachable from `[CH:1]`'s own other ring substituent
    /// via the rest of that same ring. Cutting only the matched
    /// `[CH:1]`-`[NH:5]` bond therefore cannot split the molecule in two
    /// -- topologically it can only open the ring into a single
    /// amino-ketone chain. The real retro-relationship here is an
    /// intramolecular ring-closing reductive amination (one precursor
    /// molecule, not two separate building blocks).
    ///
    /// Confirmed empirically, both directions:
    /// - `apply_retro` on the real target reproduces the exact same wrong
    ///   two-fragment split fresh (not a harness-only artifact) -- and the
    ///   two fragments' combined heavy-atom count (12: 7 + 5) *exceeds*
    ///   the target's own heavy-atom count (8), which is impossible for a
    ///   real bond-breaking disconnection (retro of this reaction class
    ///   should conserve every target atom and add exactly one new
    ///   oxygen). That numeric inflation is itself direct evidence the
    ///   declared fragmentation isn't a real graph cut of this target.
    /// - Forward-replaying the declared precursors through the reversed
    ///   SMIRKS produces exactly one product, a plain open-chain secondary
    ///   amine -- not a ring, and not the target under any of canonical /
    ///   connectivity-only (stereo-stripped) comparison.
    ///
    /// The validator's `Invalid` verdict is correct, for the right reason.
    /// Atom-balance still reports `true` in the pilot's own harness output
    /// because `atom_conservation` checks gross formula/MW, not graph
    /// connectivity -- expected, not a separate bug, same as
    /// `extracted_112`.
    #[test]
    fn extracted_109_azepane_is_genuine_template_error() {
        let target = "C1CCCC[C@H](N1)C";
        let precursors = ["C(C)(CCCC)=O".to_string(), "C(CCN)C".to_string()];
        let smirks = "[C:2]-[CH:1](-[NH:5]-[C:4])-[C:3]>>O=[C:1](-[C:2])-[C:3].[C:4]-[NH2:5]";
        let rule = rr("extracted_109", smirks);
        let target_mol = mol_from_smiles(target).unwrap();
        let target_canon = to_canonical(&target_mol);

        assert_eq!(
            target_mol.atom_count(),
            8,
            "target must be the 7-membered-ring 2-methylazepane (8 heavy atoms), \
             not the 6-membered piperidine the pilot's preliminary note assumed"
        );

        // apply_retro reproduces the exact wrong disconnection fresh, from
        // the real target -- confirms this isn't a harness-only artifact.
        let retro_outcomes = apply_retro(&target_mol, &rule);
        let retro_smiles: Vec<Vec<String>> = retro_outcomes
            .iter()
            .map(|outcome| outcome.iter().map(|p| to_canonical(&p.mol)).collect())
            .collect();
        let expected_precursors: Vec<String> = precursors
            .iter()
            .map(|s| to_canonical(&mol_from_smiles(s).unwrap()))
            .collect();
        assert!(
            retro_smiles.iter().any(|outcome| outcome
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                == expected_precursors.iter().collect()),
            "apply_retro must still reproduce this exact wrong disconnection: {retro_smiles:?}"
        );

        // The declared precursors' combined atom count must exceed the
        // target's own -- direct evidence this "split" is not a real cut
        // of this target's graph (impossible for a genuine disconnection).
        let precursor_atom_total: usize = precursors
            .iter()
            .map(|s| mol_from_smiles(s).unwrap().atom_count())
            .sum();
        assert!(
            precursor_atom_total > target_mol.atom_count(),
            "declared precursors ({precursor_atom_total} atoms) were expected to exceed the \
             target's own atom count ({}) -- if this stops holding, the underlying apply_retro \
             ring-splitting behavior changed and this classification needs re-checking",
            target_mol.atom_count()
        );

        // Forward replay of the declared precursors: only one product
        // forms, a plain open-chain secondary amine -- not the target,
        // under canonical or connectivity-only (stereo-stripped) identity.
        let reactant_mols: Vec<Molecule> = precursors
            .iter()
            .map(|s| mol_from_smiles(s).unwrap())
            .collect();
        let reactant_refs: Vec<&Molecule> = reactant_mols.iter().collect();
        let (lhs, rhs) = smirks.split_once(">>").unwrap();
        let fwd = format!("{rhs}>>{lhs}");
        let products: std::collections::BTreeSet<String> = run_reactants(&fwd, &reactant_refs)
            .unwrap()
            .into_iter()
            .flatten()
            .map(|m| to_canonical(&m))
            .collect();
        // Product count (currently 1) is supplementary evidence, not the
        // classification's essential claim -- demoted from a hard
        // assertion so a future chematic enumeration change can't
        // spuriously break this fixture without changing the chemical
        // conclusion. The essential claims are: at least one product
        // exists, and none of them is the target (checked below, plus the
        // stereo-stripped connectivity check).
        assert!(
            !products.is_empty(),
            "forward replay of the declared precursors must produce at least one candidate \
             product: {products:?}"
        );
        assert!(
            !products.contains(&target_canon),
            "two separate molecules must never forward-produce the ring target: {products:?}"
        );
        let strip_stereo = |s: &str| s.replace('@', "");
        let target_no_stereo = strip_stereo(&target_canon);
        assert!(
            !products.iter().any(|p| strip_stereo(p) == target_no_stereo),
            "products must not even match the target's connectivity ignoring stereo -- this is \
             a connectivity error, not a stereo-only gap like co_aliphatic_cleavage: {products:?}"
        );
    }

    // ── declared_forward_smirks: forward-replay evidence for graph-based rules ──
    //
    // Closes the gap in docs/design/retro-rule-precision-gaps-v0.md #5:
    // graph-based rules (empty RetroRule::smirks) have no string for
    // bridge::forward to reverse-apply, so routes using them could never
    // reach forward_validation: pass via self-audit. See that function's
    // own doc comment for the full mechanism.

    /// Frozen fixture, `extracted_112` on `c2ccc1CCC(c1c2)=O` (1-indanone)
    /// <- `C(C(=O)Cl)C.c1ccc(C)cc1` (propionyl chloride + toluene): a
    /// second individually-investigated step from Finding #4's pilot
    /// (`docs/validation/finding4-validator-pilot-2026-08-23.md`), per
    /// that doc's own protocol -- not by re-running search.
    ///
    /// Classified as **`genuine_template_error`**, more serious than the
    /// `co_aliphatic_cleavage` case
    /// (`co_aliphatic_cleavage_piperidinyl_carbamate_is_source_step_underspecified`
    /// in `validation::forward`'s tests): there the *connectivity* was
    /// right and only stereochemistry was unverifiable. Here the
    /// connectivity itself is wrong.
    ///
    /// `extracted_112`'s own SMIRKS
    /// (`[C:2]-[C:1](=[O:3])-[c:5](:[c:4]):[c:6]>>Cl-[C:1](-[C:2])=[O:3].[c:4]:[cH:5]:[c:6]`)
    /// is a completely standard, real retro-Friedel-Crafts-acylation
    /// template (Ar-C(=O)R -> ArH + Cl-C(=O)R), correct as a general
    /// *intermolecular* transform. 1-indanone's ketone carbon and its
    /// aromatic ring are not just "nearby" -- the ring is the SAME ring
    /// the ketone's own alkyl tail (via the fused 5-membered ring) loops
    /// back onto, making the real disconnection intramolecular (the real
    /// synthesis: 3-arylpropionic acid/chloride cyclizes onto its own
    /// tethered phenyl ring). The template's LHS only examines the
    /// immediate reaction-center neighborhood (the acyl carbon, its two
    /// substituents, and the three explicitly-matched ring atoms) and has
    /// no way to notice that the "two separate RHS fragments" it declares
    /// are, in THIS specific target, still connected to each other via
    /// the tether atoms outside that match.
    ///
    /// Both directions confirm this empirically, not just structurally:
    /// - `apply_retro` on the real target reproduces the exact same wrong
    ///   disconnection fresh (not a one-off harness artifact) --
    ///   `["C(C(=O)Cl)C", "c1ccc(C)cc1"]`, the fixture's own precursors.
    /// - Forward-replaying those precursors through the reversed SMIRKS
    ///   is exhaustive (chematic tries the acylation at every one of
    ///   toluene's aromatic C-H positions) and produces exactly 3 distinct
    ///   products -- the ortho/meta/para propiophenone regioisomers --
    ///   **none of which is or can be 1-indanone**: toluene and propionyl
    ///   chloride as two genuinely separate, untethered molecules can only
    ///   ever combine into an open-chain aryl ketone, never a fused
    ///   bicyclic ring system, regardless of which ring position reacts.
    ///
    /// The validator's `Invalid` verdict is correct, and correct for the
    /// right reason: this declared step does not describe a real
    /// reaction. Atom-balance still reports `true` for this step (already
    /// confirmed by the pilot's own harness output) because
    /// `atom_conservation`'s check is gross-formula/MW-based, not
    /// connectivity-aware -- expected, not a separate bug; exactly why the
    /// SMIRKS-reversal forward validator exists as a distinct check.
    ///
    /// Likely a general failure-class, not unique to this one template:
    /// any two-fragment SMIRKS retro-template whose RHS declares two
    /// independent products can silently produce this same
    /// intramolecular-modeled-as-intermolecular error whenever the target
    /// happens to have those two "fragments" tethered together elsewhere
    /// (any other ring-forming disconnection matched by a template learned
    /// from intermolecular training examples). Not investigated further
    /// here -- filing as a general issue is a separate, explicitly
    /// authorized step, not implied by this fixture.
    #[test]
    fn extracted_112_indanone_is_genuine_template_error() {
        let target = "c2ccc1CCC(c1c2)=O";
        let precursors = ["C(C(=O)Cl)C".to_string(), "c1ccc(C)cc1".to_string()];
        let smirks =
            "[C:2]-[C:1](=[O:3])-[c:5](:[c:4]):[c:6]>>Cl-[C:1](-[C:2])=[O:3].[c:4]:[cH:5]:[c:6]";
        let rule = rr("extracted_112", smirks);
        let target_mol = mol_from_smiles(target).unwrap();
        let target_canon = to_canonical(&target_mol);

        // apply_retro reproduces the exact wrong disconnection fresh, from
        // the real target -- confirms this isn't a harness-only artifact.
        let retro_outcomes = apply_retro(&target_mol, &rule);
        let retro_smiles: Vec<Vec<String>> = retro_outcomes
            .iter()
            .map(|outcome| outcome.iter().map(|p| to_canonical(&p.mol)).collect())
            .collect();
        let expected_precursors: Vec<String> = precursors
            .iter()
            .map(|s| to_canonical(&mol_from_smiles(s).unwrap()))
            .collect();
        assert!(
            retro_smiles.iter().any(|outcome| outcome
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                == expected_precursors.iter().collect()),
            "apply_retro must still reproduce this exact wrong disconnection: {retro_smiles:?}"
        );

        // Exhaustive forward replay of the declared precursors: every
        // possible product is a plain open-chain propiophenone
        // regioisomer, none of which is (or topologically can be) the
        // fused-ring target.
        let reactant_mols: Vec<Molecule> = precursors
            .iter()
            .map(|s| mol_from_smiles(s).unwrap())
            .collect();
        let reactant_refs: Vec<&Molecule> = reactant_mols.iter().collect();
        let (lhs, rhs) = smirks.split_once(">>").unwrap();
        let fwd = format!("{rhs}>>{lhs}");
        let products: std::collections::BTreeSet<String> = run_reactants(&fwd, &reactant_refs)
            .unwrap()
            .into_iter()
            .flatten()
            .map(|m| to_canonical(&m))
            .collect();
        assert_eq!(
            products.len(),
            3,
            "expected exactly the 3 open-chain ortho/meta/para regioisomers: {products:?}"
        );
        assert!(
            !products.contains(&target_canon),
            "toluene + propionyl chloride as two separate molecules must never be able to \
             forward-produce the fused-ring target -- if this starts passing, the underlying \
             chematic/apply_retro fragment-tether-tracking behavior changed and this \
             classification needs re-checking, not just the assertion updated: {products:?}"
        );
    }

    #[test]
    fn declared_forward_smirks_none_for_smirks_based_rule() {
        // co_aliphatic_cleavage already has a real smirks -- this function
        // must not do anything for it (forward.rs's existing
        // rules_by_template_id path already handles it).
        assert_eq!(
            declared_forward_smirks(
                "co_aliphatic_cleavage",
                "CO",
                &["C".to_string(), "O".to_string()]
            ),
            None
        );
    }

    #[test]
    fn declared_forward_smirks_none_for_unknown_rule() {
        assert_eq!(
            declared_forward_smirks("not_a_real_rule", "CO", &["C".to_string(), "O".to_string()]),
            None
        );
    }

    #[test]
    fn declared_forward_smirks_ester_cleavage_real_repro() {
        // The exact repro from docs/design/retro-rule-precision-gaps-v0.md
        // #5: aspirin's ester_cleavage-based route.
        let target = "CC(=O)Oc1ccccc1C(=O)O";
        let precursors = vec![
            to_canonical(&mol_from_smiles("CC(=O)O").unwrap()),
            to_canonical(&mol_from_smiles("Oc1ccccc1C(=O)O").unwrap()),
        ];
        let smirks = declared_forward_smirks("ester_cleavage", target, &precursors)
            .expect("must derive a forward smirks for this real ester_cleavage outcome");

        // Real, atom-mapped, and -- most importantly -- actually replays
        // forward via chematic's own reaction engine, exactly the check
        // bridge::forward::validate_step_forward performs.
        assert!(
            smirks.contains(':'),
            "expected atom-mapped smirks: {smirks}"
        );
        let (lhs, rhs) = smirks.split_once(">>").unwrap();
        let forward_smirks = format!("{rhs}>>{lhs}");
        let reactant_mols: Vec<Molecule> = rhs
            .split('.')
            .map(|s| mol_from_smiles(s).unwrap())
            .collect();
        let reactant_refs: Vec<&Molecule> = reactant_mols.iter().collect();
        let products = run_reactants(&forward_smirks, &reactant_refs)
            .expect("forward-reversed smirks must at least parse and run");
        let target_canon = to_canonical(&mol_from_smiles(target).unwrap());
        let reproduced = products.iter().any(|pset| {
            pset.iter()
                .any(|p| to_canonical(&clear_atom_maps(p)) == target_canon)
        });
        assert!(
            reproduced,
            "forward-reversed smirks must reproduce the real target: {smirks}"
        );
    }

    #[test]
    fn declared_forward_smirks_suzuki_retro_real_repro() {
        let target = "c1ccc(-c2ccccc2)cc1"; // biphenyl
        let precursors = vec![
            to_canonical(&mol_from_smiles("Brc1ccccc1").unwrap()),
            to_canonical(&mol_from_smiles("OB(O)c1ccccc1").unwrap()),
        ];
        let smirks = declared_forward_smirks("suzuki_retro", target, &precursors)
            .expect("must derive a forward smirks for this real suzuki_retro outcome");
        assert!(
            smirks.contains(':'),
            "expected atom-mapped smirks: {smirks}"
        );
    }

    #[test]
    fn declared_forward_smirks_preserves_tetrahedral_stereocenter() {
        // methyl (S)-2-acetoxypropanoate: acetate ester of methyl (S)-lactate.
        // `atom.chirality` is an Atom-level field copied by `Atom::clone`, so
        // with_sequential_atom_maps's rebuild carries it through even before
        // the copy_stereo_*/copy_bond_directions_from calls below -- verified
        // directly, not just asserted; those calls stay for parity with
        // clear_atom_maps and defense against stereo forms this specific
        // case doesn't exercise (e.g. enhanced/relative stereo groups).
        let target = "CC(=O)O[C@@H](C)C(=O)OC";
        let target_canon = to_canonical(&mol_from_smiles(target).unwrap());
        let rule = rr("ester_cleavage", "");
        let outcomes = apply_retro(&mol_from_smiles(target).unwrap(), &rule);
        assert!(!outcomes.is_empty());
        for outcome in &outcomes {
            let precursors: Vec<String> = outcome
                .iter()
                .map(|p| to_canonical(&clear_atom_maps(&p.mol)))
                .collect();
            let smirks = declared_forward_smirks("ester_cleavage", target, &precursors)
                .unwrap_or_else(|| panic!("must derive a forward smirks for {precursors:?}"));
            assert!(
                smirks.contains('@'),
                "expected the stereocenter to survive into the smirks: {smirks}"
            );
            let (lhs, rhs) = smirks.split_once(">>").unwrap();
            let forward_smirks = format!("{rhs}>>{lhs}");
            let reactant_mols: Vec<Molecule> = rhs
                .split('.')
                .map(|s| mol_from_smiles(s).unwrap())
                .collect();
            let reactant_refs: Vec<&Molecule> = reactant_mols.iter().collect();
            let products = run_reactants(&forward_smirks, &reactant_refs)
                .expect("forward-reversed smirks must at least parse and run");
            let reproduced_exact = products.iter().any(|pset| {
                pset.iter()
                    .any(|p| to_canonical(&clear_atom_maps(p)) == target_canon)
            });
            assert!(
                reproduced_exact,
                "forward replay must reproduce the exact stereo-tagged target: {smirks}"
            );
        }
    }

    #[test]
    fn declared_forward_smirks_preserves_spectator_e_z_double_bond() {
        // The E-alkene is a spectator far from the ester bond being cut --
        // molecule-level bond-direction data (copy_bond_directions_from),
        // not the atom-level chirality field the tetrahedral case above
        // exercises.
        let target = "CC(=O)Oc1ccc(/C=C/C)cc1";
        let target_canon = to_canonical(&mol_from_smiles(target).unwrap());
        let rule = rr("ester_cleavage", "");
        let outcomes = apply_retro(&mol_from_smiles(target).unwrap(), &rule);
        assert!(!outcomes.is_empty());
        for outcome in &outcomes {
            let precursors: Vec<String> = outcome
                .iter()
                .map(|p| to_canonical(&clear_atom_maps(&p.mol)))
                .collect();
            let smirks = declared_forward_smirks("ester_cleavage", target, &precursors)
                .unwrap_or_else(|| panic!("must derive a forward smirks for {precursors:?}"));
            assert!(
                smirks.contains('/'),
                "expected the E double bond to survive into the smirks: {smirks}"
            );
            let (lhs, rhs) = smirks.split_once(">>").unwrap();
            let forward_smirks = format!("{rhs}>>{lhs}");
            let reactant_mols: Vec<Molecule> = rhs
                .split('.')
                .map(|s| mol_from_smiles(s).unwrap())
                .collect();
            let reactant_refs: Vec<&Molecule> = reactant_mols.iter().collect();
            let products = run_reactants(&forward_smirks, &reactant_refs)
                .expect("forward-reversed smirks must at least parse and run");
            let reproduced_exact = products.iter().any(|pset| {
                pset.iter()
                    .any(|p| to_canonical(&clear_atom_maps(p)) == target_canon)
            });
            assert!(
                reproduced_exact,
                "forward replay must reproduce the exact E-configured target: {smirks}"
            );
        }
    }

    #[test]
    fn declared_forward_smirks_returns_none_for_a_precursor_set_that_was_never_produced() {
        // A fabricated, chemically-unrelated precursor pair for this target
        // -- must not fabricate a "close enough" match.
        let target = "CC(=O)Oc1ccccc1C(=O)O";
        let bogus_precursors = vec!["C".to_string(), "O".to_string()];
        assert_eq!(
            declared_forward_smirks("ester_cleavage", target, &bogus_precursors),
            None
        );
    }

    #[test]
    fn extracted_824_oxazolidinone_is_genuine_template_error() {
        // Finding #4 pilot (2026-08-23), Invalid+balanced step, target
        // O=C2NCC(O2)Cc1ccccc1 (5-benzyl-1,3-oxazolidin-2-one). SMIRKS'
        // reactant pattern is a plain acyclic carbamate: C5-O6-C3(=O4)-N2H-C1,
        // with no ring-membership or connectivity constraint tying [C:1] and
        // [C:5] together. When it matches this target, [C:1] and [C:5] are
        // mapped to the ring's C4/C5 atoms -- which the target *also* bonds
        // directly to each other (the ring's C4-C5 bond) outside the matched
        // substructure entirely. The RHS's two-fragment split
        // ([C:1]-N=C=O . [C:5]-OH) has no way to express "these two atoms
        // stay tethered" -- it silently drops the C4-C5 bond that made this
        // a ring in the first place, same ring-tether-blindness failure
        // class already confirmed for extracted_112 (genuine_template_error,
        // PR #180). Root cause of the resulting Invalid step: the retro
        // step claims methyl isocyanate + 2-phenylethanol as separate,
        // untethered precursors, but a real synthesis of this oxazolidinone
        // needs an amino-alcohol (e.g. phenylalaninol-like) reacting with an
        // *unsubstituted* one-carbon carbonyl-transfer reagent (phosgene,
        // CDI, HNCO) -- not a fully alkylated isocyanate paired with an
        // unrelated free alcohol.
        let target = "O=C2NCC(O2)Cc1ccccc1";
        let precursors = ["C(=NC)=O".to_string(), "OCCc1ccccc1".to_string()];
        let smirks = "[C:5]-[O:6]-[C:3](=[O:4])-[NH:2]-[C:1]>>[C:1]-[N:2]=[C:3]=[O:4].[C:5]-[OH:6]";
        let rule = rr("extracted_824", smirks);
        let target_mol = mol_from_smiles(target).unwrap();
        let target_canon = to_canonical(&target_mol);

        // apply_retro reproduces the exact wrong disconnection fresh, from
        // the real target -- confirms this isn't a harness-only artifact.
        let retro_outcomes = apply_retro(&target_mol, &rule);
        let retro_smiles: Vec<Vec<String>> = retro_outcomes
            .iter()
            .map(|outcome| outcome.iter().map(|p| to_canonical(&p.mol)).collect())
            .collect();
        let expected_precursors: Vec<String> = precursors
            .iter()
            .map(|s| to_canonical(&mol_from_smiles(s).unwrap()))
            .collect();
        assert!(
            retro_smiles.iter().any(|outcome| outcome
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                == expected_precursors.iter().collect()),
            "apply_retro must still reproduce this exact wrong disconnection: {retro_smiles:?}"
        );

        // Forward replay of the declared precursors: the only possible
        // product is the plain open-chain N-methyl carbamate ester -- never
        // the fused-ring target, which would require the alcohol and amine
        // to already be tethered in one molecule before ring closure.
        let reactant_mols: Vec<Molecule> = precursors
            .iter()
            .map(|s| mol_from_smiles(s).unwrap())
            .collect();
        let reactant_refs: Vec<&Molecule> = reactant_mols.iter().collect();
        let (lhs, rhs) = smirks.split_once(">>").unwrap();
        let fwd = format!("{rhs}>>{lhs}");
        let products: std::collections::BTreeSet<String> = run_reactants(&fwd, &reactant_refs)
            .unwrap()
            .into_iter()
            .flatten()
            .map(|m| to_canonical(&m))
            .collect();
        let expected_open_chain = to_canonical(&mol_from_smiles("O=C(OCCc1ccccc1)NC").unwrap());
        assert_eq!(
            products,
            std::collections::BTreeSet::from([expected_open_chain]),
            "expected exactly the single open-chain N-methyl carbamate ester: {products:?}"
        );
        assert!(
            !products.contains(&target_canon),
            "methyl isocyanate + 2-phenylethanol as two separate, untethered molecules must \
             never be able to forward-produce the fused-ring target -- if this starts passing, \
             the underlying chematic/apply_retro fragment-tether-tracking behavior changed and \
             this classification needs re-checking, not just the assertion updated: {products:?}"
        );
    }
}

// ── Phase 15: tetrahedral @/@@ full integration ──────────────────────────────

#[cfg(test)]
mod phase15_stereo {
    use super::*;

    /// Phase 15.1 — @/@@ templates load from file and apply correctly.
    /// The top-500 extracted templates contain 2 stereo-specific rules.
    /// Both must load via load_rules_from_file and respect chirality.
    #[test]
    fn stereo_templates_load_from_file_and_filter() {
        let rules = load_rules_from_file("data/templates_extracted.smi");
        let stereo_rules: Vec<_> = rules.iter().filter(|r| r.smirks.contains('@')).collect();
        assert!(
            stereo_rules.len() >= 2,
            "top-500 must contain ≥2 @/@@ templates; got {}",
            stereo_rules.len()
        );
        // Apply the R-selective template ([C@H]) to R and S secondary alcohols
        let r_rule = stereo_rules
            .iter()
            .find(|r| r.smirks.contains("[C@H"))
            .expect("R-selective template not found");
        let r_alcohol = parse("C[C@H](O)c1ccccc1").unwrap(); // (R)-1-phenylethanol
        let s_alcohol = parse("C[C@@H](O)c1ccccc1").unwrap(); // (S)-1-phenylethanol
        assert!(
            !apply_retro(&r_alcohol, r_rule).is_empty(),
            "R-template must produce routes for R-alcohol"
        );
        assert!(
            apply_retro(&s_alcohol, r_rule).is_empty(),
            "R-template must reject S-alcohol"
        );
    }

    /// Phase 15.2 — SMIRKS without @/@@ must match both enantiomers (permissive).
    #[test]
    fn non_stereo_smirks_matches_both_enantiomers() {
        // No stereo annotation in reactant → both R and S must match
        let smirks = "[C:1][CH:2]([OH:3])[c:4]>>[C:1][C:2](=[O:3])[c:4]";
        let r_mol = parse("C[C@H](O)c1ccccc1").unwrap();
        let s_mol = parse("C[C@@H](O)c1ccccc1").unwrap();
        assert!(
            !run_reactants(smirks, &[&r_mol])
                .unwrap_or_default()
                .is_empty(),
            "non-stereo SMIRKS must match R-alcohol"
        );
        assert!(
            !run_reactants(smirks, &[&s_mol])
                .unwrap_or_default()
                .is_empty(),
            "non-stereo SMIRKS must match S-alcohol"
        );
    }

    /// Phase 15.3 — Stereo transfer to product (chematic #20 point 2).
    /// SMIRKS product template with @/@@ must produce a stereodefined product,
    /// and the filter rejects the wrong enantiomer (L-alanine example from chematic #20).
    #[test]
    fn stereo_transferred_to_product() {
        // Retro-reduction of L-alanine: [N:1][C@@H:2](C)C(=O)O → [N:1][C@@H:2](C)C=O
        // L-alanine (N[C@@H](C)C(=O)O) must match; D-alanine must not.
        // Product retains @@ stereo — verifies TRANSFER (chematic #20 point 2).
        let smirks = "[N:1][C@@H:2](C)C(=O)O>>[N:1][C@@H:2](C)C=O";
        let l_ala = parse("N[C@@H](C)C(=O)O").unwrap(); // L-alanine — should match
        let d_ala = parse("N[C@H](C)C(=O)O").unwrap(); // D-alanine — must NOT match

        let l_results = run_reactants(smirks, &[&l_ala]).unwrap_or_default();
        let d_results = run_reactants(smirks, &[&d_ala]).unwrap_or_default();

        assert!(!l_results.is_empty(), "L-alanine must match @@-SMIRKS");
        assert!(
            d_results.is_empty(),
            "D-alanine must NOT match @@-SMIRKS; got {} result(s)",
            d_results.len()
        );

        // Product must carry @@ stereo (transfer confirmed)
        let product_smiles: Vec<String> = l_results[0].iter().map(canonical_smiles).collect();
        assert!(
            product_smiles.iter().any(|s| s.contains('@')),
            "product must carry @/@@ stereo annotation; got {:?}",
            product_smiles
        );
    }

    /// Phase 15.3 — Both @-specific and @@-specific templates resolve correctly
    /// from the USPTO-50k extracted template set (end-to-end pipeline).
    #[test]
    fn both_stereo_templates_are_enantiomer_selective() {
        let rules = load_rules_from_file("data/templates_extracted.smi");
        let r_rule = rules.iter().find(|r| r.smirks.contains("[C@H")).unwrap();
        let s_rule = rules.iter().find(|r| r.smirks.contains("[C@@H")).unwrap();
        let r_mol = parse("C[C@H](O)c1ccccc1").unwrap();
        let s_mol = parse("C[C@@H](O)c1ccccc1").unwrap();
        // R-template: R matches, S rejected
        assert!(!apply_retro(&r_mol, r_rule).is_empty());
        assert!(apply_retro(&s_mol, r_rule).is_empty());
        // S-template: S matches, R rejected
        assert!(!apply_retro(&s_mol, s_rule).is_empty());
        assert!(apply_retro(&r_mol, s_rule).is_empty());
    }
}
