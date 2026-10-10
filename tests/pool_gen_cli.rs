//! Exercise the candidate-pool driver as a process, including provenance and
//! groups that must remain in the denominator despite failed parsing.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory() -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "renkin_pool_gen_cli_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).expect("create temporary output directory");
    path
}

#[test]
fn pool_driver_keeps_successful_and_failed_groups_in_its_index() {
    let dir = temporary_directory();
    let groups = dir.join("groups.jsonl");
    let pool = dir.join("pool.jsonl");
    let index = dir.join("index.jsonl");
    let manifest = dir.join("manifest.json");
    let target = renkin::chem_env::to_canonical(&renkin::chem_env::mol_from_smiles("CCO").unwrap());
    std::fs::write(
        &groups,
        format!(
            "{}\n{}\n",
            serde_json::json!({"group_id": "valid", "target_id": target}),
            serde_json::json!({"group_id": "invalid", "target_id": "not-a-smiles"}),
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_renkin-pool-gen"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "--groups",
            groups.to_str().unwrap(),
            "--templates",
            "tests/fixtures/coverage_mode_templates.smi",
            "--pool-output",
            pool.to_str().unwrap(),
            "--groups-output",
            index.to_str().unwrap(),
            "--manifest-output",
            manifest.to_str().unwrap(),
            "--limit",
            "2",
        ])
        .output()
        .expect("run candidate-pool driver");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let records: Vec<serde_json::Value> = std::fs::read_to_string(&index)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["group_id"], "valid");
    assert_eq!(records[0]["proposal_status"], "ok");
    assert_eq!(records[1]["group_id"], "invalid");
    assert_eq!(records[1]["proposal_status"], "target_parse_failed");
    assert!(pool.is_file());

    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    assert!(provenance.to_string().contains("exhaustive"));
    assert!(provenance.to_string().contains("sha256:"));

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pool_driver_rejects_duplicate_cli_flags_before_opening_inputs() {
    let output = Command::new(env!("CARGO_BIN_EXE_renkin-pool-gen"))
        .args(["--limit", "1", "--limit", "2"])
        .output()
        .expect("run candidate-pool driver");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate argument"));
}
