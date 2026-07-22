//! Tiny helper for Phase 32's per-target screening runner: prints canonical
//! SMILES for each input line, tab-separated with the original, so target
//! identity hashes are stable across differently-written-but-identical
//! inputs. Reads one SMILES per line from stdin.
use chematic::smiles::canonical_smiles;
use renkin::chem_env::mol_from_smiles;
use std::io::Read;

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    for line in input.lines() {
        let smiles = line.trim();
        if smiles.is_empty() {
            continue;
        }
        match mol_from_smiles(smiles) {
            Ok(mol) => println!("{smiles}\t{}", canonical_smiles(&mol)),
            Err(_) => println!("{smiles}\tPARSE_ERROR"),
        }
    }
}
