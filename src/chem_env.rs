use std::fs;

use anyhow::{Context, Result};
use chematic::chem::standardize::{StandardizeOptions, ZwitterionHandling, standardize};
use chematic::chem::aromatic_ring_count;
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
    atom_count: usize,
    bond_count: usize,
}

pub struct ChemEnv {
    building_blocks: Vec<BbEntry>,
}

impl ChemEnv {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read building blocks from {path}"))?;
        let building_blocks = Self::parse_smi_content(&content);
        Ok(Self { building_blocks })
    }

    pub fn in_memory(smiles_list: &[&str]) -> Self {
        let building_blocks = smiles_list
            .iter()
            .filter_map(|s| Self::smiles_to_entry(s))
            .collect();
        Self { building_blocks }
    }

    fn parse_smi_content(content: &str) -> Vec<BbEntry> {
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

    fn smiles_to_entry(smiles: &str) -> Option<BbEntry> {
        let mol = parse(smiles).ok()?;
        let query = parse_smarts(smiles).ok()?;
        Some(BbEntry {
            atom_count: mol.atom_count(),
            bond_count: mol.bonds().count(),
            query,
        })
    }

    /// Check if `mol` is identical to any building block using VF2 isomorphism.
    pub fn is_building_block(&self, mol: &Molecule) -> bool {
        let n_atoms = mol.atom_count();
        let n_bonds = mol.bonds().count();
        self.building_blocks
            .iter()
            .filter(|bb| bb.atom_count == n_atoms && bb.bond_count == n_bonds)
            .any(|bb| {
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

/// Apply a single retro-rule to a molecule.
/// Returns all possible precursor sets as (canonical_smiles, Molecule) pairs.
///
/// chematic's run_reactants seeds BFS from ALL mapped atoms across all product
/// templates, so each product Molecule may contain disconnected fragments.
/// We split by '.' in canonical SMILES to get clean independent precursors,
/// then standardize to normalize explicit H counts.
pub fn apply_retro(mol: &Molecule, rule: &RetroRule) -> Vec<Vec<PrecursorMol>> {
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
        RetroRule {
            name: "ester_cleavage",
            // Ester C(=O)-O → carboxylic acid + alcohol/phenol fragment
            smirks: "[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]",
        },
        RetroRule {
            name: "amide_cleavage",
            // Amide C(=O)-N → carboxylic acid + amine
            smirks: "[C:1](=[O:2])[N:3]>>[C:1](=[O:2])O.[N:3]",
        },
        RetroRule {
            name: "aryl_carboxylation_retro",
            // Retro-Kolbe-Schmitt / decarboxylation: Ar-COOH → Ar-H + HCOOH surrogate
            // Disconnects the bond between aromatic C and carboxyl C
            smirks: "[c:1][C:2](=O)O>>[c:1].[C:2](=O)O",
        },
        RetroRule {
            name: "aryl_amine_retro",
            // Retro-amination: Ar-NH2 → Ar-H + NH3 surrogate
            // Disconnects the bond between aromatic C and amine N
            smirks: "[c:1][N:2]>>[c:1].[N:2]",
        },
        RetroRule {
            name: "cc_single_cleavage",
            // Generic aliphatic C-C bond cleavage
            smirks: "[C:1][C:2]>>[C:1].[C:2]",
        },
    ]
}
