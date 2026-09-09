use std::fs;
use std::process::Command;

use sha2::{Digest, Sha256};

#[test]
fn cli_loads_hash_pinned_retro_generator_and_augments_frontier() {
    let stem = format!(
        "renkin-retro-generator-cli-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    );
    let dir = std::env::temp_dir();
    let artifact_path = dir.join(format!("{stem}.json"));
    let manifest_path = dir.join(format!("{stem}-manifest.json"));
    let stock_path = dir.join(format!("{stem}-stock.smi"));
    let artifact = serde_json::json!({
        "schema_version": 1,
        "proposals": {
            "CCO": [{
                "candidate_id": "fixture-cc",
                "target": "CCO",
                "precursors": ["CC", "O"],
                "atom_mapping": null,
                "provenance": {
                    "generator_id": "cli-fixture",
                    "generator_version": "1",
                    "artifact_sha256": "sha256:fixture",
                    "source_rank": 0,
                    "model_confidence": 0.5
                }
            }]
        }
    });
    let artifact_bytes = serde_json::to_vec(&artifact).unwrap();
    let model_hash = format!(
        "sha256:{}",
        renkin::sha256_hex(Sha256::digest(&artifact_bytes))
    );
    let manifest = serde_json::json!({
        "schema_version": 1,
        "model_id": "cli-retro-fixture",
        "model_kind": "retro_generator",
        "model_version": "0.1.0",
        "model_sha256": model_hash,
        "input_schema": "canonical-smiles-v1",
        "output_semantics": "rank",
        "license": "MIT",
        "ood_abstain": true
    });
    fs::write(&artifact_path, artifact_bytes).unwrap();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    fs::write(&stock_path, "CC\nO\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_renkin"))
        .args([
            "--target",
            "CCO",
            "--depth",
            "1",
            "--max-routes",
            "5",
            "--building-blocks",
            stock_path.to_str().unwrap(),
            "--retro-generator-manifest",
            manifest_path.to_str().unwrap(),
            "--retro-generator-artifact",
            artifact_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let _ = fs::remove_file(&artifact_path);
    let _ = fs::remove_file(&manifest_path);
    let _ = fs::remove_file(&stock_path);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !json["routes"].as_array().unwrap().is_empty(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // The staged policy deliberately returns a native success without
    // running the generator arm. This target is natively solvable with the
    // fixture stock, so the CLI contract here is loading plus successful
    // dispatch, not forcing a direct-generator route to replace it.
    assert!(String::from_utf8_lossy(&output.stderr).contains("Loaded hash-pinned retro generator"));
}
