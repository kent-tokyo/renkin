//! Prints machine-readable facts derived live from the current code/data, so
//! CI can cross-check docs/README against reality instead of relying on
//! hardcoded numbers that silently drift (see .github/workflows/ci.yml).
use renkin::chem_env::{default_rules, mol_from_smiles, to_canonical};

fn main() {
    let rules = default_rules();
    println!("HAND_CRAFTED_RULE_COUNT={}", rules.len());

    let content = std::fs::read_to_string("data/building_blocks.smi")
        .expect("data/building_blocks.smi must be readable");
    let mut canon_set = std::collections::HashSet::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let smiles = line.split_whitespace().next().unwrap_or("");
        if let Ok(mol) = mol_from_smiles(smiles) {
            canon_set.insert(to_canonical(&mol));
        }
    }
    println!("BUILDING_BLOCK_COUNT={}", canon_set.len());
}
