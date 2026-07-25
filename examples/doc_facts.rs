//! Prints machine-readable facts derived live from the current code/data, so
//! CI can cross-check docs/README against reality instead of relying on
//! hardcoded numbers that silently drift (see .github/workflows/ci.yml).
use renkin::chem_env::{default_rules, mol_from_smiles, to_canonical};

fn unique_canonical_count(smiles_iter: impl Iterator<Item = String>) -> usize {
    let mut canon_set = std::collections::HashSet::new();
    for smiles in smiles_iter {
        if let Ok(mol) = mol_from_smiles(&smiles) {
            canon_set.insert(to_canonical(&mol));
        }
    }
    canon_set.len()
}

fn main() {
    let rules = default_rules();
    println!("HAND_CRAFTED_RULE_COUNT={}", rules.len());

    // data/building_blocks.smi: the full curated stock, used by CLI/Python
    // only when found relative to the current working directory.
    let content = std::fs::read_to_string("data/building_blocks.smi")
        .expect("data/building_blocks.smi must be readable");
    let file_count = unique_canonical_count(content.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        line.split_whitespace().next().map(str::to_string)
    }));
    println!("BUILDING_BLOCK_FILE_COUNT={file_count}");

    // DEFAULT_BUILDING_BLOCKS: the compiled-in fallback used by CLI/Python
    // when the file above isn't found, and always used by WASM (which can't
    // read a filesystem path at all).
    let fallback_count = unique_canonical_count(
        renkin::DEFAULT_BUILDING_BLOCKS
            .iter()
            .map(|s| s.to_string()),
    );
    println!("BUILDING_BLOCK_FALLBACK_COUNT={fallback_count}");
}
