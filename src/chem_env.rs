use std::collections::{HashMap, HashSet};
use std::fs;

use anyhow::{Context, Result};
use chematic::chem::standardize::{StandardizeOptions, ZwitterionHandling, standardize};
use chematic::core::{Atom, AtomIdx, BondOrder, Element, MoleculeBuilder};
use chematic::rxn::run_reactants;
use chematic::smarts::{QueryMolecule, find_matches, parse_smarts};
use chematic::smiles::{canonical_smiles, parse};

pub use chematic::core::Molecule;

#[derive(Debug, Clone)]
pub struct RetroRule {
    pub name: &'static str,
    /// SMIRKS in "reactant>>product1.product2" form (retro direction).
    pub smirks: &'static str,
}

/// Building-block library.
///
/// Two-tier storage for scalability:
/// - `canon_set`: canonical-SMILES HashSet used for all lookups (O(1), low memory).
///   Scales to millions of BBs (500k BBs ≈ 12 MB vs 2.8 GB for VF2 QueryMolecules).
/// - `vf2_index`: (atom_count, bond_count) → VF2 QueryMolecule fallback for small
///   sets (DEFAULT_BUILDING_BLOCKS). Kept for correctness when canonical SMILES
///   might diverge due to chematic Bug #14.
///
/// In practice the canonical-SMILES path is used for all lookups; the VF2 index
/// provides a secondary confirmation only when the canon check fails and the VF2
/// index is populated (small in-memory sets).
pub struct ChemEnv {
    /// Canonical SMILES of every BB — primary fast lookup.
    canon_set: HashSet<String>,
    /// VF2 fallback for small sets (populated only when bb_count ≤ VF2_THRESHOLD).
    vf2_index: HashMap<(usize, usize), Vec<QueryMolecule>>,
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
        let mut canon_set: HashSet<String> = HashSet::new();
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

        let mut vf2_index: HashMap<(usize, usize), Vec<QueryMolecule>> = HashMap::new();
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

    /// Check if `mol` is in the building-block library.
    ///
    /// Primary: O(1) canonical-SMILES HashSet lookup with double-pass normalisation.
    /// Double-pass (mol → SMILES string → re-parse → canonical) ensures consistent
    /// canonical form regardless of how the Molecule was constructed (from string vs
    /// from graph manipulation), working around chematic Bug #14.
    ///
    /// Fallback: VF2 subgraph isomorphism (small sets only, bb_count ≤ VF2_THRESHOLD).
    pub fn is_building_block(&self, mol: &Molecule) -> bool {
        // Double-pass: mol → SMILES → re-parse → canonical
        // Molecules from build_sub_molecule() and from parse() can differ in internal
        // representation and produce different canonical SMILES on the first pass.
        // Re-parsing normalises both to the same canonical form.
        let smiles_str = canonical_smiles(mol);
        let canon = match parse(&smiles_str) {
            Ok(reparsed) => canonical_smiles(&reparsed),
            Err(_) => smiles_str,
        };
        if self.canon_set.contains(&canon) {
            return true;
        }
        // VF2 fallback for small sets (catches edge cases Bug #14 still misses)
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
    let mut visited = HashSet::new();
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
) -> HashSet<AtomIdx> {
    let mut visited = HashSet::new();
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
fn build_sub_molecule(mol: &Molecule, atoms: &HashSet<AtomIdx>) -> Option<Molecule> {
    let mut builder = MoleculeBuilder::new();
    let mut idx_map: HashMap<AtomIdx, AtomIdx> = HashMap::new();

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
    atoms: &HashSet<AtomIdx>,
    cut_atom: AtomIdx,
) -> Option<Molecule> {
    let mut builder = MoleculeBuilder::new();
    let mut idx_map: HashMap<AtomIdx, AtomIdx> = HashMap::new();

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

/// Graph-based retro-Suzuki: cleave every Ar–Ar bridge bond and return
/// [Ar-Br, Ar'] and [Ar, Ar'-Br] precursor sets.
fn biaryl_cleavage(mol: &Molecule) -> Vec<Vec<PrecursorMol>> {
    let mut results: Vec<Vec<PrecursorMol>> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

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
    let mut seen: HashSet<String> = HashSet::new();

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
    atoms: &HashSet<AtomIdx>,
    cut_atom: AtomIdx,
) -> Option<Molecule> {
    let mut builder = MoleculeBuilder::new();
    let mut idx_map: HashMap<AtomIdx, AtomIdx> = HashMap::new();

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
        return match rule.name {
            "suzuki_retro" => biaryl_cleavage(mol),
            "amide_cleavage" => amide_cleavage(mol),
            "boc_deprotection_retro" => boc_deprotection(mol),
            "cbz_deprotection_retro" => cbz_deprotection(mol),
            _ => vec![],
        };
    }
    run_reactants(rule.smirks, &[mol])
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
            let smi_check = canonical_smiles(&std_mol);
            let has_aromatic = smi_check
                .chars()
                .any(|c| matches!(c, 'c' | 'n' | 'o' | 's' | 'p'));
            let has_ring = smi_check.chars().any(|c| c.is_ascii_digit());
            if has_aromatic && !has_ring {
                return None;
            }
            let smi = to_canonical(&std_mol);
            let final_mol = parse(&smi).ok()?;
            Some(PrecursorMol {
                smiles: smi,
                mol: final_mol,
            })
        })
        .collect()
}

pub fn default_rules() -> Vec<RetroRule> {
    vec![
        // ── Acyl disconnections ──────────────────────────────────────────
        RetroRule {
            name: "ester_cleavage",
            // Ester C(=O)-O → carboxylic acid + alcohol/phenol
            smirks: "[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]",
        },
        RetroRule {
            name: "amide_cleavage",
            // Graph-based: dispatched in apply_retro, not run_reactants
            // (SMIRKS-based version had BFS-leakage producing spurious fragments)
            smirks: "",
        },
        RetroRule {
            name: "friedel_crafts_acylation_retro",
            // Ar-C(=O)R → Ar-H + R-C(=O)Cl (Friedel-Crafts retro)
            smirks: "[c:1][C:2](=[O:3])>>[c:1].[C:2](=[O:3])Cl",
        },
        // ── Aryl C-heteroatom disconnections ────────────────────────────
        RetroRule {
            name: "aryl_carboxylation_retro",
            // Ar-COOH → Ar-H + HCOOH (retro-Kolbe-Schmitt / decarboxylation)
            smirks: "[c:1][C:2](=O)O>>[c:1].[C:2](=O)O",
        },
        RetroRule {
            name: "aryl_amine_retro",
            // Ar-N → Ar-H + amine (retro-SNAr / retro-Chan-Lam)
            smirks: "[c:1][N:2]>>[c:1].[N:2]",
        },
        RetroRule {
            name: "buchwald_hartwig_retro",
            // Ar-N → Ar-Br + amine (retro-Buchwald-Hartwig; gives halide BB)
            smirks: "[c:1][N:2]>>[c:1]Br.[N:2]",
        },
        RetroRule {
            name: "aryl_ether_retro",
            // Ar-O → Ar-OH + leaving fragment (retro-Ullmann ether synthesis)
            smirks: "[c:1][O:2]>>[c:1]O.[O:2]",
        },
        // ── Aryl C-halide disconnections ────────────────────────────────
        RetroRule {
            name: "aryl_chloride_retro",
            // Ar-Cl → Ar-H + HCl (retro-SNAr or retro-Pd C-Cl activation)
            smirks: "[c:1][Cl]>>[c:1]",
        },
        RetroRule {
            name: "aryl_iodide_retro",
            // Ar-I → Ar-H (retro-Pd/Cu C-I; iodides are activated leaving groups)
            smirks: "[c:1][I]>>[c:1]",
        },
        RetroRule {
            name: "aryl_fluoride_snAr_retro",
            // Ar-F → Ar-H (retro-SNAr; fluorine is best SNAr leaving group)
            smirks: "[c:1][F]>>[c:1]",
        },
        RetroRule {
            name: "aryl_chloride_to_bromide",
            // Ar-Cl → Ar-Br (halogen exchange retro; Ar-Br is often a cheaper BB)
            smirks: "[c:1][Cl]>>[c:1][Br]",
        },
        // ── Aryl C-C disconnections ──────────────────────────────────────
        RetroRule {
            name: "suzuki_retro",
            // Graph-based: find Ar-Ar bridge bonds and split into Ar-Br + Ar.
            // smirks is empty; apply_retro dispatches to biaryl_cleavage().
            smirks: "",
        },
        RetroRule {
            name: "heck_retro",
            // Ar-CH=CH-R → Ar-Br + CH2=CH-R (retro-Heck, internal alkene)
            smirks: "[c:1][CH:2]=[CH:3]>>[c:1][Br].[CH2:2]=[CH:3]",
        },
        RetroRule {
            name: "heck_retro_terminal",
            // Ar-CH=CH2 → Ar-Br + CH2=CH2 (retro-Heck, terminal alkene / styrene)
            smirks: "[c:1][CH:2]=[CH2:3]>>[c:1][Br].[CH2:2]=[CH2:3]",
        },
        RetroRule {
            name: "negishi_retro",
            // Ar-alkyl → Ar-Br + alkyl (retro-Negishi; Pd-catalyzed C-C)
            smirks: "[c:1][CH2:2]>>[c:1][Br].[CH3:2]",
        },
        // ── Aliphatic C-C disconnections ─────────────────────────────────
        RetroRule {
            name: "cc_single_cleavage",
            // Generic aliphatic C-C bond cleavage
            smirks: "[C:1][C:2]>>[C:1].[C:2]",
        },
        RetroRule {
            name: "wittig_retro",
            // Alkene → two carbonyls (retro-Wittig / retro-HWE)
            smirks: "[C:1]=[C:2]>>[C:1]=O.[C:2]=O",
        },
        // ── C-N disconnections ───────────────────────────────────────────
        RetroRule {
            name: "reductive_amination_retro",
            // C-N → C=O + amine (retro-reductive amination; aliphatic C only)
            smirks: "[C:1][N:2]>>[C:1]=O.[N:2]",
        },
        RetroRule {
            name: "cn_aliphatic_cleavage",
            // Generic aliphatic C-N bond cleavage (N-alkylation retro)
            smirks: "[C:1][N:2]>>[C:1].[N:2]",
        },
        // ── C-O disconnections ───────────────────────────────────────────
        RetroRule {
            name: "co_aliphatic_cleavage",
            // Generic aliphatic C-O bond cleavage (ether / O-alkylation retro)
            smirks: "[C:1][O:2]>>[C:1].[O:2]",
        },
        RetroRule {
            name: "alcohol_oxidation_retro",
            // Alcohol → ketone/aldehyde (retro-reduction; converts C-OH to C=O)
            smirks: "[C:1][OH:2]>>[C:1]=O",
        },
        // ── Sonogashira coupling ─────────────────────────────────────────────
        RetroRule {
            name: "sonogashira_retro",
            // Ar-C≡C-R → Ar-Br + HC≡C-R (retro-Sonogashira, Pd/Cu catalysis)
            smirks: "[c:1][C:2]#[C:3]>>[c:1]Br.[C:2]#[C:3]",
        },
        // ── Sulfonamide disconnection ────────────────────────────────────────
        RetroRule {
            name: "sulfonamide_retro",
            // Ar-SO2-NHR → Ar-SO2Cl + HNR (sulfonyl chloride + amine)
            smirks: "[S:1](=O)(=O)[N:2]>>[S:1](=O)(=O)Cl.[N:2]",
        },
        // ── N-protection / deprotection ──────────────────────────────────────
        RetroRule {
            name: "boc_deprotection_retro",
            // N-Boc → N-H (deprotect: TFA removes Boc). Graph-based to avoid leakage.
            smirks: "",
        },
        // ── N-alkylation (more specific than cn_aliphatic_cleavage) ──────────
        RetroRule {
            name: "n_benzylation_retro",
            // N-CH2Ar → N-H + BrCH2Ar (N-benzyl retro)
            smirks: "[N:1][CH2:2][c:3]>>[N:1].[Br][CH2:2][c:3]",
        },
        // ── Grignard / organolithium retro ───────────────────────────────────
        RetroRule {
            name: "grignard_addition_retro",
            // Tertiary alcohol → ketone + R-MgBr (retro-Grignard)
            smirks: "[C:1]([OH:2])([C:3])[C:4]>>[C:1](=O)[C:3].[C:4]",
        },
        // ── Claisen / Dieckmann condensation ────────────────────────────────
        RetroRule {
            name: "claisen_retro",
            // β-ketoester → ester + ester (retro-Claisen condensation)
            smirks: "[C:1](=O)[CH2:2][C:3](=O)[O:4]>>[C:1](=O)O.[C:2]=[C:3][O:4]",
        },
        // ── Michael addition retro ───────────────────────────────────────────
        RetroRule {
            name: "michael_retro",
            // R-CH2-C(=O)R' ← CH2=C(=O)R' + H (retro-1,4-addition at α)
            smirks: "[C:1][CH2:2][C:3]=[O:4]>>[C:1].[CH2:2]=[C:3][OH:4]",
        },
        // ── Acyl chloride as electrophile source ─────────────────────────────
        RetroRule {
            name: "acyl_chloride_from_acid",
            // Acid chloride → carboxylic acid (SOCl2 activation retro)
            smirks: "[C:1](=[O:2])Cl>>[C:1](=[O:2])O",
        },
        // ── N-formylation / N-acylation (Cbz retro) ─────────────────────────
        RetroRule {
            name: "cbz_deprotection_retro",
            // N-Cbz → N-H (hydrogenolysis retro, graph-based)
            smirks: "",
        },
    ]
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
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

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
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

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
        let rule = RetroRule {
            name: "ester_cleavage",
            smirks: "[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]",
        };
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty(), "ester_cleavage must match aspirin");
    }

    #[test]
    fn aromatic_ring_fragment_filter() {
        use chematic::chem::aromatic_ring_count;
        // Open-chain aromatic fragments (BFS leakage, L4) must be discarded.
        let mol = mol_from_smiles("c1ccc(N)cc1C(=O)O").unwrap();
        let rule = RetroRule {
            name: "aryl_carboxylation_retro",
            smirks: "[c:1][C:2](=O)O>>[c:1].[C:2](=O)O",
        };
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
        };
        let routes = find_routes("c1ccc(-c2ccncc2)cc1", &env, &rules, &config)
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
        let rule = RetroRule {
            name: "aryl_carboxylation_retro",
            smirks: "[c:1][C:2](=O)O>>[c:1].[C:2](=O)O",
        };
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty());
    }

    #[test]
    fn suzuki_retro_biphenyl_gives_bromobenzene_and_benzene() {
        let mol = mol_from_smiles("c1ccc(-c2ccccc2)cc1").unwrap();
        let rule = RetroRule {
            name: "suzuki_retro",
            smirks: "",
        };
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
        };
        let routes = find_routes("c1ccc(-c2ccccc2)cc1", &env, &rules, &cfg).unwrap();
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
        };
        let routes = find_routes("Fc1ccc(-c2ccccc2)cc1", &env, &rules, &cfg).unwrap();
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
        };
        let routes = find_routes("c1ccc(-c2ccccc2)cc1", &env, &rules, &cfg).unwrap();
        assert!(
            !routes.is_empty(),
            "biphenyl must be solvable with DEFAULT_BUILDING_BLOCKS"
        );
    }

    #[test]
    fn amide_cleavage_paracetamol() {
        // Verify amide_cleavage rule fires on paracetamol.
        let mol = mol_from_smiles("CC(=O)Nc1ccc(O)cc1").unwrap();
        let rule = RetroRule {
            name: "amide_cleavage",
            smirks: "[C:1](=[O:2])[N:3]>>[C:1](=[O:2])O.[N:3]",
        };
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
            let routes = find_routes(smiles, &env, &rules, &cfg).unwrap();
            assert!(
                !routes.is_empty(),
                "{name} ({smiles}) must be solvable with DEFAULT_BUILDING_BLOCKS"
            );
        }
    }

    #[test]
    fn wittig_retro_cleaves_alkene() {
        let mol = mol_from_smiles("C=C").unwrap(); // ethylene
        let rule = RetroRule {
            name: "wittig_retro",
            smirks: "[C:1]=[C:2]>>[C:1]=O.[C:2]=O",
        };
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
        let rule = RetroRule {
            name: "friedel_crafts_acylation_retro",
            smirks: "[c:1][C:2](=[O:3])>>[c:1].[C:2](=[O:3])Cl",
        };
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
        let rule = RetroRule {
            name: "heck_retro_terminal",
            smirks: "[c:1][CH:2]=[CH2:3]>>[c:1][Br].[CH2:2]=[CH2:3]",
        };
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
        // chematic may serialise ethylene as "C=C" or "[CH2]=[CH2]" depending on
        // whether the Molecule was constructed with implicit or explicit H counts.
        assert!(
            flat.iter().any(|s| s == "C=C" || s == "[CH2]=[CH2]"),
            "products must include ethylene; got {flat:?}"
        );
    }

    #[test]
    fn heck_retro_internal_on_stilbene() {
        // (E)-stilbene: c1ccccc1/C=C/c1ccccc1
        let mol = mol_from_smiles("C(=Cc1ccccc1)c1ccccc1").unwrap();
        let rule = RetroRule {
            name: "heck_retro",
            smirks: "[c:1][CH:2]=[CH:3]>>[c:1][Br].[CH2:2]=[CH:3]",
        };
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
        let rule = RetroRule {
            name: "negishi_retro",
            smirks: "[c:1][CH2:2]>>[c:1][Br].[CH3:2]",
        };
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
        let rule = RetroRule {
            name: "alcohol_oxidation_retro",
            smirks: "[C:1][OH:2]>>[C:1]=O",
        };
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
        let rule = RetroRule {
            name: "aryl_chloride_retro",
            smirks: "[c:1][Cl]>>[c:1]",
        };
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
        let rule = RetroRule {
            name: "amide_cleavage",
            smirks: "",
        };
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
        let rule = RetroRule {
            name: "reductive_amination_retro",
            smirks: "[C:1][N:2]>>[C:1]=O.[N:2]",
        };
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
#[ignore]
fn debug_canonical_smiles_consistency() {
    // Check if chematic gives consistent canonical SMILES
    // for the same molecule represented in different input forms.
    let pairs = [
        ("Nc1ccccc1", "c1ccc(N)cc1", "aniline"),
        ("Oc1ccccc1", "c1ccc(O)cc1", "phenol"),
        ("c1ccccc1", "C1=CC=CC=C1", "benzene"),
        ("Brc1ccccc1", "c1ccc(Br)cc1", "bromobenzene"),
        ("CC(=O)O", "OC(C)=O", "acetic acid"),
    ];
    for (s1, s2, name) in pairs {
        let c1 = canonical_smiles(&parse(s1).unwrap());
        let c2 = canonical_smiles(&parse(s2).unwrap());
        let dp1 = canonical_smiles(&parse(&c1).unwrap());
        let dp2 = canonical_smiles(&parse(&c2).unwrap());
        eprintln!(
            "{}: '{}' → '{}' (2-pass '{}'), '{}' → '{}' (2-pass '{}'), match={}",
            name,
            s1,
            c1,
            dp1,
            s2,
            c2,
            dp2,
            dp1 == dp2
        );
    }
}
