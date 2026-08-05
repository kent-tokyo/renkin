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

static STANDARDIZE_OPTS: StandardizeOptions = StandardizeOptions {
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
            "sulfonamide_retro" => sulfonamide_cleavage_graph(mol),
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

// ── Hash-atom ([#N]) wildcard expansion ────────────────────────────────
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
// validation reports success -- see Issue #72 follow-up investigation for
// the empirical confirmation.
//
// `#N` genuinely doesn't say whether the atom is aromatic, so this module
// never guesses a single answer: it expands each bracket into every
// aromatic/aliphatic reading and lets real re-parsing (not a heuristic)
// decide which readings are even syntactically valid. The union of the
// resulting variants is exactly the semantics `[#N]` describes -- no
// narrowing. Candidates that don't independently pass
// `chematic::rxn::parse_reaction` are dropped, never guessed into.

/// Upper bound on how many variant SMIRKS one template can expand into.
/// The checked-in extracted-template corpora need at most 8 (`k=3`); a
/// larger, differently-curated template file could have more distinct
/// `[#N]` atoms per template, so this cap exists to keep expansion
/// bounded. Never applied silently -- see `HashAtomExpansion::Expanded`'s
/// `capped` field.
const MAX_HASH_ATOM_VARIANTS: usize = 16;

/// One `[#N]`/`[#N:map]` bracket-atom occurrence in a raw SMIRKS string.
/// `byte_range` covers the whole bracket (`[` through `]` inclusive) so it
/// can be sliced out and replaced.
struct HashAtomOccurrence {
    byte_range: std::ops::Range<usize>,
    atomic_number: u8,
    atom_map: Option<u32>,
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
            occurrences.push(HashAtomOccurrence {
                byte_range: start..end,
                atomic_number: atomic_number as u8,
                atom_map,
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

/// Result of attempting to expand a raw SMIRKS's `[#N]` atoms into
/// concrete-element variants. See the module docs above `find_hash_atoms`.
enum HashAtomExpansion {
    /// No `[#N]` atoms found -- caller should use the SMIRKS unchanged.
    NotApplicable,
    /// Found `[#N]` atoms but couldn't safely/usefully expand them (a
    /// combined primitive, inconsistent atom-map usage across the two
    /// sides, or zero candidate variants survived re-parsing) -- caller
    /// should fall back to the original, unmodified SMIRKS, i.e. today's
    /// pre-fix behavior for that one template (load succeeds, apply
    /// fails) rather than a regression.
    Unexpandable,
    /// One or more variant SMIRKS strings, each independently confirmed
    /// parseable by `chematic::rxn::parse_reaction`. `total_combinations`
    /// is the full 2^k count before any cap; `capped` is true when
    /// `variants.len() < total_combinations` because the space exceeded
    /// `MAX_HASH_ATOM_VARIANTS` -- callers must surface `capped`
    /// (`load_rules_from_file` does, via an `eprintln!` warning), never
    /// swallow it.
    Expanded {
        variants: Vec<String>,
        total_combinations: usize,
        capped: bool,
    },
}

fn expand_hash_atom_variants(smirks: &str) -> HashAtomExpansion {
    let occurrences = match find_hash_atoms(smirks) {
        Some(o) => o,
        None => return HashAtomExpansion::Unexpandable,
    };
    if occurrences.is_empty() {
        return HashAtomExpansion::NotApplicable;
    }

    // Group occurrences by atom-map number: the same atom-map appearing on
    // both sides of `>>` must get the same element/aromaticity choice.
    // Unmapped `[#N]` atoms (no atom-map -- can't have "another side" to
    // agree with) each get their own independent group.
    let mut group_order: Vec<Option<u32>> = Vec::new();
    let mut group_members: Vec<Vec<usize>> = Vec::new();
    let mut group_atomic_number: Vec<u8> = Vec::new();
    for (idx, occ) in occurrences.iter().enumerate() {
        let existing = occ
            .atom_map
            .and_then(|m| group_order.iter().position(|g| *g == Some(m)));
        match existing {
            Some(gi) => {
                if group_atomic_number[gi] != occ.atomic_number {
                    // Same atom-map resolves to two different elements --
                    // internally inconsistent template; don't guess.
                    return HashAtomExpansion::Unexpandable;
                }
                group_members[gi].push(idx);
            }
            None => {
                group_order.push(occ.atom_map);
                group_members.push(vec![idx]);
                group_atomic_number.push(occ.atomic_number);
            }
        }
    }

    let mut group_candidates: Vec<Vec<String>> = Vec::with_capacity(group_order.len());
    for &an in &group_atomic_number {
        let candidates = hash_atom_candidate_symbols(an);
        if candidates.is_empty() {
            return HashAtomExpansion::Unexpandable;
        }
        group_candidates.push(candidates);
    }

    let total_combinations: usize = group_candidates.iter().map(Vec::len).product();
    let capped = total_combinations > MAX_HASH_ATOM_VARIANTS;
    let combos_to_try = total_combinations.min(MAX_HASH_ATOM_VARIANTS);

    // Deterministic mixed-radix enumeration ("odometer"): combo_indices[g]
    // selects group g's candidate symbol. Same order every run (no
    // randomness), so a capped template's tried subset is reproducible.
    // The cap bounds how many combinations are *attempted* (each costs a
    // real `parse_reaction` call), not how many end up validating --
    // those are independent (a combination can be tried and still fail).
    let mut combo_indices = vec![0usize; group_candidates.len()];
    let mut variants = Vec::new();
    for _ in 0..combos_to_try {
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
        return HashAtomExpansion::Unexpandable;
    }
    HashAtomExpansion::Expanded {
        variants,
        total_combinations,
        capped,
    }
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
        // Ar-N → Ar-H + amine (retro-SNAr / retro-Chan-Lam)
        rr("aryl_amine_retro", "[c:1][N:2]>>[c:1].[N:2]"),
        // Ar-N → Ar-Br + amine (retro-Buchwald-Hartwig; gives halide BB)
        rr("buchwald_hartwig_retro", "[c:1][N:2]>>[c:1]Br.[N:2]"),
        // Ar-O → Ar-OH + leaving fragment (retro-Ullmann ether synthesis)
        rr("aryl_ether_retro", "[c:1][O:2]>>[c:1]O.[O:2]"),
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
        // Ar-SO2-NHR → Ar-SO2Cl + HNR. Graph-based (avoids BFS-leakage).
        rr("sulfonamide_retro", ""),
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

        if top_k > 0 && candidates.len() > top_k {
            // Sort SMIRKS portion by weight desc, keep top_k total.
            let fixed = self.graph_indices.len() + self.fallback_indices.len();
            candidates[fixed..].sort_unstable_by(|&a, &b| {
                rules[b]
                    .weight
                    .partial_cmp(&rules[a].weight)
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
        .flat_map(|(i, line)| {
            // Format is exactly 2 tab-separated columns today: SMIRKS, count.
            // `splitn(2, '\t')`'s second half is everything after the first tab,
            // including any further tab-separated content -- so a naive 3rd column
            // (e.g. a template ID or DOI for provenance metadata) added later without
            // a format-version bump won't error here. It'll just make `count.parse()`
            // fail on the combined string and silently fall back to `weight = 1.0`
            // via `.unwrap_or(1.0)` below, corrupting the frequency weight for every
            // such line. Whoever adds a 3rd column needs to change this split first.
            let mut cols = line.splitn(2, '\t');
            let Some(smirks) = cols.next().map(str::trim) else {
                return vec![];
            };
            let count: f64 = cols
                .next()
                .and_then(|c| c.trim().parse().ok())
                .unwrap_or(1.0);
            let weight = (count + 1.0).ln();
            let Some(reactant) = smirks.split(">>").next() else {
                return vec![];
            };
            // Validate that chematic can parse the reactant SMARTS pattern.
            if parse_smarts(reactant).is_err() {
                return vec![];
            }
            // Stable identity computed once from the *original* raw SMIRKS --
            // every hash-atom variant of this line shares it (see
            // `expand_hash_atom_variants`'s docs), so a template-keyed
            // sidecar (e.g. the ring-context guard's metadata) generated
            // against this file still resolves for the fixed-up variants,
            // rather than silently missing them.
            let template_id = template_id_for_smirks(smirks);
            match expand_hash_atom_variants(smirks) {
                HashAtomExpansion::NotApplicable => {
                    let required_elements = required_elements_from_smirks(smirks);
                    vec![RetroRule {
                        name: format!("extracted_{i}"),
                        template_id,
                        smirks: smirks.to_string(),
                        weight,
                        required_elements,
                    }]
                }
                HashAtomExpansion::Unexpandable => {
                    // Same as today's pre-fix behavior: keep the single,
                    // unmodified rule (it loads fine; `run_reactants` will
                    // still fail on it at apply time, exactly as before --
                    // not a regression, just not fixed by this expansion).
                    let required_elements = required_elements_from_smirks(smirks);
                    vec![RetroRule {
                        name: format!("extracted_{i}"),
                        template_id,
                        smirks: smirks.to_string(),
                        weight,
                        required_elements,
                    }]
                }
                HashAtomExpansion::Expanded {
                    variants,
                    total_combinations,
                    capped,
                } => {
                    if capped {
                        eprintln!(
                            "Warning: template extracted_{i} has {total_combinations} \
                             hash-atom ([#N]) variant combinations, exceeding the \
                             {MAX_HASH_ATOM_VARIANTS}-variant cap -- only the first \
                             {MAX_HASH_ATOM_VARIANTS} were generated and validated; \
                             the rest are not represented as usable rules"
                        );
                    }
                    variants
                        .into_iter()
                        .enumerate()
                        .map(|(vi, variant_smirks)| {
                            let required_elements = required_elements_from_smirks(&variant_smirks);
                            RetroRule {
                                name: format!("extracted_{i}_h{vi}"),
                                template_id: template_id.clone(),
                                smirks: variant_smirks,
                                weight,
                                required_elements,
                            }
                        })
                        .collect()
                }
            }
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
    fn parse_aspirin_roundtrip() {
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        assert_eq!(mol.atom_count(), 13);
    }

    // ── Hash-atom ([#N]) wildcard expansion ────────────────────────────

    #[test]
    fn hash_atom_not_applicable_for_plain_smirks() {
        let smirks = "[N:1][CH2:2][c:3]>>[N:1].[Br][CH2:2][c:3]";
        assert!(matches!(
            expand_hash_atom_variants(smirks),
            HashAtomExpansion::NotApplicable
        ));
    }

    #[test]
    fn hash_atom_expands_bare_nitrogen_wildcard_into_validated_variants() {
        // Real extracted template (2-anilinopyrimidine-class retro):
        // "any nitrogen" on both ring positions, aromaticity unspecified.
        let smirks = "[#7:2]:[c:1](-[NH:4]-[c:5]):[#7:3]>>Cl-[c:1](:[#7:2]):[#7:3].[NH2:4]-[c:5]";
        match expand_hash_atom_variants(smirks) {
            HashAtomExpansion::Expanded {
                variants,
                total_combinations,
                capped,
            } => {
                // One distinct atom-map per hash atom (map 2, map 3) -- but
                // this template's neighboring-ring context means only the
                // aromatic (lowercase) reading is a real pyrimidine; the
                // aliphatic reading may or may not itself parse. Either
                // way, expansion must not guess -- it must try both and
                // keep only what `parse_reaction` actually accepts.
                assert!(!capped);
                assert_eq!(total_combinations, 4); // 2 atom-maps x 2 readings each
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
                // The aromatic reading must be among the survivors -- this
                // is the chemically-correct one for a pyrimidine ring.
                assert!(
                    variants
                        .iter()
                        .any(|v| v.contains("[n:2]") && v.contains("[n:3]")),
                    "expected an all-aromatic variant among {variants:?}"
                );
            }
            _ => panic!("expected Expanded, got a different outcome"),
        }
    }

    #[test]
    fn hash_atom_same_atom_map_gets_consistent_choice_both_sides() {
        let smirks = "[#7:2]:[c:1]:[c:3]>>Cl-[c:1](:[#7:2]):[c:3]";
        if let HashAtomExpansion::Expanded { variants, .. } = expand_hash_atom_variants(smirks) {
            for v in &variants {
                let is_upper = v.contains("[N:2]");
                let is_lower = v.contains("[n:2]");
                assert!(
                    is_upper ^ is_lower,
                    "atom-map 2 must resolve to exactly one consistent choice per variant: {v}"
                );
            }
        }
    }

    #[test]
    fn hash_atom_bails_on_inconsistent_element_for_same_atom_map() {
        // Synthetic: atom-map 2 is #7 (N) on one side, #8 (O) on the other --
        // internally inconsistent, must not guess a choice.
        let smirks = "[#7:2]-[C:1]>>[#8:2]-[C:1]";
        assert!(matches!(
            expand_hash_atom_variants(smirks),
            HashAtomExpansion::Unexpandable
        ));
    }

    #[test]
    fn hash_atom_bails_on_combined_primitive() {
        // `#7` combined with another primitive in the same bracket --
        // outside what this expansion attempts to rewrite safely.
        let smirks = "[#7;+0:2]-[C:1]>>[N:2]-[C:1]";
        assert!(matches!(
            expand_hash_atom_variants(smirks),
            HashAtomExpansion::Unexpandable
        ));
    }

    #[test]
    fn hash_atom_expansion_is_capped_and_visible_when_combinatorial_space_is_large() {
        // 10 distinct unmapped hash atoms (5 per side, no atom-map to
        // share a choice across) -> 2^10 = 1024 combinations, hugely over
        // the 16-variant cap.
        let smirks = "[#7]-[#8]-[#16]-[#7]-[#8]>>[#7]-[#8]-[#16]-[#7]-[#8]";
        match expand_hash_atom_variants(smirks) {
            HashAtomExpansion::Expanded {
                variants,
                total_combinations,
                capped,
            } => {
                assert_eq!(total_combinations, 1024);
                assert!(capped);
                assert!(variants.len() <= MAX_HASH_ATOM_VARIANTS);
            }
            _ => panic!("expected a capped Expanded outcome"),
        }
    }

    #[test]
    fn load_rules_from_file_hash_atom_variants_share_original_template_id() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("renkin_hash_atom_test_{}.smi", std::process::id()));
        let plain = "[N:1][CH2:2][c:3]>>[N:1].[Br][CH2:2][c:3]";
        let hash_atom =
            "[#7:2]:[c:1](-[NH:4]-[c:5]):[#7:3]>>Cl-[c:1](:[#7:2]):[#7:3].[NH2:4]-[c:5]";
        std::fs::write(&path, format!("{plain}\t10\n{hash_atom}\t167\n")).unwrap();

        let rules = load_rules_from_file(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();

        let plain_rules: Vec<_> = rules.iter().filter(|r| r.smirks == plain).collect();
        assert_eq!(
            plain_rules.len(),
            1,
            "unaffected line must produce exactly one rule"
        );
        assert_eq!(plain_rules[0].name, "extracted_0");
        assert_eq!(plain_rules[0].template_id, template_id_for_smirks(plain));

        let expected_id = template_id_for_smirks(hash_atom);
        let hash_variants: Vec<_> = rules
            .iter()
            .filter(|r| r.template_id == expected_id)
            .collect();
        assert!(
            !hash_variants.is_empty(),
            "hash-atom line must produce at least one surviving variant"
        );
        for r in &hash_variants {
            assert!(!r.smirks.contains('#'));
            assert!(r.name.starts_with("extracted_1_h"));
        }
    }

    #[test]
    fn apply_retro_succeeds_end_to_end_on_hash_atom_template_after_expansion() {
        // Regression test for the real bug this module fixes: today,
        // `apply_retro` on the *original* `[#7:2]`-bearing SMIRKS silently
        // fails (chematic's SMILES-based `run_reactants` can't parse `#7`),
        // returning zero precursors for every molecule -- even one that
        // obviously matches. After expansion, the surviving variant must
        // actually decompose the real target correctly.
        let hash_atom_retro =
            "[#7:2]:[c:1](-[NH:4]-[c:5]):[#7:3]>>Cl-[c:1](:[#7:2]):[#7:3].[NH2:4]-[c:5]";

        // Sanity check the documented bug still reproduces on the raw,
        // unexpanded template -- if chematic ever starts accepting `#N`
        // directly, this assertion (not the fix) should be revisited.
        let target = mol_from_smiles("c1ccc(Nc2ncccn2)cc1").unwrap(); // 2-anilinopyrimidine
        let broken_rule = RetroRule {
            name: "extracted_test".to_string(),
            template_id: template_id_for_smirks(hash_atom_retro),
            smirks: hash_atom_retro.to_string(),
            weight: 1.0,
            required_elements: 0,
        };
        assert!(
            apply_retro(&target, &broken_rule).is_empty(),
            "documents the pre-fix failure mode: the raw #N SMIRKS applied directly must \
             still produce nothing, confirming the fix works via expansion, not by changing \
             apply_retro/run_reactants itself"
        );

        let HashAtomExpansion::Expanded { variants, .. } =
            expand_hash_atom_variants(hash_atom_retro)
        else {
            panic!("expected the real corpus template to expand successfully");
        };
        let aromatic_variant = variants
            .iter()
            .find(|v| v.contains("[n:2]") && v.contains("[n:3]"))
            .expect("aromatic reading must be among the survivors");
        let fixed_rule = RetroRule {
            name: "extracted_test_h0".to_string(),
            template_id: template_id_for_smirks(hash_atom_retro),
            smirks: aromatic_variant.clone(),
            weight: 1.0,
            required_elements: 0,
        };
        let outcomes = apply_retro(&target, &fixed_rule);
        assert!(
            !outcomes.is_empty(),
            "expanded variant must successfully decompose the real target"
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
        SubstituentPreservationCase {
            // The CH2's real -OH substituent must survive, not be discarded
            // when the rule re-declares CH2 (2H) -> CH3 (3H).
            rule_name: "negishi_retro",
            target: "OCc1ccccc1",              // benzyl alcohol
            expected_preserved_fragment: "CO", // methanol
        },
        SubstituentPreservationCase {
            // C:1's extra branch (isopropyl) must survive into the acid fragment.
            rule_name: "claisen_retro",
            target: "CC(C)C(=O)CC(=O)OCC", // ethyl 4-methyl-3-oxopentanoate
            expected_preserved_fragment: "CC(C)C(=O)O", // isobutyric acid
        },
        SubstituentPreservationCase {
            // C:1's aryl substituent must survive into the enol fragment.
            rule_name: "michael_retro",
            target: "c1ccccc1CC(=O)CC", // 1-phenylpentan-2-one-ish chain
            expected_preserved_fragment: "C=C(O)Cc1ccccc1",
        },
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
        SubstituentPreservationCase {
            // The ethyl ketone fragment (not just a bare carbonyl) must survive.
            rule_name: "grignard_addition_retro",
            target: "CCC(O)(C)CC",                   // 3-methylpentan-3-ol
            expected_preserved_fragment: "CCC(C)=O", // butan-2-one
        },
        SubstituentPreservationCase {
            // The ethylamine substituent must survive as a standalone amine,
            // not be discarded when the aryl ring is cut away.
            rule_name: "aryl_amine_retro",
            target: "c1ccccc1NCC",              // N-phenylethylamine
            expected_preserved_fragment: "CCN", // ethylamine
        },
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

        // Expect exactly bromobenzene and benzene. Compare against canonical
        // forms computed at test time (not a hardcoded string) — the exact
        // canonical SMILES chematic emits for a given molecule is an
        // implementation detail that can change between chematic versions
        // (e.g. 0.4.25 wrote "Brc1ccccc1", 0.4.30 writes "c1ccc(cc1)Br" for
        // the same molecule); what must hold is chemical identity, not a
        // specific string layout.
        let bromobenzene_canon = canonical_smiles(&mol_from_smiles("Brc1ccccc1").unwrap());
        let benzene_canon = canonical_smiles(&mol_from_smiles("c1ccccc1").unwrap());
        let has_bromobenzene = all_smiles.contains(&bromobenzene_canon);
        let has_benzene = all_smiles.contains(&benzene_canon);
        assert!(
            has_bromobenzene,
            "expected bromobenzene fragment ({bromobenzene_canon:?}); got {all_smiles:?}"
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
