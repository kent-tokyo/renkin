#![forbid(unsafe_code)]

//! Issue #101 runtime-integration golden test, Rust side.
//!
//! Reads JSON Lines from stdin, each `{"features": [18 floats or nulls]}`
//! (`null` = missing/NaN, matching `CandidateFeatures.missing`), runs each
//! through `renkin::reranker::LightGbmModel::predict`, and writes
//! `{"rust_score": f64}` per line to stdout in the same order. Pure
//! stdin/stdout so the Python side of the golden test
//! (`scripts/reranker_golden_test.py` in the offline PR) can pipe real
//! formal-TEST feature vectors through and diff against its own
//! `booster.predict()` output for the identical rows -- this binary has no
//! opinion on where the feature vectors came from.
//!
//! Usage:
//!   ./target/release/renkin-reranker-predict --model <path/to/model.txt> \
//!       < features.jsonl > scores.jsonl

use renkin::reranker::LightGbmModel;
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Deserialize)]
struct InRow {
    features: Vec<Option<f64>>,
}

#[derive(Serialize)]
struct OutRow {
    rust_score: f64,
}

fn arg_value(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() {
    let model_path = arg_value("--model").unwrap_or_else(|| {
        eprintln!("Usage: renkin-reranker-predict --model <path/to/model.txt> < features.jsonl");
        std::process::exit(2);
    });
    let model = LightGbmModel::from_path(&model_path)
        .unwrap_or_else(|e| panic!("load model {model_path}: {e}"));

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut input = stdin.lock();
    let mut i = 0;
    while let Some(line) = renkin::io_limits::read_bounded_line(&mut input, "reranker stdin")
        .unwrap_or_else(|e| panic!("read stdin line {i}: {e}"))
    {
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        let row: InRow =
            serde_json::from_str(&line).unwrap_or_else(|e| panic!("parse line {i}: {e}"));
        let features: Vec<f64> = row.features.iter().map(|v| v.unwrap_or(f64::NAN)).collect();
        let score = model
            .predict(&features)
            .unwrap_or_else(|e| panic!("predict line {i}: {e}"));
        let out_row = OutRow { rust_score: score };
        writeln!(out, "{}", serde_json::to_string(&out_row).unwrap())
            .unwrap_or_else(|e| panic!("write stdout line {i}: {e}"));
        i += 1;
    }
}
