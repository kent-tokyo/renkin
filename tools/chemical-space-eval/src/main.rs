#![forbid(unsafe_code)]

//! Chemical Space Coverage Diagnosis (Phase A PoC): nearest-TRAIN ECFP4
//! Tanimoto similarity for each formal-TEST target.
//!
//! Standalone from RENKIN core by design (see this crate's Cargo.toml) --
//! reads two plain-text/JSONL inputs, writes one JSONL output, no RENKIN
//! dependency at all.
//!
//! Usage:
//!   cargo run --release --manifest-path tools/chemical-space-eval/Cargo.toml -- \
//!       --train-reference data/chemical_space_coverage_diagnosis/train_reference_products.smi \
//!       --test-labels data/chemical_space_coverage_diagnosis/test_target_labels.jsonl \
//!       --output data/chemical_space_coverage_diagnosis/nearest_train_tanimoto.jsonl

use chematic::fp::{BitVec2048, ecfp4, top_k_similar};
use chematic::smiles;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};

/// Pinned HF revision for `bisectgroup/USPTO_50K` that
/// `train_reference_products.smi` and `test_target_labels.jsonl` both
/// ultimately derive from -- see
/// `data/phase3a_reranker_ground_truth_audit/findings.md` for the full
/// provenance chain. Not read from any file here since neither input
/// carries it; recorded directly so the manifest is self-contained.
const SOURCE_HF_REVISION: &str = "08a575f0546b2be57242997fd45f684d6814d5a9";

fn arg_value(flag: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| panic!("missing required flag {flag}"))
}

/// sha2/digest 0.11's output type (`Array<u8, N>`, from the `hybrid-array`
/// crate) no longer implements `LowerHex` the way `generic_array::
/// GenericArray` did in 0.10 (same issue/fix as RENKIN's own
/// `renkin::sha256_hex` -- not reused here to keep this crate RENKIN-free).
fn sha256_file(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let hex: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sha256:{hex}")
}

/// Best-effort -- `git rev-parse HEAD` if run from within a checkout, else
/// `None` rather than failing the whole run over a provenance nicety.
fn renkin_commit() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

#[derive(Deserialize)]
struct TestTargetRow {
    target_id: String,
    target_smiles: String,
}

#[derive(Serialize)]
struct OutputRow<'a> {
    target_id: &'a str,
    nearest_train_tanimoto: f32,
    nearest_train_smiles: &'a str,
}

fn main() {
    let train_reference_path = arg_value("--train-reference");
    let test_labels_path = arg_value("--test-labels");
    let output_path = arg_value("--output");

    let train_reference_sha256 = sha256_file(&train_reference_path);
    let test_labels_sha256 = sha256_file(&test_labels_path);

    eprintln!("Loading TRAIN reference SMILES from {train_reference_path}...");
    let train_smiles: Vec<String> = BufReader::new(
        File::open(&train_reference_path)
            .unwrap_or_else(|e| panic!("open {train_reference_path}: {e}")),
    )
    .lines()
    .map(|l| l.unwrap())
    .filter(|l| !l.trim().is_empty())
    .collect();

    let mut train_fps: Vec<BitVec2048> = Vec::with_capacity(train_smiles.len());
    let mut train_ok_smiles: Vec<String> = Vec::with_capacity(train_smiles.len());
    let mut train_parse_failures = 0usize;
    for s in &train_smiles {
        match smiles::parse(s) {
            Ok(mol) => {
                train_fps.push(ecfp4(&mol));
                train_ok_smiles.push(s.clone());
            }
            Err(_) => train_parse_failures += 1,
        }
    }
    eprintln!(
        "TRAIN reference: {} parsed, {} parse failures",
        train_fps.len(),
        train_parse_failures
    );

    eprintln!("Loading TEST targets from {test_labels_path}...");
    let test_rows: Vec<TestTargetRow> = BufReader::new(
        File::open(&test_labels_path).unwrap_or_else(|e| panic!("open {test_labels_path}: {e}")),
    )
    .lines()
    .map(|l| l.unwrap())
    .filter(|l| !l.trim().is_empty())
    .map(|l| serde_json::from_str(&l).unwrap_or_else(|e| panic!("parse {l:?}: {e}")))
    .collect();

    let out_file =
        File::create(&output_path).unwrap_or_else(|e| panic!("create {output_path}: {e}"));
    let mut out = BufWriter::new(out_file);

    let mut test_parse_failures = 0usize;
    for (i, row) in test_rows.iter().enumerate() {
        if i % 1000 == 0 {
            eprintln!("{i}/{}...", test_rows.len());
        }
        let mol = match smiles::parse(&row.target_smiles) {
            Ok(m) => m,
            Err(_) => {
                test_parse_failures += 1;
                continue;
            }
        };
        let query_fp = ecfp4(&mol);
        let nearest = top_k_similar(&query_fp, &train_fps, 1);
        let (idx, score) = nearest[0];
        let out_row = OutputRow {
            target_id: &row.target_id,
            nearest_train_tanimoto: score,
            nearest_train_smiles: &train_ok_smiles[idx],
        };
        writeln!(out, "{}", serde_json::to_string(&out_row).unwrap()).unwrap();
    }

    out.flush()
        .unwrap_or_else(|e| panic!("flush {output_path}: {e}"));
    drop(out);

    eprintln!(
        "Done. {} TEST targets scored, {} parse failures.",
        test_rows.len() - test_parse_failures,
        test_parse_failures
    );

    let output_sha256 = sha256_file(&output_path);

    let manifest = serde_json::json!({
        "fingerprint": "ecfp4",
        "radius": 2,
        "nbits": 2048,
        "chirality": false,
        "chematic_version": "0.11.0",
        "renkin_commit": renkin_commit(),
        "source_hf_revision": SOURCE_HF_REVISION,
        "train_reference_path": train_reference_path,
        "train_reference_sha256": train_reference_sha256,
        "train_reference_count_total": train_smiles.len(),
        "train_reference_count_parsed": train_fps.len(),
        "train_reference_parse_failures": train_parse_failures,
        "test_labels_path": test_labels_path,
        "test_labels_sha256": test_labels_sha256,
        "test_target_count_total": test_rows.len(),
        "test_target_parse_failures": test_parse_failures,
        "output_path": output_path,
        "output_sha256": output_sha256,
    });
    let manifest_path = format!("{output_path}.manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap_or_else(|e| panic!("write {manifest_path}: {e}"));
    eprintln!("Wrote {output_path} and {manifest_path}");
}
