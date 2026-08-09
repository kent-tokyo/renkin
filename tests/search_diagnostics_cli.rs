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

// ── Phase 1B: candidate-level trace (--candidate-trace-limit) ────────────

#[test]
fn candidate_trace_absent_without_the_flag() {
    let v = run(&[
        "--target",
        BUILDING_BLOCK,
        "--depth",
        "2",
        "--max-routes",
        "1",
        "--search-diagnostics",
    ]);
    let sd = v.get("search_diagnostics").unwrap();
    let trace = sd
        .get("candidate_trace")
        .expect("candidate_trace key must still be present (empty array)");
    assert_eq!(
        trace.as_array().unwrap().len(),
        0,
        "no records collected without --candidate-trace-limit"
    );
    for key in [
        "candidates_generated_before_dedup",
        "candidates_after_same_template_dedup",
        "candidates_after_cross_template_dedup",
    ] {
        assert!(sd.get(key).is_some(), "missing aggregate field {key}");
    }
}

#[test]
fn candidate_trace_limit_implies_search_diagnostics_and_bounds_record_count() {
    // ASPIRIN needs more than one rule application to reach a building block;
    // a tiny cap (2) must never be exceeded even though many more candidates
    // are generated during this search.
    let v = run(&[
        "--target",
        ASPIRIN,
        "--depth",
        "2",
        "--max-routes",
        "1",
        "--candidate-trace-limit",
        "2",
    ]);
    let sd = v
        .get("search_diagnostics")
        .expect("--candidate-trace-limit must imply --search-diagnostics");
    let trace = sd["candidate_trace"].as_array().unwrap();
    assert!(
        trace.len() <= 2,
        "cap must never be exceeded, got {}",
        trace.len()
    );
    assert!(
        !trace.is_empty(),
        "aspirin's search must generate candidates"
    );
    for record in trace {
        for key in [
            "depth",
            "parent_smiles",
            "template_id",
            "rule_name",
            "provenance",
            "precursor_signature",
            "f_score",
            "survived_beam",
            "later_reached_stock",
        ] {
            assert!(record.get(key).is_some(), "missing field {key} in {record}");
        }
    }
}

#[test]
fn candidate_trace_limit_missing_value_is_hard_error() {
    let out = std::process::Command::new(bin())
        .args(["--target", ASPIRIN, "--candidate-trace-limit"])
        .output()
        .expect("failed to spawn renkin");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--candidate-trace-limit"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
