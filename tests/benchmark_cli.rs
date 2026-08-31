//! Process-level coverage-mode checks for `renkin-bench`.

use std::fs;
use std::process::Command;

fn bench_bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin-bench")
}

fn target_fixture() -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("renkin_benchmark_cli_{}.smi", std::process::id()));
    fs::write(&path, "CC(=O)O\tacetic acid\n").unwrap();
    path
}

#[test]
fn coverage_mode_report_identifies_the_selected_stage() {
    let input = target_fixture();
    let templates = format!(
        "{}/tests/fixtures/coverage_mode_templates.smi",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = Command::new(bench_bin())
        .args([
            "--input",
            input.to_str().unwrap(),
            "--depth",
            "0",
            "--search-mode",
            "coverage",
            "--coverage-templates",
            &templates,
        ])
        .output()
        .unwrap();
    let _ = fs::remove_file(&input);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["search_mode"], "coverage");
    assert_eq!(report["coverage_template_count"].as_u64(), Some(23));
    let result = &report["results"][0];
    assert_eq!(result["coverage_selected_stage"], "stage1");
    assert_eq!(result["coverage_stage2_invoked"], false);
    assert_eq!(result["coverage_stage1_timeout"], false);
    assert_eq!(result["coverage_stage2_timeout"], false);
    assert!(result["coverage_stage2_elapsed_ms"].is_null());
}

#[test]
fn standard_mode_rejects_coverage_options_before_reading_targets() {
    let output = Command::new(bench_bin())
        .args([
            "--input",
            "/path/that/does/not/exist.smi",
            "--search-mode",
            "standard",
            "--coverage-timeout-secs",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires --search-mode coverage"));
}
