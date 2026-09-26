//! Process-level tests for ASKCOS/Syntheseus-parity search options:
//! `--max-expansions`, `--first-route-stats`, `--ban-molecules`/`--ban-smiles`,
//! and `--max-bb-price`.

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

fn temp_file(name: &str, content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("renkin-parity-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

const ASPIRIN: &str = "CC(=O)Oc1ccccc1C(=O)O";
const SALICYLIC_ACID_ALT_SPELLING: &str = "OC(=O)c1ccccc1O";

#[test]
fn legacy_output_has_no_new_fields() {
    let v = run(&["--target", ASPIRIN, "--depth", "2"]);
    for key in [
        "max_expansions",
        "first_route",
        "banned_molecules",
        "stock_price_filter",
        "termination",
    ] {
        assert!(v.get(key).is_none(), "{key} must be opt-in");
    }
}

#[test]
fn max_expansions_stops_deterministically() {
    let args = [
        "--target",
        ASPIRIN,
        "--depth",
        "4",
        "--max-routes",
        "50",
        "--max-expansions",
        "3",
        "--first-route-stats",
    ];
    let a = run(&args);
    let b = run(&args);
    assert_eq!(a["termination"], "expansion_limit_reached");
    assert_eq!(a["max_expansions"], 3);
    assert_eq!(a["first_route"]["total_nodes_expanded"], 3);
    assert_eq!(a["routes"], b["routes"]);
    assert_eq!(
        a["first_route"]["nodes_expanded"],
        b["first_route"]["nodes_expanded"]
    );
}

#[test]
fn max_expansions_validation() {
    let err = run_failure(&["--target", ASPIRIN, "--max-expansions", "0"]);
    assert!(err.contains("--max-expansions must be a positive integer"));
    let err = run_failure(&[
        "--target",
        ASPIRIN,
        "--search-mode",
        "recovery",
        "--max-expansions",
        "5",
    ]);
    assert!(err.contains("--max-expansions applies to --search-mode standard"));
}

#[test]
fn first_route_stats_reports_counts_and_time() {
    let v = run(&["--target", ASPIRIN, "--depth", "3", "--first-route-stats"]);
    let receipt = &v["first_route"];
    assert_eq!(receipt["found"], true);
    let first = receipt["nodes_expanded"].as_u64().unwrap();
    assert!(first <= receipt["total_nodes_expanded"].as_u64().unwrap());
    assert!(
        receipt["expansion_calls"].as_u64().unwrap()
            <= receipt["total_expansion_calls"].as_u64().unwrap()
    );
    assert!(receipt["elapsed_ms"].as_f64().unwrap() >= 0.0);
    assert!(v.get("termination").is_none());

    let empty_stock = temp_file("empty.smi", "[Xe]\n");
    let none = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "1",
        "--building-blocks",
        empty_stock.to_str().unwrap(),
        "--first-route-stats",
    ]);
    assert_eq!(none["routes_found"], 0);
    assert_eq!(none["first_route"]["found"], false);
    assert!(none["first_route"]["nodes_expanded"].is_null());
}

#[test]
fn banned_molecules_are_excluded_everywhere() {
    let ban_file = temp_file(
        "ban.smi",
        &format!("# banned chemicals\n{SALICYLIC_ACID_ALT_SPELLING} salicylic acid\n"),
    );
    let v = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "3",
        "--ban-molecules",
        ban_file.to_str().unwrap(),
    ]);
    assert_eq!(v["banned_molecules"]["count"], 1);
    assert!(
        v["banned_molecules"]["candidates_removed"]
            .as_u64()
            .unwrap()
            > 0
    );
    let banned_forms = ["c1(O)ccccc1C(O)=O", "OC(=O)c1ccccc1O", "c1cccc(C(O)=O)c1O"];
    for route in v["routes"].as_array().unwrap() {
        for step in route["steps"].as_array().unwrap() {
            for precursor in step["precursors"].as_array().unwrap() {
                assert!(!banned_forms.contains(&precursor.as_str().unwrap()));
            }
        }
    }

    let inline = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "3",
        "--ban-smiles",
        SALICYLIC_ACID_ALT_SPELLING,
    ]);
    assert_eq!(inline["routes"], v["routes"]);
}

#[test]
fn ban_list_validation() {
    let err = run_failure(&["--target", ASPIRIN, "--ban-smiles", ASPIRIN]);
    assert!(err.contains("target itself is in the banned-molecule list"));
    let err = run_failure(&["--target", ASPIRIN, "--ban-smiles", "not((smiles"]);
    assert!(err.contains("invalid banned molecule"));
}

#[test]
fn max_bb_price_filters_stock_csv() {
    let stock = temp_file(
        "stock.csv",
        "smiles,name,vendor,price_jpy\nCC(=O)O,acetic acid,v,100\nOc1ccccc1C(=O)O,salicylic acid,v,5000\nCC=O,acetaldehyde,v,\n",
    );
    let cheap = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "2",
        "--stock",
        stock.to_str().unwrap(),
        "--max-bb-price",
        "1000",
    ]);
    let filter = &cheap["stock_price_filter"];
    assert_eq!(filter["entries_excluded"], 1);
    assert_eq!(filter["entries_kept"], 2);
    assert_eq!(filter["unpriced_entries_kept"], 1);
    assert_eq!(
        cheap["routes_found"], 0,
        "salicylic acid is now too expensive"
    );

    let generous = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "2",
        "--stock",
        stock.to_str().unwrap(),
        "--max-bb-price",
        "10000",
    ]);
    assert_eq!(generous["stock_price_filter"]["entries_excluded"], 0);
    assert!(generous["routes_found"].as_u64().unwrap() > 0);

    let err = run_failure(&["--target", ASPIRIN, "--max-bb-price", "5"]);
    assert!(err.contains("--max-bb-price requires --stock"));
}

#[test]
fn capabilities_advertise_new_options() {
    let v = run(&["capabilities", "--output", "json"]);
    for key in [
        "max_expansions",
        "first_route_stats",
        "banned_molecules",
        "max_bb_price",
    ] {
        assert_eq!(v["search"][key], true, "{key}");
    }
}

#[test]
fn route_diversity_reports_packing_number() {
    let v = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "3",
        "--max-routes",
        "8",
        "--route-diversity",
    ]);
    let routes = v["routes"].as_array().unwrap();
    let d = &v["route_set_diversity"];
    assert_eq!(d["distance"], "reaction_jaccard");
    assert_eq!(d["routes"], routes.len());
    assert_eq!(d["method"], "exact");
    let packing = d["packing_number"].as_u64().unwrap() as usize;
    assert!(packing >= 1 && packing <= routes.len());
    assert_eq!(
        d["packing_route_indices"].as_array().unwrap().len(),
        packing
    );

    let loose = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "3",
        "--max-routes",
        "8",
        "--diversity-radius",
        "0",
    ]);
    assert!(
        loose["route_set_diversity"]["packing_number"]
            .as_u64()
            .unwrap() as usize
            >= packing
    );

    let err = run_failure(&["--target", ASPIRIN, "--diversity-radius", "1"]);
    assert!(err.contains("--diversity-radius must be a number in [0,1)"));
    let err = run_failure(&["--target", ASPIRIN, "--route-diversity", "--format", "tree"]);
    assert!(err.contains("require --format json"));
}

#[test]
fn max_branching_reports_pruned_candidates() {
    let v = run(&["--target", ASPIRIN, "--depth", "3", "--max-branching", "1"]);
    assert_eq!(v["max_branching"]["limit"], 1);
    assert!(v["max_branching"]["candidates_pruned"].as_u64().unwrap() > 0);
    let err = run_failure(&["--target", ASPIRIN, "--max-branching", "0"]);
    assert!(err.contains("--max-branching must be a positive integer"));
}
