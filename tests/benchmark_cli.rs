//! Process-level coverage-mode checks for `renkin-bench`.

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

fn bench_bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin-bench")
}

fn target_fixture() -> std::path::PathBuf {
    let fixture_id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "renkin_benchmark_cli_{}_{}.smi",
        std::process::id(),
        fixture_id
    ));
    fs::write(&path, "CC(=O)O\tacetic acid\n").unwrap();
    path
}

fn fixture_path(extension: &str) -> std::path::PathBuf {
    let fixture_id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "renkin_benchmark_cli_{}_{}.{}",
        std::process::id(),
        fixture_id,
        extension
    ))
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
    // The active handcrafted rule set includes the complementary
    // `boc_protection_retro` candidate generator.
    assert_eq!(report["coverage_template_count"].as_u64(), Some(26));
    assert_eq!(report["coverage_stage1_selected_targets"], 1);
    assert_eq!(report["coverage_stage2_invoked_targets"], 0);
    assert_eq!(report["coverage_stage2_timeout_targets"], 0);
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

#[test]
fn metadata_sidecar_failure_is_reported_loudly() {
    let input = target_fixture();
    let output = Command::new(bench_bin())
        .args([
            "--input",
            input.to_str().unwrap(),
            "--template-metadata",
            "/path/that/does/not/exist.json",
        ])
        .output()
        .unwrap();
    let _ = fs::remove_file(&input);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("template metadata file"));
}

#[test]
fn first_route_stats_are_opt_in_per_target_fields() {
    let input = std::env::temp_dir().join(format!(
        "renkin_benchmark_cli_first_route_{}.smi",
        std::process::id()
    ));
    fs::write(&input, "CC(=O)Oc1ccccc1C(=O)O\taspirin\n").unwrap();
    let run = |extra: &[&str]| {
        let mut args = vec!["--input", input.to_str().unwrap(), "--depth", "3"];
        args.extend_from_slice(extra);
        let output = Command::new(bench_bin()).args(&args).output().unwrap();
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let plain = run(&[]);
    let with_stats = run(&["--first-route-stats"]);
    let limited = run(&["--first-route-stats", "--max-expansions", "1"]);
    let _ = fs::remove_file(&input);

    assert!(
        plain["results"][0]
            .get("first_route_nodes_expanded")
            .is_none()
    );
    let result = &with_stats["results"][0];
    assert_eq!(result["solved"], true);
    let first = result["first_route_nodes_expanded"].as_u64().unwrap();
    assert!(first <= result["nodes_expanded"].as_u64().unwrap());
    assert!(result["first_route_expansion_calls"].as_u64().is_some());
    assert!(result["first_route_time_ms"].as_f64().unwrap() >= 0.0);
    assert!(limited["results"][0]["nodes_expanded"].as_u64().unwrap() <= 1);
}

#[test]
fn compare_reports_newly_solved_and_regressed_targets() {
    let baseline = fixture_path("baseline.json");
    let current = fixture_path("current.json");
    fs::write(
        &baseline,
        r#"{"success_rate":0.5,"avg_time_ms":20.0,"results":[{"smiles":"CCO","solved":true},{"smiles":"CCN","solved":false}]}"#,
    )
    .unwrap();
    fs::write(
        &current,
        r#"{"success_rate":0.5,"avg_time_ms":10.0,"results":[{"smiles":"CCO","solved":false},{"smiles":"CCN","solved":true}]}"#,
    )
    .unwrap();
    let output = Command::new(bench_bin())
        .args([
            "compare",
            baseline.to_str().unwrap(),
            current.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let _ = fs::remove_file(baseline);
    let _ = fs::remove_file(current);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Newly solved (1):\n  + CCN"));
    assert!(text.contains("Regressions (1):\n  - CCO"));
    assert!(text.contains("20.0 ms → 10.0 ms  (-10.0 ms)"));
}

#[test]
fn cascade_only_retries_unsolved_targets_and_keeps_quality_fields() {
    let input = fixture_path("smi");
    let stock = fixture_path("stock.smi");
    let first = fixture_path("first.json");
    let second = fixture_path("second.json");
    fs::write(&input, "CCO ethanol\nCCN ethylamine\n").unwrap();
    fs::write(&stock, "CCO\n").unwrap();
    let stock_path = stock.to_str().unwrap();
    fs::write(
        &first,
        serde_json::json!({"name":"stock","depth":0,"beam_width":0,"building_blocks":stock_path})
            .to_string(),
    )
    .unwrap();
    fs::write(
        &second,
        serde_json::json!({"name":"retry","depth":0,"beam_width":0,"building_blocks":stock_path,"only_unsolved_from_previous":true}).to_string(),
    )
    .unwrap();
    let output = Command::new(bench_bin())
        .args([
            "cascade",
            "--input",
            input.to_str().unwrap(),
            "--stage",
            first.to_str().unwrap(),
            "--stage",
            second.to_str().unwrap(),
            "--quality",
        ])
        .output()
        .unwrap();
    for path in [input, stock, first, second] {
        let _ = fs::remove_file(path);
    }
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["total"], 2);
    assert_eq!(report["total_solved"], 1);
    assert_eq!(report["stages"][0]["attempted"], 2);
    assert_eq!(report["stages"][0]["newly_solved"], 1);
    assert_eq!(report["stages"][1]["attempted"], 1);
    assert_eq!(report["stages"][1]["newly_solved"], 0);
    assert!(report["forward_validation_pass_rate"].as_f64().is_some());
}

#[test]
fn paroutes_input_preserves_ground_truth_depth() {
    let input = fixture_path("paroutes.json");
    fs::write(
        &input,
        r#"[{"smiles":"CCO","children":[{"type":"reaction","children":[{"type":"mol","smiles":"CC"}]}]}]"#,
    )
    .unwrap();
    let output = Command::new(bench_bin())
        .args([
            "--input",
            input.to_str().unwrap(),
            "--input-format",
            "paroutes",
            "--depth",
            "0",
        ])
        .output()
        .unwrap();
    let _ = fs::remove_file(input);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["total"], 1);
    assert_eq!(report["results"][0]["name"], "paroutes_0");
    assert_eq!(report["results"][0]["gt_depth"], 1);
}

#[test]
fn benchmark_emits_failure_taxonomy_and_quietset_observations() {
    let input = fixture_path("smi");
    let stock = fixture_path("stock.smi");
    let observations = fixture_path("jsonl");
    fs::write(&input, "CCO ethanol\nCCN ethylamine\n").unwrap();
    fs::write(&stock, "CCO\n").unwrap();
    let output = Command::new(bench_bin())
        .args([
            "--input",
            input.to_str().unwrap(),
            "--building-blocks",
            stock.to_str().unwrap(),
            "--depth",
            "0",
            "--plausibility",
            "--failure-taxonomy",
            "--quietset-out",
            observations.to_str().unwrap(),
            "--evaluator-id",
            "fixture-evaluator",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["total"], 2);
    assert_eq!(report["solved"], 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Failure Taxonomy (1 unsolved)"));
    let rows: Vec<serde_json::Value> = fs::read_to_string(&observations)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for path in [input, stock, observations] {
        let _ = fs::remove_file(path);
    }
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["label"], "solved");
    assert_eq!(rows[1]["label"], "unsolved");
    assert_eq!(rows[0]["evaluator_id"], "fixture-evaluator");
}
