use std::collections::{HashMap, HashSet};
use std::fs;

use anyhow::{Context, Result};
use chematic::chem::standardize::{StandardizeOptions, ZwitterionHandling, standardize};
use chematic::chem::aromatic_ring_count;
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

/// One entry in the building-block library.
struct BbEntry {
    query: QueryMolecule,
}

/// Building-block library indexed by (atom_count, bond_count) for O(1) candidate
/// pre-filtering. With millions of entries this reduces VF2 calls to only the
/// molecules that share the same heavy-atom / bond count as the query.
pub struct ChemEnv {
    building_blocks: HashMap<(usize, usize), Vec<BbEntry>>,
    bb_count: usize,
}

impl ChemEnv {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read building blocks from {path}"))?;
        Ok(Self::from_entries(Self::parse_smi_content(&content)))
    }

    pub fn in_memory(smiles_list: &[&str]) -> Self {
        let entries: Vec<_> = smiles_list
            .iter()
            .filter_map(|s| Self::smiles_to_entry(s))
            .collect();
        Self::from_entries(entries)
    }

    fn from_entries(entries: Vec<(usize, usize, BbEntry)>) -> Self {
        let bb_count = entries.len();
        let mut building_blocks: HashMap<(usize, usize), Vec<BbEntry>> = HashMap::new();
        for (n_atoms, n_bonds, entry) in entries {
            building_blocks.entry((n_atoms, n_bonds)).or_default().push(entry);
        }
        Self { building_blocks, bb_count }
    }

    fn parse_smi_content(content: &str) -> Vec<(usize, usize, BbEntry)> {
        content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|line| {
                let smiles = line.split_whitespace().next()?;
                Self::smiles_to_entry(smiles)
            })
            .collect()
    }

    fn smiles_to_entry(smiles: &str) -> Option<(usize, usize, BbEntry)> {
        let mol = parse(smiles).ok()?;
        let query = parse_smarts(smiles).ok()?;
        Some((mol.atom_count(), mol.bonds().count(), BbEntry { query }))
    }

    /// Number of building blocks in the library.
    pub fn bb_count(&self) -> usize {
        self.bb_count
    }

    /// Check if `mol` is identical to any building block using VF2 isomorphism.
    /// Pre-filtered by (atom_count, bond_count) → O(1) HashMap lookup before VF2.
    pub fn is_building_block(&self, mol: &Molecule) -> bool {
        let key = (mol.atom_count(), mol.bonds().count());
        let Some(candidates) = self.building_blocks.get(&key) else { return false; };
        let n_atoms = mol.atom_count();
        candidates.iter().any(|bb| {
            let matches = find_matches(&bb.query, mol);
            matches.iter().any(|m| m.len() == n_atoms)
        })
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
            if cur == a && neighbor == b { continue; }
            if visited.insert(neighbor) {
                stack.push(neighbor);
            }
        }
    }
    !visited.contains(&b)
}

/// Collect all atoms reachable from `start` when the bond (bridge_a, bridge_b) is removed.
fn get_component(mol: &Molecule, start: AtomIdx, bridge_a: AtomIdx, bridge_b: AtomIdx) -> HashSet<AtomIdx> {
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
        if !atom_a.aromatic || atom_a.element != Element::C { continue; }
        if !atom_b.aromatic || atom_b.element != Element::C { continue; }

        // Must be a bridge bond (not inside any ring)
        if !is_bridge_bond(mol, a, b) { continue; }

        let comp_a = get_component(mol, a, a, b);
        let comp_b = get_component(mol, b, a, b);

        // Generate both orientations: which ring gets Br
        for (comp_br, cut, comp_plain) in [
            (&comp_a, a, &comp_b),
            (&comp_b, b, &comp_a),
        ] {
            let Some(frag_br)    = build_sub_molecule_with_br(mol, comp_br, cut) else { continue };
            let Some(frag_plain) = build_sub_molecule(mol, comp_plain)           else { continue };

            let precs_br    = split_fragments(&frag_br);
            let precs_plain = split_fragments(&frag_plain);
            if precs_br.is_empty() || precs_plain.is_empty() { continue; }

            // De-duplicate identical orientations (e.g. symmetric biaryls)
            let mut key_parts: Vec<&str> = precs_br.iter()
                .chain(precs_plain.iter())
                .map(|p| p.smiles.as_str())
                .collect();
            key_parts.sort_unstable();
            let key = key_parts.join("|");
            if !seen.insert(key) { continue; }

            let mut prec_set = precs_br;
            prec_set.extend(precs_plain);
            results.push(prec_set);
        }
    }
    results
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
            // Reject fragments that have aromatic atoms but no rings —
            // these are open-chain aromatic chains produced by BFS leakage.
            let has_aromatic = canonical_smiles(&std_mol)
                .chars()
                .any(|c| matches!(c, 'c' | 'n' | 'o' | 's' | 'p'));
            if has_aromatic && aromatic_ring_count(&std_mol) == 0 {
                return None;
            }
            let smi = to_canonical(&std_mol);
            let final_mol = parse(&smi).ok()?;
            Some(PrecursorMol { smiles: smi, mol: final_mol })
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
            // Amide C(=O)-N → carboxylic acid + amine
            smirks: "[C:1](=[O:2])[N:3]>>[C:1](=[O:2])O.[N:3]",
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
        // ── Aryl C-C disconnections ──────────────────────────────────────
        RetroRule {
            name: "suzuki_retro",
            // Graph-based: find Ar-Ar bridge bonds and split into Ar-Br + Ar.
            // smirks is empty; apply_retro dispatches to biaryl_cleavage().
            smirks: "",
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
    ]
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
        assert!(env.is_building_block(&mol), "acetic acid should be a building block");
    }

    #[test]
    fn non_building_block_rejected() {
        let env = env_aspirin_bbs();
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        assert!(!env.is_building_block(&mol), "aspirin should not be a building block");
    }

    #[test]
    fn building_block_canonical_form_variant() {
        // VF2 must match even when canonical SMILES differ (L2 in lessons.md).
        let env = ChemEnv::in_memory(&["CC(=O)O"]);
        let mol = mol_from_smiles("OC(C)=O").unwrap();   // different SMILES, same molecule
        assert!(env.is_building_block(&mol), "OC(C)=O is the same as CC(=O)O");
    }

    #[test]
    fn benzoic_acid_variant_matches() {
        // Different SMILES representations of benzoic acid must match via VF2 (L2).
        let env = ChemEnv::in_memory(&["c1ccccc1C(=O)O"]);
        let mol = mol_from_smiles("c1c(C(=O)O)cccc1").unwrap();
        assert!(env.is_building_block(&mol), "c1c(C(=O)O)cccc1 is benzoic acid");
    }

    #[test]
    fn ester_cleavage_fires_on_aspirin() {
        let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O").unwrap();
        let rule = RetroRule { name: "ester_cleavage", smirks: "[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]" };
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty(), "ester_cleavage must match aspirin");
    }

    #[test]
    fn aromatic_ring_fragment_filter() {
        // Open-chain aromatic fragments (BFS leakage, L4) must be discarded.
        let mol = mol_from_smiles("c1ccc(N)cc1C(=O)O").unwrap();
        let rule = RetroRule { name: "aryl_carboxylation_retro", smirks: "[c:1][C:2](=O)O>>[c:1].[C:2](=O)O" };
        let results = apply_retro(&mol, &rule);
        // All returned fragments must have rings if they contain aromatic atoms.
        for precursor_set in &results {
            for p in precursor_set {
                let smi = &p.smiles;
                let has_lowercase = smi.chars().any(|c| matches!(c, 'c'|'n'|'o'|'s'|'p'));
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
    fn degenerate_route_not_in_precursors() {
        // apply_retro itself does not filter self-referencing; the search does.
        // This test just verifies that for anthranilic acid the aryl_carboxylation
        // rule returns aniline-like and acid-like fragments without crashing.
        let mol = mol_from_smiles("c1ccc(N)cc1C(=O)O").unwrap();
        let rule = RetroRule { name: "aryl_carboxylation_retro", smirks: "[c:1][C:2](=O)O>>[c:1].[C:2](=O)O" };
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty());
    }

    #[test]
    fn suzuki_retro_biphenyl_gives_bromobenzene_and_benzene() {
        let mol = mol_from_smiles("c1ccc(-c2ccccc2)cc1").unwrap();
        let rule = RetroRule { name: "suzuki_retro", smirks: "" };
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty(), "suzuki_retro must find at least one biaryl disconnection");

        let all_smiles: Vec<String> =
            results.iter().flat_map(|set| set.iter().map(|p| p.smiles.clone())).collect();

        // Expect exactly bromobenzene and benzene (in some canonical form)
        let has_bromobenzene = all_smiles.iter().any(|s| s.contains("Br") && s.contains("c1ccccc1"));
        let has_benzene      = all_smiles.iter().any(|s| s == "c1ccccc1");
        assert!(has_bromobenzene, "expected bromobenzene fragment; got {all_smiles:?}");
        assert!(has_benzene,      "expected benzene fragment; got {all_smiles:?}");
    }

    #[test]
    fn suzuki_retro_biphenyl_solvable_with_bb() {
        // End-to-end: the engine must resolve biphenyl given bromobenzene + benzene as BBs.
        use crate::search::{SearchConfig, find_routes};
        let env = ChemEnv::in_memory(&["Brc1ccccc1", "c1ccccc1"]);
        let rules = default_rules();
        let cfg = SearchConfig { max_depth: 2, max_routes: 3, beam_width: 0 };
        let routes = find_routes("c1ccc(-c2ccccc2)cc1", &env, &rules, &cfg).unwrap();
        assert!(!routes.is_empty(), "biphenyl must be solvable with Br-PhH + PhH BBs");
        assert!(routes.iter().any(|r| r.depth == 1), "should need only 1 step");
    }

    #[test]
    fn suzuki_retro_4_fluorobiphenyl_solvable() {
        use crate::search::{SearchConfig, find_routes};
        let env = ChemEnv::load("data/building_blocks.smi")
            .unwrap_or_else(|_| ChemEnv::in_memory(&["Brc1ccccc1", "Brc1ccc(F)cc1", "c1ccccc1"]));
        let rules = default_rules();
        let cfg = SearchConfig { max_depth: 2, max_routes: 3, beam_width: 0 };
        let routes = find_routes("Fc1ccc(-c2ccccc2)cc1", &env, &rules, &cfg).unwrap();
        assert!(!routes.is_empty(), "4-fluorobiphenyl must be solvable");
    }

    #[test]
    fn wittig_retro_cleaves_alkene() {
        let mol = mol_from_smiles("C=C").unwrap(); // ethylene
        let rule = RetroRule { name: "wittig_retro", smirks: "[C:1]=[C:2]>>[C:1]=O.[C:2]=O" };
        let results = apply_retro(&mol, &rule);
        assert!(!results.is_empty(), "wittig_retro must match ethylene");
        // Products must contain oxygen atoms (carbonyls — canonical form may be C=O or O=C).
        let smiles: Vec<_> = results[0].iter().map(|p| p.smiles.as_str()).collect();
        assert!(
            smiles.iter().any(|s| s.contains('O')),
            "products should contain oxygen; got {smiles:?}"
        );
    }
}
