#![forbid(unsafe_code)]

/// Batch ECFP4 fingerprint computation using chematic (for training data alignment).
///
/// Reads SMILES from stdin (one per line), outputs space-separated set-bit indices
/// to stdout (one line per SMILES). Outputs "ERR" for invalid/unparseable SMILES.
///
/// Usage (from training script):
///   cargo build --release --features nn-scoring --bin renkin-fp
///   echo "CC(=O)Oc1ccccc1C(=O)O" | ./target/release/renkin-fp
///
/// Output format: "4 17 42 ..." (set bit indices, space-separated) or "ERR"
#[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
fn main() {
    use std::io::{self, BufRead, Write};
    use chematic::fp::{EcfpConfig, ecfp};
    use renkin::chem_env::mol_from_smiles;

    const ECFP_CONFIG: EcfpConfig = EcfpConfig {
        radius: 2,
        nbits: 2048,
        use_chirality: false,
        use_double_fold: false,
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let smiles = match line {
            Ok(s) => s,
            Err(_) => { writeln!(out, "ERR").ok(); continue; }
        };
        let smiles = smiles.trim();
        if smiles.is_empty() || smiles.starts_with('#') {
            writeln!(out, "ERR").ok();
            continue;
        }
        match mol_from_smiles(smiles) {
            Ok(mol) => {
                let bv = ecfp(&mol, &ECFP_CONFIG);
                let bits: Vec<String> = (0..2048usize)
                    .filter(|&i| bv.get(i))
                    .map(|i| i.to_string())
                    .collect();
                writeln!(out, "{}", bits.join(" ")).ok();
            }
            Err(_) => { writeln!(out, "ERR").ok(); }
        }
    }
}

#[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
fn main() {
    eprintln!("renkin-fp requires --features nn-scoring");
    std::process::exit(1);
}
