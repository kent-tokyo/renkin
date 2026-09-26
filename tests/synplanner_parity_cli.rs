//! Process-level tests for SynPlanner-parity options.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin")
}

fn run(args: &[&str]) -> serde_json::Value {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn renkin");
    assert!(
        out.status.success(),
        "renkin exited non-zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON")
}

fn run_failure(args: &[&str]) -> String {
    let out = Command::new(bin())
        .args(args)
        .env("RUST_BACKTRACE", "0")
        .output()
        .expect("failed to spawn renkin");
    assert!(!out.status.success(), "renkin unexpectedly succeeded");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn temp_file(name: &str, content: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("renkin-synplanner-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

const ASPIRIN: &str = "CC(=O)Oc1ccccc1C(=O)O";

#[test]
fn small_molecule_terminal_solves_with_size_terminals_and_reports_them() {
    let stock = temp_file("salicylic.smi", b"Oc1ccccc1C(=O)O\n");
    let plain = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "2",
        "--building-blocks",
        stock.to_str().unwrap(),
    ]);
    assert_eq!(plain["routes_found"], 0);
    assert!(plain.get("small_molecule_terminal").is_none());

    let v = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "2",
        "--building-blocks",
        stock.to_str().unwrap(),
        "--small-molecule-terminal",
        "4",
    ]);
    let routes = v["routes"].as_array().unwrap();
    assert!(!routes.is_empty());
    let receipt = &v["small_molecule_terminal"];
    assert_eq!(receipt["max_heavy_atoms"], 4);
    let leaves = receipt["non_stock_leaves"].as_array().unwrap();
    assert_eq!(leaves.len(), routes.len());
    assert_eq!(receipt["routes_with_non_stock_leaves"], routes.len());
    for (route, non_stock) in routes.iter().zip(leaves) {
        let bbs = route["building_blocks"].as_array().unwrap();
        for leaf in non_stock.as_array().unwrap() {
            assert!(bbs.contains(leaf));
        }
    }

    // Audit keeps stock identity exact: size terminals are not stock.
    let err = run_failure(&["--target", ASPIRIN, "--small-molecule-terminal", "-1"]);
    assert!(err.contains("--small-molecule-terminal must be a non-negative integer"));
}

#[test]
fn synplanner_format_export_round_trips_through_audit_route() {
    let out = Command::new(bin())
        .args([
            "--target",
            ASPIRIN,
            "--depth",
            "3",
            "--max-routes",
            "3",
            "--format",
            "synplanner",
        ])
        .output()
        .expect("failed to spawn renkin");
    assert!(out.status.success());
    let export: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let map = export.as_object().expect("{route_id: RouteNode}");
    assert!(!map.is_empty());
    for (key, tree) in map {
        assert!(key.parse::<u64>().is_ok());
        assert_eq!(tree["type"], "mol");
    }
    let path = temp_file("routes.json", &out.stdout);
    let report = run(&[
        "audit-route",
        path.to_str().unwrap(),
        "--stock",
        "data/building_blocks.smi",
        "--output",
        "json",
    ]);
    assert_eq!(report["source_format"], "synplanner");
    assert_eq!(report["summary"]["routes_total"], map.len());
    for route in report["routes"].as_array().unwrap() {
        assert_eq!(route["route_tree_parseable"], true);
        assert_eq!(route["stock_validation"]["status"], "pass");
    }
}

#[test]
fn audit_route_reads_synplanner_export_routes_results_gz() {
    use std::io::Write;
    let raw = std::fs::read("tests/fixtures/synplanner/v1.6.0/real_planning_export.results.json")
        .unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&raw).unwrap();
    let path = temp_file("results.json.gz", &encoder.finish().unwrap());
    let report = run(&["audit-route", path.to_str().unwrap(), "--output", "json"]);
    assert_eq!(report["source_format"], "synplanner");
    assert_eq!(report["summary"]["routes_total"], 2);
}

#[test]
fn max_tree_size_stops_deterministically() {
    let args = [
        "--target",
        ASPIRIN,
        "--depth",
        "4",
        "--max-routes",
        "50",
        "--max-tree-size",
        "20",
    ];
    let a = run(&args);
    let b = run(&args);
    assert_eq!(a["termination"], "tree_size_limit_reached");
    assert_eq!(a["max_tree_size"]["reached"], true);
    assert_eq!(a["max_tree_size"]["nodes_generated"], 20);
    assert_eq!(a["routes"], b["routes"]);
    let err = run_failure(&["--target", ASPIRIN, "--max-tree-size", "0"]);
    assert!(err.contains("--max-tree-size must be a positive integer"));
    let err = run_failure(&[
        "--target",
        ASPIRIN,
        "--search-mode",
        "recovery",
        "--max-tree-size",
        "5",
    ]);
    assert!(err.contains("--max-tree-size applies to --search-mode standard"));
}

#[test]
fn priority_rules_promote_and_report_unknown_names() {
    let baseline = run(&["--target", ASPIRIN, "--depth", "2"]);
    let first_rule = |v: &serde_json::Value| {
        v["routes"][0]["steps"][0]["rule"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let baseline_first = first_rule(&baseline);
    let other = baseline["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["steps"][0]["rule"].as_str().unwrap().to_owned())
        .find(|rule| *rule != baseline_first)
        .expect("at least two distinct first-step rules");

    let file = temp_file("priority.txt", format!("# priority\n{other}\n").as_bytes());
    let v = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "2",
        "--priority-templates",
        file.to_str().unwrap(),
        "--priority-rules",
        "no_such_rule",
    ]);
    let receipt = &v["priority_templates"];
    assert_eq!(receipt["count"], 2);
    assert_eq!(receipt["unknown"], serde_json::json!(["no_such_rule"]));
    assert!(receipt["candidates_promoted"].as_u64().unwrap() > 0);
    assert_eq!(first_rule(&v), other);
}
