//! Process-level tests for the SynPlanner-gap features: the depicted HTML
//! route report, `renkin batch`, and `--search-stats`.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin")
}

fn run_raw(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .env("RUST_BACKTRACE", "0")
        .output()
        .expect("failed to spawn renkin")
}

fn run(args: &[&str]) -> serde_json::Value {
    let out = run_raw(args);
    assert!(
        out.status.success(),
        "renkin exited non-zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON")
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("renkin-gap-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const ASPIRIN: &str = "CC(=O)Oc1ccccc1C(=O)O";

#[test]
fn html_format_renders_a_self_contained_depicted_report() {
    let out = run_raw(&[
        "--target",
        ASPIRIN,
        "--depth",
        "2",
        "--max-routes",
        "2",
        "--format",
        "html",
    ]);
    assert!(out.status.success());
    let html = String::from_utf8(out.stdout).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert!(!html.contains("<script"));
    assert_eq!(html.matches("<section class=\"route\">").count(), 2);
    assert!(html.matches("<svg").count() >= 3);
    assert!(html.contains("class=\"mol stock\""));

    let caps = run(&["capabilities", "--output", "json"]);
    assert_eq!(caps["search"]["html_report"], true);
}

#[test]
fn search_stats_is_opt_in_and_reported_with_routes() {
    let plain = run(&["--target", ASPIRIN, "--depth", "2"]);
    assert!(plain.get("search_stats").is_none());
    let v = run(&["--target", ASPIRIN, "--depth", "2", "--search-stats"]);
    let stats = &v["search_stats"];
    assert!(v["routes_found"].as_u64().unwrap() > 0);
    assert!(stats["nodes_expanded"].as_u64().unwrap() > 0);
    assert!(
        stats["nodes_generated"].as_u64().unwrap() >= stats["nodes_expanded"].as_u64().unwrap()
    );
    assert_eq!(stats["routes_returned"], v["routes_found"]);
    assert!(stats["search_elapsed_ms"].as_f64().unwrap() >= 0.0);
    assert!(stats.get("crowd_out").is_none());
    assert!(stats["first_route_nodes_expanded"].as_u64().is_some());
}

#[test]
fn batch_writes_per_target_results_summary_and_manifest() {
    let dir = temp_dir("batch");
    let input = dir.join("targets.smi");
    std::fs::write(
        &input,
        "# targets\nCC(=O)Oc1ccccc1C(=O)O aspirin\nnot((smiles broken, \"quoted\"\nCC(=O)O\n",
    )
    .unwrap();
    let out_dir = dir.join("out");
    let manifest = run(&[
        "batch",
        "--input",
        input.to_str().unwrap(),
        "--output-dir",
        out_dir.to_str().unwrap(),
        "--jobs",
        "2",
        "--html",
        "--",
        "--depth",
        "2",
        "--max-routes",
        "2",
    ]);
    assert_eq!(manifest["targets"], 3);
    assert_eq!(manifest["solved"], 2);
    assert_eq!(manifest["errors"], 1);
    assert_eq!(
        manifest["search_options"],
        serde_json::json!(["--depth", "2", "--max-routes", "2"])
    );

    let csv = std::fs::read_to_string(out_dir.join("summary.csv")).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 4, "header + one row per target in input order");
    assert!(lines[0].starts_with("index,name,smiles,status,routes_found,solved"));
    assert!(lines[1].starts_with("1,aspirin,CC(=O)Oc1ccccc1C(=O)O,ok,2,true,"));
    assert!(lines[2].starts_with("2,\"broken, \"\"quoted\"\"\",not((smiles,error,"));
    assert!(lines[3].starts_with("3,,CC(=O)O,ok,"));

    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out_dir.join("routes/0001_aspirin.json")).unwrap())
            .unwrap();
    assert_eq!(result["routes_found"], 2);
    assert!(result["search_stats"]["nodes_expanded"].as_u64().is_some());
    let html = std::fs::read_to_string(out_dir.join("routes/0001_aspirin.html")).unwrap();
    assert!(html.contains("<svg"));
    assert!(out_dir.join("manifest.json").exists());

    // Existing results are not overwritten silently.
    let again = run_raw(&[
        "batch",
        "--input",
        input.to_str().unwrap(),
        "--output-dir",
        out_dir.to_str().unwrap(),
    ]);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("--overwrite"));

    // `summary.csv` is not the sole guard: a partial/manual deletion must
    // not make the already-written route artifacts silently overwritable.
    std::fs::remove_file(out_dir.join("summary.csv")).unwrap();
    let route_before = std::fs::read(out_dir.join("routes/0001_aspirin.json")).unwrap();
    let missing_summary = run_raw(&[
        "batch",
        "--input",
        input.to_str().unwrap(),
        "--output-dir",
        out_dir.to_str().unwrap(),
    ]);
    assert!(!missing_summary.status.success());
    assert!(String::from_utf8_lossy(&missing_summary.stderr).contains("--overwrite"));
    assert_eq!(
        route_before,
        std::fs::read(out_dir.join("routes/0001_aspirin.json")).unwrap()
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_rejects_target_or_format_in_passthrough() {
    let dir = temp_dir("batch-bad");
    let input = dir.join("t.smi");
    std::fs::write(&input, "CCO\n").unwrap();
    let out = run_raw(&[
        "batch",
        "--input",
        input.to_str().unwrap(),
        "--output-dir",
        dir.join("o").to_str().unwrap(),
        "--",
        "--format",
        "tree",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--format is set by batch"));
    std::fs::remove_dir_all(&dir).ok();
}
