//! Process-level tests for `--search-diagnostics` (Issue #101): spawn the
//! real `renkin` binary and check actual stdout JSON, not just library
//! internals -- this is the only place that exercises `main.rs`'s wiring
//! (the `Output` struct field and the `diagnose()`-branch json! insertion).

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

const ASPIRIN: &str = "CC(=O)Oc1ccccc1C(=O)O";
const BUILDING_BLOCK: &str = "CC(=O)O"; // acetic acid: a depth-0 (routes_found>0) target

#[test]
fn default_output_omits_search_diagnostics_when_route_found() {
    let v = run(&[
        "--target",
        BUILDING_BLOCK,
        "--depth",
        "2",
        "--max-routes",
        "1",
    ]);
    assert!(
        v.get("search_diagnostics").is_none(),
        "search_diagnostics must be absent by default: {v}"
    );
}

#[test]
fn default_output_omits_search_diagnostics_when_no_route_found() {
    // depth=1 vs a stock containing only water: exercises the routes.is_empty() branch.
    let v = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "1",
        "--max-routes",
        "1",
        "--building-blocks",
        "/dev/null",
    ]);
    assert_eq!(v["routes_found"], 0);
    assert!(
        v.get("search_diagnostics").is_none(),
        "search_diagnostics must be absent by default even on the diagnostics branch: {v}"
    );
    // The pre-existing "diagnostics" block (unrelated key) must be untouched.
    assert!(v.get("diagnostics").is_some());
}

#[test]
fn search_diagnostics_flag_adds_block_when_route_found() {
    let v = run(&[
        "--target",
        BUILDING_BLOCK,
        "--depth",
        "2",
        "--max-routes",
        "1",
        "--search-diagnostics",
    ]);
    let sd = v
        .get("search_diagnostics")
        .expect("search_diagnostics must be present with the flag");
    for key in [
        "beam_prune_invocations",
        "candidates_evicted_total",
        "rules_attempted_total",
        "cross_template_duplicate_precursor_signatures",
        "stock_terminal_candidates",
        "non_stock_candidates",
        "branching_by_depth",
    ] {
        assert!(sd.get(key).is_some(), "missing field {key} in {sd}");
    }
}

#[test]
fn search_diagnostics_flag_adds_block_when_no_route_found() {
    let v = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "1",
        "--max-routes",
        "1",
        "--building-blocks",
        "/dev/null",
        "--search-diagnostics",
    ]);
    assert_eq!(v["routes_found"], 0);
    assert!(v.get("search_diagnostics").is_some());
    // Pre-existing diagnostics block must still be present and unchanged in shape.
    assert!(v["diagnostics"].get("nodes_expanded").is_some());
}
