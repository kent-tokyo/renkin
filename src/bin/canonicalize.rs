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
/// `--clear-atom-maps`: structurally clear each atom's atom-map number
/// (`chem_env::clear_atom_maps`) before canonicalizing -- input SMILES may
/// be atom-mapped (`[CH3:1]O`). Never strip atom maps by regex/string
/// manipulation on SMILES text upstream of this binary: `:` is also SMILES
/// bond syntax, so a text-level strip can corrupt a ring-closure digit that
/// happens to follow an explicit bond symbol (see
/// `chem_env::clear_atom_maps_tests::explicit_colon_bond_with_ring_closure_digit_is_not_corrupted`
/// for a concrete case). Without this flag, atom maps in the input are
/// preserved in the output canonical SMILES, unchanged from prior behavior.
///
/// Usage:
///   cargo build --release --bin renkin-canonicalize
///   echo "CC(=O)Oc1ccccc1C(=O)O" | ./target/release/renkin-canonicalize
///   echo "[CH3:1]O" | ./target/release/renkin-canonicalize --clear-atom-maps
fn main() {
    use renkin::chem_env::{clear_atom_maps, mol_from_smiles, to_canonical};
    use std::io::{self, Write};

    let clear_maps = std::env::args().any(|a| a == "--clear-atom-maps");

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    let mut input = stdin.lock();
    loop {
        let smiles = match renkin::io_limits::read_bounded_line(&mut input, "canonicalize stdin") {
            Ok(Some(s)) => s,
            Ok(None) => break,
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
                let mol = if clear_maps {
                    clear_atom_maps(&mol)
                } else {
                    mol
                };
                writeln!(out, "{}", to_canonical(&mol)).ok();
            }
            Err(_) => {
                writeln!(out, "ERR").ok();
            }
        }
    }
}
