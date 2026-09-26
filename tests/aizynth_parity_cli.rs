//! Process-level tests for the AiZynthFinder-parity search options:
//! `--time-limit-secs` (AiZ `time_limit`) and `--exclude-target-from-stock`
//! (AiZ `exclude_target_from_stock`).

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

const ASPIRIN: &str = "CC(=O)Oc1ccccc1C(=O)O";
const ACETIC_ACID: &str = "CC(=O)O";

#[test]
fn legacy_output_has_no_parity_fields() {
    let v = run(&["--target", ASPIRIN, "--depth", "2"]);
    assert!(v.get("time_limit_secs").is_none());
    assert!(v.get("termination").is_none());
    assert!(v.get("exclude_target_from_stock").is_none());
}

#[test]
fn time_limit_reports_completed_when_search_finishes() {
    let v = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "2",
        "--time-limit-secs",
        "60",
    ]);
    assert_eq!(v["time_limit_secs"], 60);
    assert_eq!(v["termination"], "completed");
    assert!(v["routes_found"].as_u64().unwrap() > 0);
}

#[test]
fn time_limit_stops_a_long_search_and_keeps_found_routes() {
    // Depth 10 with an unlimited beam and 1000 requested routes does not
    // finish within one second; the cooperative deadline must stop it.
    let started = std::time::Instant::now();
    let v = run(&[
        "--target",
        "CC(=O)Nc1ccc(OCC(=O)NCc2ccc(Cl)cc2)cc1C(=O)OCC",
        "--depth",
        "10",
        "--max-routes",
        "1000",
        "--time-limit-secs",
        "1",
    ]);
    assert_eq!(v["termination"], "deadline_exceeded");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "deadline must bound the search"
    );
    assert_eq!(
        v["routes_found"].as_u64().unwrap() as usize,
        v["routes"].as_array().map_or(0, Vec::len)
    );
}

#[test]
fn time_limit_rejects_zero_and_non_standard_modes() {
    let err = run_failure(&["--target", ASPIRIN, "--time-limit-secs", "0"]);
    assert!(err.contains("--time-limit-secs must be a positive integer"));
    let err = run_failure(&[
        "--target",
        ASPIRIN,
        "--search-mode",
        "recovery",
        "--time-limit-secs",
        "5",
    ]);
    assert!(err.contains("--time-limit-secs applies to --search-mode standard"));
}

#[test]
fn exclude_target_from_stock_removes_depth_zero_route() {
    let default = run(&["--target", ACETIC_ACID, "--depth", "2"]);
    assert!(
        default["routes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["depth"] == 0),
        "legacy behaviour returns the depth-0 buy route"
    );

    let v = run(&[
        "--target",
        ACETIC_ACID,
        "--depth",
        "2",
        "--exclude-target-from-stock",
    ]);
    assert_eq!(v["exclude_target_from_stock"], true);
    let routes = v["routes"].as_array().unwrap();
    assert!(!routes.is_empty());
    for route in routes {
        assert_ne!(route["depth"], 0);
        assert!(!route["steps"].as_array().unwrap().is_empty());
    }
}

#[test]
fn capabilities_advertise_parity_options() {
    let v = run(&["capabilities", "--output", "json"]);
    assert_eq!(v["search"]["standard_time_limit"], true);
    assert_eq!(v["search"]["exclude_target_from_stock"], true);
}

#[test]
fn expand_lists_ranked_one_step_disconnections() {
    let v = run(&["expand", "--target", ASPIRIN, "--max-candidates", "3"]);
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["target_in_stock"], false);
    let candidates = v["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 3);
    assert_eq!(v["candidates_returned"], 3);
    assert!(v["candidates_total"].as_u64().unwrap() >= 3);
    for (index, candidate) in candidates.iter().enumerate() {
        assert_eq!(candidate["rank"], index + 1);
        assert!(
            candidate["reaction_smiles"]
                .as_str()
                .unwrap()
                .contains(">>")
        );
        assert!(!candidate["template_ids"].as_array().unwrap().is_empty());
    }
    assert!(
        candidates.iter().any(|c| c["all_in_stock"] == true),
        "aspirin has a one-step disconnection into stock"
    );
}

#[test]
fn expand_rejects_missing_target_and_bad_output() {
    let err = run_failure(&["expand"]);
    assert!(err.contains("--target is required"));
    let err = run_failure(&["expand", "--target", ASPIRIN, "--output", "xml"]);
    assert!(err.contains("--output must be json or human"));
    let err = run_failure(&["expand", "--target", ASPIRIN, "--bogus"]);
    assert!(err.contains("unknown option"));
}

#[test]
fn cluster_adds_distance_matrix_and_labels() {
    let v = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "3",
        "--max-routes",
        "6",
        "--cluster",
    ]);
    let routes = v["routes"].as_array().unwrap();
    let clusters = &v["route_clusters"];
    assert_eq!(clusters["schema_version"], 1);
    assert_eq!(
        clusters["distance_method"],
        "canonical_ordered_ted_unit_cost"
    );
    let labels = clusters["labels"].as_array().unwrap();
    assert_eq!(labels.len(), routes.len());
    assert_eq!(labels.first().and_then(|l| l.as_u64()), Some(0));
    let matrix = clusters["distance_matrix"].as_array().unwrap();
    assert_eq!(matrix.len(), routes.len());
    for (i, row) in matrix.iter().enumerate() {
        assert_eq!(row[i], 0.0);
    }

    let fixed = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "3",
        "--max-routes",
        "6",
        "--n-clusters",
        "2",
    ]);
    assert_eq!(fixed["route_clusters"]["selection"], "fixed");
}

#[test]
fn cluster_requires_json_and_valid_counts() {
    let err = run_failure(&["--target", ASPIRIN, "--cluster", "--format", "tree"]);
    assert!(err.contains("require --format json"));
    let err = run_failure(&["--target", ASPIRIN, "--n-clusters", "0"]);
    assert!(err.contains("--n-clusters must be a positive integer"));
    let err = run_failure(&["--target", ASPIRIN, "--max-clusters", "1"]);
    assert!(err.contains("--max-clusters must be an integer >= 2"));
}

#[test]
fn aizynth_format_exports_trees_that_audit_route_reimports() {
    let out = Command::new(bin())
        .args([
            "--target",
            ASPIRIN,
            "--depth",
            "3",
            "--max-routes",
            "3",
            "--format",
            "aizynth",
        ])
        .output()
        .expect("failed to spawn renkin");
    assert!(out.status.success());
    let trees: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let trees = trees
        .as_array()
        .expect("top-level array like aizynthcli trees");
    assert!(!trees.is_empty());
    for tree in trees {
        assert_eq!(tree["type"], "mol");
        assert_eq!(tree["metadata"]["is_solved"], true);
        let reaction = &tree["children"][0];
        assert_eq!(reaction["type"], "reaction");
        assert!(
            reaction["smiles"]
                .as_str()
                .unwrap()
                .starts_with(tree["smiles"].as_str().unwrap()),
            "retro direction product>>reactants"
        );
        assert!(reaction["metadata"].get("mapped_reaction_smiles").is_none());
        assert_eq!(
            tree["scores"]["number of reactions"].as_u64().unwrap() as usize,
            count_reactions(tree)
        );
    }

    let dir = std::env::temp_dir().join(format!("renkin-aizynth-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("trees.json");
    std::fs::write(&path, &out.stdout).unwrap();
    let report = run(&[
        "audit-route",
        path.to_str().unwrap(),
        "--format",
        "aizynthfinder",
        "--stock",
        "data/building_blocks.smi",
        "--output",
        "json",
    ]);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(report["source_format"], "aizynthfinder");
    assert_eq!(report["summary"]["routes_total"], trees.len());
    for route in report["routes"].as_array().unwrap() {
        assert_eq!(route["route_tree_parseable"], true);
        assert_eq!(route["stock_validation"]["status"], "pass");
    }
}

fn count_reactions(node: &serde_json::Value) -> usize {
    let own = usize::from(node["type"] == "reaction");
    own + node["children"]
        .as_array()
        .map_or(0, |children| children.iter().map(count_reactions).sum())
}
