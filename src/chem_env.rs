use std::fs;

use rustc_hash::{FxHashMap, FxHashSet};

use anyhow::{Context, Result};
use chematic::chem::standardize::{StandardizeOptions, ZwitterionHandling, standardize};
use chematic::core::{Atom, AtomIdx, BondIdx, BondOrder, Element, MoleculeBuilder};
use chematic::rxn::run_reactants;
use chematic::smarts::{QueryMolecule, find_matches, parse_smarts};
use chematic::smiles::{canonical_smiles, parse};

pub use chematic::core::Molecule;

#[derive(Debug, Clone)]
pub struct RetroRule {
    pub name: String,
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
            smirks: String::new(),
            weight: 1.0,
            required_elements: 0,
        }
    }
}

/// Building-block library.
///
/// Two-tier storage for scalability:
/// - `canon_set`: canonical-SMILES FxHashSet used for all lookups (O(1), low memory).
///   Scales to millions of BBs (500k BBs ≈ 12 MB vs 2.8 GB for VF2 QueryMolecules).
/// - `vf2_index`: (atom_count, bond_count) → VF2 QueryMolecule fallback for small
///   sets (DEFAULT_BUILDING_BLOCKS). Provides a secondary confirmation when the
///   canonical-SMILES check fails, e.g. for molecules with explicit-H notation
///   produced by `run_reactants`.
///
/// In practice the canonical-SMILES path handles all lookups; the VF2 index
/// only activates when `bb_count ≤ VF2_THRESHOLD` (small in-memory sets).
pub struct ChemEnv {
    /// Canonical SMILES of every BB — primary fast lookup.
    canon_set: FxHashSet<String>,
    /// VF2 fallback for small sets (populated only when bb_count ≤ VF2_THRESHOLD).
    vf2_index: FxHashMap<(usize, usize), Vec<QueryMolecule>>,
    bb_count: usize,
}

/// BBs up to this count also build a VF2 index for secondary confirmation.
const VF2_THRESHOLD: usize = 2000;

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
        let mut vf2_raw: Vec<(usize, usize, QueryMolecule)> = Vec::new();
        let mut bb_count = 0usize;

        for smiles in iter {
            let Ok(mol) = parse(&smiles) else { continue };
            let canon = canonical_smiles(&mol);
            if !canon_set.insert(canon) {
                continue; // duplicate
            }
            bb_count += 1;
            // VF2 index only for small sets (skip parse_smarts for large sets to save memory)
            if bb_count <= VF2_THRESHOLD
                && let Ok(query) = parse_smarts(&smiles)
            {
                vf2_raw.push((mol.atom_count(), mol.bonds().count(), query));
            }
        }

        let mut vf2_index: FxHashMap<(usize, usize), Vec<QueryMolecule>> = FxHashMap::default();
        for (n_atoms, n_bonds, query) in vf2_raw {
            vf2_index.entry((n_atoms, n_bonds)).or_default().push(query);
        }

        Self {
            canon_set,
            vf2_index,
            bb_count,
        }
    }

    /// Number of building blocks in the library.
    pub fn bb_count(&self) -> usize {
        self.bb_count
    }

    /// Fast O(1) BB check for an already-canonical SMILES string.
    /// Skips molecule parsing and re-canonicalization. Use this when the
    /// input is guaranteed to be canonical (e.g. `FEntry.smiles` in search).
    pub fn is_building_block_smiles(&self, canonical_smi: &str) -> bool {
        self.canon_set.contains(canonical_smi)
    }

    /// Check if `mol` is in the building-block library.
    ///
    /// Primary: O(1) canonical-SMILES FxHashSet lookup.
    /// Fallback: VF2 subgraph isomorphism (small sets only, bb_count ≤ VF2_THRESHOLD).
    pub fn is_building_block(&self, mol: &Molecule) -> bool {
        let canon = canonical_smiles(mol);
        if self.canon_set.contains(&canon) {
            return true;
        }
        // VF2 fallback for small sets
        if !self.vf2_index.is_empty() {
            let key = (mol.atom_count(), mol.bonds().count());
            if let Some(candidates) = self.vf2_index.get(&key) {
                let n_atoms = mol.atom_count();
                return candidates
                    .iter()
                    .any(|q| find_matches(q, mol).iter().any(|m| m.len() == n_atoms));
            }
        }
        false
    }
}

pub fn mol_from_smiles(smiles: &str) -> Result<Molecule> {
    parse(smiles).with_context(|| format!("Failed to parse SMILES: {smiles}"))
}

pub fn to_canonical(mol: &Molecule) -> String {
    canonical_smiles(mol)
}

static STANDARDIZE_OPTS: StandardizeOptions = StandardizeOptions {
    canonical_tautomer: false,
    neutralize_charges: false,
    remove_explicit_h: true,
    largest_fragment_only: false,
    zwitterion_handling: ZwitterionHandling::Keep,
};

// ── Graph-based Ar-Ar bond cleavage (Suzuki retro) ─────────────────────────
//
// chematic's run_reactants seeds BFS globally, so applying the SMIRKS
// [c:1][c:2]>>[c:1]Br.[c:2] to biphenyl produces broken fragments like
// c(Br)(-c1ccccc1)cccc instead of clean Brc1ccccc1 + c1ccccc1.
// We work around this by computing the two connected components directly
// from the molecular graph using MoleculeBuilder.

/// Test whether removing the bond (a, b) disconnects the graph (i.e., it is a bridge bond).
fn is_bridge_bond(mol: &Molecule, a: AtomIdx, b: AtomIdx) -> bool {
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

        // Generate both orientations: which ring gets Br
        for (comp_br, cut, comp_plain) in [(&comp_a, a, &comp_b), (&comp_b, b, &comp_a)] {
            let Some(frag_br) = build_sub_molecule_with_br(mol, comp_br, cut) else {
                continue;
            };
            let Some(frag_plain) = build_sub_molecule(mol, comp_plain) else {
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

/// Apply a single retro-rule to a molecule.
/// Returns all possible precursor sets as (canonical_smiles, Molecule) pairs.
///
/// Rules with an empty `smirks` field are dispatched to graph-based handlers
/// (keyed by `name`). SMIRKS rules use chematic's run_reactants; fragments are
/// split on '.' in canonical SMILES and filtered for BFS-leakage artefacts.
pub fn apply_retro(mol: &Molecule, rule: &RetroRule) -> Vec<Vec<PrecursorMol>> {
    if rule.smirks.is_empty() {
        return match rule.name.as_str() {
            "suzuki_retro" => biaryl_cleavage(mol),
            "diaryl_sulfone_retro" => diaryl_sulfone_cleavage(mol),
            "amide_cleavage" => amide_cleavage(mol),
            "boc_deprotection_retro" => boc_deprotection(mol),
            "cbz_deprotection_retro" => cbz_deprotection(mol),
            _ => vec![],
        };
    }
    run_reactants(&rule.smirks, &[mol])
        .unwrap_or_default()
        .into_iter()
        .map(|products| {
            products
                .into_iter()
                .flat_map(|product_mol| split_fragments(&product_mol))
                .collect()
        })
        .collect()
}

/// A standardized precursor molecule with its canonical SMILES.
pub struct PrecursorMol {
    pub smiles: String,
    pub mol: Molecule,
}

/// Split a (possibly disconnected) molecule into standardized PrecursorMol fragments.
/// Filters out chemically invalid fragments (aromatic atoms outside any ring) that
/// arise from chematic's SMIRKS BFS leaking substituents across product templates.
fn split_fragments(mol: &Molecule) -> Vec<PrecursorMol> {
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

fn rr(name: &str, smirks: &str) -> RetroRule {
    let required_elements = required_elements_from_smirks(smirks);
    RetroRule {
        name: name.into(),
        smirks: smirks.into(),
        required_elements,
        ..Default::default()
    }
}

pub fn default_rules() -> Vec<RetroRule> {
    vec![
        // ── Acyl disconnections ──────────────────────────────────────────
        // Ester C(=O)-O → carboxylic acid + alcohol/phenol
        rr("ester_cleavage", "[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]"),
        // Graph-based: dispatched in apply_retro (SMIRKS-based had BFS-leakage)
        rr("amide_cleavage", ""),
        // Ar-C(=O)R → Ar-H + R-C(=O)Cl (Friedel-Crafts retro)
        rr(
            "friedel_crafts_acylation_retro",
            "[c:1][C:2](=[O:3])>>[c:1].[C:2](=[O:3])Cl",
        ),
        // ── Aryl C-heteroatom disconnections ────────────────────────────
        // Ar-COOH → Ar-H + HCOOH (retro-Kolbe-Schmitt / decarboxylation)
        rr(
            "aryl_carboxylation_retro",
            "[c:1][C:2](=O)O>>[c:1].[C:2](=O)O",
        ),
        // Ar-N → Ar-H + amine (retro-SNAr / retro-Chan-Lam)
        rr("aryl_amine_retro", "[c:1][N:2]>>[c:1].[N:2]"),
        // Ar-N → Ar-Br + amine (retro-Buchwald-Hartwig; gives halide BB)
        rr("buchwald_hartwig_retro", "[c:1][N:2]>>[c:1]Br.[N:2]"),
        // Ar-O → Ar-OH + leaving fragment (retro-Ullmann ether synthesis)
        rr("aryl_ether_retro", "[c:1][O:2]>>[c:1]O.[O:2]"),
        // ── Aryl C-halide disconnections ────────────────────────────────
        // Ar-Cl → Ar-H + HCl (retro-SNAr or retro-Pd C-Cl activation)
        rr("aryl_chloride_retro", "[c:1][Cl]>>[c:1]"),
        // Ar-I → Ar-H (retro-Pd/Cu C-I; iodides are activated leaving groups)
        rr("aryl_iodide_retro", "[c:1][I]>>[c:1]"),
        // Ar-F → Ar-H (retro-SNAr; fluorine is best SNAr leaving group)
        rr("aryl_fluoride_snAr_retro", "[c:1][F]>>[c:1]"),
        // Ar-Cl → Ar-Br (halogen exchange retro; Ar-Br is often a cheaper BB)
        rr("aryl_chloride_to_bromide", "[c:1][Cl]>>[c:1][Br]"),
        // ── Aryl C-C disconnections ──────────────────────────────────────
        // Graph-based: find Ar-Ar bridge bonds and split into Ar-Br + Ar.
        rr("suzuki_retro", ""),
        // Ar-CH=CH-R → Ar-Br + CH2=CH-R (retro-Heck, internal alkene)
        rr("heck_retro", "[c:1][CH:2]=[CH:3]>>[c:1][Br].[CH2:2]=[CH:3]"),
        // Ar-CH=CH2 → Ar-Br + CH2=CH2 (retro-Heck, terminal alkene / styrene)
        rr(
            "heck_retro_terminal",
            "[c:1][CH:2]=[CH2:3]>>[c:1][Br].[CH2:2]=[CH2:3]",
        ),
        // Ar-alkyl → Ar-Br + alkyl (retro-Negishi; Pd-catalyzed C-C)
        rr("negishi_retro", "[c:1][CH2:2]>>[c:1][Br].[CH3:2]"),
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
        // Ar-SO2-NHR → Ar-SO2Cl + HNR (sulfonyl chloride + amine)
        rr(
            "sulfonamide_retro",
            "[S:1](=O)(=O)[N:2]>>[S:1](=O)(=O)Cl.[N:2]",
        ),
        // Ar-SO2-Ar' → Ar-SO2Cl + Ar'H (graph-based; Friedel-Crafts sulfonylation retro)
        rr("diaryl_sulfone_retro", ""),
        // ── N-protection / deprotection ──────────────────────────────────────
        // N-Boc → N-H (deprotect: TFA removes Boc). Graph-based to avoid leakage.
        rr("boc_deprotection_retro", ""),
        // ── N-alkylation (more specific than cn_aliphatic_cleavage) ──────────
        // N-CH2Ar → N-H + BrCH2Ar (N-benzyl retro)
        rr(
            "n_benzylation_retro",
            "[N:1][CH2:2][c:3]>>[N:1].[Br][CH2:2][c:3]",
        ),
        // ── Grignard / organolithium retro ───────────────────────────────────
        // Tertiary alcohol → ketone + R-MgBr (retro-Grignard)
        rr(
            "grignard_addition_retro",
            "[C:1]([OH:2])([C:3])[C:4]>>[C:1](=O)[C:3].[C:4]",
        ),
        // ── Claisen / Dieckmann condensation ────────────────────────────────
        // β-ketoester → ester + ester (retro-Claisen condensation)
        rr(
            "claisen_retro",
            "[C:1](=O)[CH2:2][C:3](=O)[O:4]>>[C:1](=O)O.[C:2]=[C:3][O:4]",
        ),
        // ── Michael addition retro ───────────────────────────────────────────
        // R-CH2-C(=O)R' ← CH2=C(=O)R' + H (retro-1,4-addition at α)
        rr(
            "michael_retro",
            "[C:1][CH2:2][C:3]=[O:4]>>[C:1].[CH2:2]=[C:3][OH:4]",
        ),
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
        ("Cl", 17), ("Br", 35), ("Si", 14), ("Se", 34), ("Te", 52),
        ("Sn", 50), ("Zn", 30), ("Pd", 46), ("Cu", 29), ("Fe", 26),
        ("B", 5), ("C", 6), ("N", 7), ("O", 8), ("F", 9),
        ("P", 15), ("S", 16), ("I", 53),
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
                while i < bytes.len() && bytes[i] != b']' { i += 1; }
            }
            b'(' => stack.push(prev),
            b')' => prev = stack.pop().flatten(),
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
pub struct TemplateBondIndex {
    index: FxHashMap<(u8, u8), Vec<usize>>,
    /// Graph-based rules (empty SMIRKS) — always included.
    graph_indices: Vec<usize>,
    /// Rules with unparseable / empty bond pairs — included as fallback.
    fallback_indices: Vec<usize>,
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
        Self { index, graph_indices, fallback_indices }
    }

    /// Return indices (into the original `rules` slice) of templates relevant to `mol`.
    /// Includes graph-based rules and fallback rules unconditionally.
    /// If `top_k > 0`, the SMIRKS-matched candidates are trimmed to the top-K by weight.
    pub fn retrieve(&self, mol: &Molecule, top_k: usize, rules: &[RetroRule]) -> Vec<usize> {
        let mut seen: FxHashSet<usize> = FxHashSet::default();
        let mut candidates: Vec<usize> = Vec::new();

        // Always include graph-based and fallback rules.
        for &idx in &self.graph_indices {
            if seen.insert(idx) { candidates.push(idx); }
        }
        for &idx in &self.fallback_indices {
            if seen.insert(idx) { candidates.push(idx); }
        }

        // Retrieve SMIRKS rules matching bonds present in the target.
        for (atom_idx, _) in mol.atoms() {
            let e1 = mol.atom(atom_idx).element.atomic_number();
            for (nb_idx, _bond_idx) in mol.neighbors(atom_idx) {
                // Only process each bond once (lower-index atom first).
                if nb_idx <= atom_idx { continue; }
                let e2 = mol.atom(nb_idx).element.atomic_number();
                let pair = if e1 <= e2 { (e1, e2) } else { (e2, e1) };
                if let Some(indices) = self.index.get(&pair) {
                    for &idx in indices {
                        if seen.insert(idx) { candidates.push(idx); }
                    }
                }
            }
        }

        if top_k > 0 && candidates.len() > top_k {
            // Sort SMIRKS portion by weight desc, keep top_k total.
            let fixed = self.graph_indices.len() + self.fallback_indices.len();
            candidates[fixed..].sort_unstable_by(|&a, &b| {
                rules[b].weight.partial_cmp(&rules[a].weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            candidates.truncate(fixed + top_k);
        }
        candidates
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
            let required_elements = required_elements_from_smirks(smirks);
            Some(RetroRule {
                name: format!("extracted_{i}"),
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
    fn parse_aspirin_roundtrip() {
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        assert_eq!(mol.atom_count(), 13);
    }

    #[test]
    fn building_block_recognized_by_vf2() {
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
        // VF2 must match even when canonical SMILES differ (L2 in lessons.md).
        let env = ChemEnv::in_memory(&["CC(=O)O"]);
        let mol = mol_from_smiles("OC(C)=O").unwrap(); // different SMILES, same molecule
        assert!(
            env.is_building_block(&mol),
            "OC(C)=O is the same as CC(=O)O"
        );
    }

    #[test]
    fn benzoic_acid_variant_matches() {
        // Different SMILES representations of benzoic acid must match via VF2 (L2).
        let env = ChemEnv::in_memory(&["c1ccccc1C(=O)O"]);
        let mol = mol_from_smiles("c1c(C(=O)O)cccc1").unwrap();
        assert!(
            env.is_building_block(&mol),
            "c1c(C(=O)O)cccc1 is benzoic acid"
        );
    }

    #[test]
    fn ester_cleavage_fires_on_aspirin() {
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        let rule = rr("ester_cleavage", "[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]");
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty(), "ester_cleavage must match aspirin");
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

    #[test]
    fn suzuki_retro_biphenyl_gives_bromobenzene_and_benzene() {
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

        // Expect exactly bromobenzene and benzene (in some canonical form)
        let has_bromobenzene = all_smiles
            .iter()
            .any(|s| s.contains("Br") && s.contains("c1ccccc1"));
        let has_benzene = all_smiles.iter().any(|s| s == "c1ccccc1");
        assert!(
            has_bromobenzene,
            "expected bromobenzene fragment; got {all_smiles:?}"
        );
        assert!(has_benzene, "expected benzene fragment; got {all_smiles:?}");
    }

    #[test]
    fn suzuki_retro_biphenyl_solvable_with_bb() {
        // End-to-end: the engine must resolve biphenyl given bromobenzene + benzene as BBs.
        use crate::search::{SearchConfig, find_routes};
        let env = ChemEnv::in_memory(&["Brc1ccccc1", "c1ccccc1"]);
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

    fn smiles_set(results: &[Vec<PrecursorMol>], idx: usize) -> Vec<String> {
        results[idx].iter().map(|p| p.smiles.clone()).collect()
    }

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
    fn heck_retro_internal_on_stilbene() {
        // (E)-stilbene: c1ccccc1/C=C/c1ccccc1
        let mol = mol_from_smiles("C(=Cc1ccccc1)c1ccccc1").unwrap();
        let rule = rr("heck_retro", "[c:1][CH:2]=[CH:3]>>[c:1][Br].[CH2:2]=[CH:3]");
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty(), "heck_retro must fire on stilbene");
        let flat: Vec<_> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.as_str()))
            .collect();
        assert!(
            flat.iter().any(|s| s.contains("Br")),
            "products must include aryl bromide; got {flat:?}"
        );
    }

    #[test]
    fn negishi_retro_on_ethylbenzene() {
        // negishi_retro SMIRKS [c:1][CH2:2] matches the benzylic CH2 in ethylbenzene,
        // not the methyl (CH3) in toluene (toluene has 3H on that carbon, not 2H).
        let mol = mol_from_smiles("CCc1ccccc1").unwrap();
        let rule = rr("negishi_retro", "[c:1][CH2:2]>>[c:1][Br].[CH3:2]");
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "negishi_retro must fire on ethylbenzene (benzylic CH2)"
        );
        let flat: Vec<_> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.as_str()))
            .collect();
        assert!(
            flat.iter().any(|s| s.contains("Br")),
            "products must include aryl bromide; got {flat:?}"
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

    #[test]
    fn aryl_chloride_retro_on_chlorobenzene() {
        let mol = mol_from_smiles("Clc1ccccc1").unwrap();
        let rule = rr("aryl_chloride_retro", "[c:1][Cl]>>[c:1]");
        let results = apply_retro(&mol, &rule);
        assert!(
            !results.is_empty(),
            "aryl_chloride_retro must fire on chlorobenzene"
        );
        let flat: Vec<_> = results
            .iter()
            .flat_map(|s| s.iter().map(|p| p.smiles.as_str()))
            .collect();
        let benzene_smi = canonical_smiles(&mol_from_smiles("c1ccccc1").unwrap());
        assert!(
            flat.iter().any(|s| *s == benzene_smi),
            "products must include benzene; got {flat:?}"
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
        let has_benzene = flat.iter().any(|s| *s == "c1ccccc1");
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
        let product_smiles: Vec<String> =
            l_results[0].iter().map(|m| canonical_smiles(m)).collect();
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
