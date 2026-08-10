#![forbid(unsafe_code)]

/// Batch SMILES canonicalization using RENKIN's own canonicalizer
/// (`chem_env::to_canonical`), the same function `propose_one_step`/
/// `merge_into_candidates` use to produce `precursor_smiles` strings in the
/// candidate pool. Ground-truth label sources (e.g. raw reaction datasets)
/// must be canonicalized through this exact path -- not a third-party
/// toolkit's canonical form -- or exact-string label matching in
/// `train_reranker.py::label_and_split_rows` will silently mismatch.
///
/// Reads SMILES from stdin (one per line), writes canonical SMILES to
/// stdout (one line per input line, same order). Outputs "ERR" for
/// unparseable input so line-alignment with the input is always preserved.
///
/// Usage:
///   cargo build --release --bin renkin-canonicalize
///   echo "CC(=O)Oc1ccccc1C(=O)O" | ./target/release/renkin-canonicalize
fn main() {
    use renkin::chem_env::{mol_from_smiles, to_canonical};
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let smiles = match line {
            Ok(s) => s,
            Err(_) => {
                writeln!(out, "ERR").ok();
                continue;
            }
        };
        let smiles = smiles.trim();
        if smiles.is_empty() {
            writeln!(out, "ERR").ok();
            continue;
        }
        match mol_from_smiles(smiles) {
            Ok(mol) => {
                writeln!(out, "{}", to_canonical(&mol)).ok();
            }
            Err(_) => {
                writeln!(out, "ERR").ok();
            }
        }
    }
}
