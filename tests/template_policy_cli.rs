use std::fs;
use std::process::Command;

use sha2::{Digest, Sha256};

#[test]
fn cli_loads_hash_pinned_template_policy_without_changing_route_contract() {
    let stem = format!(
        "renkin-template-policy-cli-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    );
    let dir = std::env::temp_dir();
    let artifact_path = dir.join(format!("{stem}.json"));
    let manifest_path = dir.join(format!("{stem}-manifest.json"));
    let artifact = serde_json::json!({
        "schema_version": 1,
        "scores": {"CCO": {"rule:ester_cleavage": 2.0}}
    });
    let artifact_bytes = serde_json::to_vec(&artifact).unwrap();
    let model_hash = format!(
        "sha256:{}",
        renkin::sha256_hex(Sha256::digest(&artifact_bytes))
    );
    let manifest = serde_json::json!({
        "schema_version": 1,
        "model_id": "cli-fixture",
        "model_kind": "template_policy",
        "model_version": "0.1.0",
        "model_sha256": model_hash,
        "input_schema": "canonical-smiles-v1",
        "output_semantics": "logit",
        "template_set_sha256": format!("sha256:{}", "b".repeat(64)),
        "license": "MIT",
        "ood_abstain": true
    });
    fs::write(&artifact_path, artifact_bytes).unwrap();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_renkin"))
        .args([
            "--target",
            "CCO",
            "--max-routes",
            "1",
            "--template-policy-manifest",
            manifest_path.to_str().unwrap(),
            "--template-policy-artifact",
            artifact_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let _ = fs::remove_file(&artifact_path);
    let _ = fs::remove_file(&manifest_path);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("routes").is_some());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Loaded ordering-only template policy")
    );
}
