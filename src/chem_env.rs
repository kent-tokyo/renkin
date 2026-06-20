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
            // Ar-Ar → Ar-Br + Ar-H (retro-Suzuki; one fragment gets halide BB)
            smirks: "[c:1][c:2]>>[c:1]Br.[c:2]",
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
        let env = ChemEnv::in_memory(&["c1ccc(N)cc1", "C(=O)O"]);
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
