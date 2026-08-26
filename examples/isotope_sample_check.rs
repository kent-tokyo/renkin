//! Standing verification tool: runs every line of a stock `.smi` sample
//! through the real `recanonicalize_stock_smiles` path and reports whether
//! deuterium/tritium (`[2H]`/`[3H]`) and heavy-atom isotope tags
//! (`[13C]`/`[14C]`/`[15N]`/`[18O]`) survive canonicalization.
//!
//! Grew out of the v0.36.0 Phase 2 investigation that found chematic
//! silently stripping isotope-labeled hydrogen (chematic#389, fixed in
//! chematic 0.20.1) -- kept as a permanent example so any future chematic
//! bump can re-run the same check against a real corpus sample before
//! trusting a `renkin doctor stock reimport_idempotency` PASS. See
//! `docs/design/PHASE3A_CHEMATIC_ISOTOPE_FIX_STATUS.md` for the full
//! investigation record.

use renkin::stock_import::recanonicalize_stock_smiles;
use std::io::BufRead;

fn main() {
    let path = std::env::args().nth(1).expect("usage: <sample.smi>");
    let file = std::fs::File::open(path).unwrap();
    let reader = std::io::BufReader::new(file);

    let mut total = 0;
    let mut lost_h_isotope = 0;
    let mut kept_h_isotope = 0;
    let mut parse_failed = 0;

    for line in reader.lines() {
        let line = line.unwrap();
        let smi = line.trim();
        if smi.is_empty() {
            continue;
        }
        total += 1;
        match recanonicalize_stock_smiles(smi) {
            Ok(canon) => {
                let input_has_h_isotope = smi.contains("[2H]") || smi.contains("[3H]");
                let output_has_h_isotope = canon.contains("[2H]") || canon.contains("[3H]");
                if input_has_h_isotope && !output_has_h_isotope {
                    lost_h_isotope += 1;
                } else if input_has_h_isotope && output_has_h_isotope {
                    kept_h_isotope += 1;
                }
                // Heavy-isotope survival check, when present.
                for tag in ["[13C", "[14C", "[15N", "[18O"] {
                    if smi.contains(tag) && !canon.contains(tag) {
                        println!(
                            "UNEXPECTED: heavy isotope tag {tag} present in input but missing \
                             from output -- input={smi} output={canon}"
                        );
                    }
                }
            }
            Err(_) => parse_failed += 1,
        }
    }

    println!("total: {total}");
    println!("lost_h_isotope: {lost_h_isotope}");
    println!("kept_h_isotope: {kept_h_isotope}");
    println!("parse_failed: {parse_failed}");
}
