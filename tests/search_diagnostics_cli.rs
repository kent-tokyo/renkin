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

fn run_failure(args: &[&str]) -> String {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn renkin");
    assert!(!out.status.success(), "renkin unexpectedly succeeded");
    String::from_utf8_lossy(&out.stderr).into_owned()
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
fn malformed_numeric_limits_fail_closed_instead_of_using_defaults() {
    for flag in ["--depth", "--max-routes", "--beam-width", "--top-templates"] {
        let stderr = run_failure(&["--target", BUILDING_BLOCK, flag, "not-a-number"]);
        assert!(
            stderr.contains("must be a non-negative integer"),
            "{flag} must reject malformed numeric input: {stderr}"
        );
    }
}

#[test]
fn missing_numeric_limit_values_fail_closed() {
    for flag in ["--depth", "--max-routes", "--beam-width", "--top-templates"] {
        let stderr = run_failure(&["--target", BUILDING_BLOCK, flag]);
        assert!(
            stderr.contains("requires a value"),
            "{flag} must reject a missing value: {stderr}"
        );
    }
}

#[test]
fn missing_string_values_fail_closed() {
    for flag in [
        "--target",
        "--building-blocks",
        "--templates",
        "--format",
        "--constraints",
    ] {
        let stderr = run_failure(&[flag]);
        assert!(
            stderr.contains("requires a value"),
            "{flag} must reject a missing value: {stderr}"
        );
    }
}

#[test]
fn unknown_main_option_fails_closed() {
    let stderr = run_failure(&["--target", BUILDING_BLOCK, "--typo"]);
    assert!(
        stderr.contains("unknown option"),
        "unexpected stderr: {stderr}"
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

// ── --spectator-bond-policy (docs/design/spectator-bond-fail-closed-gating-v0.md) ──

/// A single-line templates file carrying the exact extracted_824 positive
/// control (Finding #4 / PR #186's own fixture): an oxazolidinone ring
/// whose C1-C5 ring-closing bond isn't declared by either the LHS or any
/// RHS fragment. `load_rules_from_file`'s two-column format is
/// `SMIRKS\tcount`; the loader assigns its own name/template_id, so the
/// finding is attributed to whatever it picks (checked by SMIRKS-derived
/// case/lost_bonds shape below, not by a hardcoded rule name).
fn write_extracted_824_templates_file() -> std::path::PathBuf {
    // Keyed by test name, not just PID: cargo's default test harness runs
    // every #[test] fn on its own thread within one process, so a
    // PID-only path collides across the concurrently-running tests in
    // this file (one test's cleanup racing another's still-in-flight
    // read) -- confirmed empirically, not assumed.
    let test_name = std::thread::current()
        .name()
        .unwrap_or("unknown")
        .replace("::", "_");
    let path = std::env::temp_dir().join(format!(
        "renkin_spectator_bond_cli_test_{}_{test_name}.smi",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "[C:5]-[O:6]-[C:3](=[O:4])-[NH:2]-[C:1]>>[C:1]-[N:2]=[C:3]=[O:4].[C:5]-[OH:6]\t824\n",
    )
    .unwrap();
    path
}

const OXAZOLIDINONE_TARGET: &str = "O=C2NCC(O2)Cc1ccccc1";

#[test]
fn spectator_bond_policy_defaults_to_off() {
    let templates = write_extracted_824_templates_file();
    let v = run(&[
        "--target",
        OXAZOLIDINONE_TARGET,
        "--depth",
        "1",
        "--templates",
        templates.to_str().unwrap(),
        "--search-diagnostics",
    ]);
    std::fs::remove_file(&templates).ok();
    let sd = v.get("search_diagnostics").expect("flag was passed");
    assert_eq!(
        sd["spectator_bond_loss_findings"].as_array().unwrap().len(),
        0,
        "policy Off must never run the detectors, even against a rule/target pair that would \
         flag if enabled: {sd}"
    );
    assert_eq!(sd["spectator_bond_gated_out"].as_array().unwrap().len(), 0);
}

#[test]
fn spectator_bond_policy_diagnostics_only_finds_but_never_excludes() {
    let templates = write_extracted_824_templates_file();
    let v = run(&[
        "--target",
        OXAZOLIDINONE_TARGET,
        "--depth",
        "1",
        "--templates",
        templates.to_str().unwrap(),
        "--spectator-bond-policy",
        "diagnostics-only",
        "--search-diagnostics",
    ]);
    std::fs::remove_file(&templates).ok();
    let sd = v.get("search_diagnostics").expect("flag was passed");
    let findings = sd["spectator_bond_loss_findings"].as_array().unwrap();
    assert_eq!(
        findings.len(),
        1,
        "the real extracted_824 defect must be detected through the CLI's own wiring, not just \
         library internals: {sd}"
    );
    assert_eq!(findings[0]["case"], "matched_pair_undeclared");
    assert_eq!(
        sd["spectator_bond_gated_out"].as_array().unwrap().len(),
        0,
        "diagnostics-only must never exclude a candidate"
    );
}

#[test]
fn spectator_bond_policy_gated_excludes_the_known_defect() {
    let templates = write_extracted_824_templates_file();
    let v = run(&[
        "--target",
        OXAZOLIDINONE_TARGET,
        "--depth",
        "1",
        "--templates",
        templates.to_str().unwrap(),
        "--spectator-bond-policy",
        "gated",
        "--search-diagnostics",
    ]);
    std::fs::remove_file(&templates).ok();
    let sd = v.get("search_diagnostics").expect("flag was passed");
    // Policy changes the verdict, never the finding set -- still recorded.
    assert_eq!(
        sd["spectator_bond_loss_findings"].as_array().unwrap().len(),
        1
    );
    let gated_out = sd["spectator_bond_gated_out"].as_array().unwrap();
    assert_eq!(
        gated_out.len(),
        1,
        "the known-defective candidate must be excluded under Gated: {sd}"
    );
    assert!(!gated_out[0]["findings"].as_array().unwrap().is_empty());
    assert!(
        !gated_out[0]["precursor_smiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn spectator_bond_policy_invalid_value_is_hard_error() {
    let out = std::process::Command::new(bin())
        .args(["--target", "CCO", "--spectator-bond-policy", "bogus"])
        .output()
        .expect("failed to spawn renkin");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--spectator-bond-policy"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn spectator_bond_policy_missing_value_is_hard_error() {
    let out = std::process::Command::new(bin())
        .args(["--target", "CCO", "--spectator-bond-policy"])
        .output()
        .expect("failed to spawn renkin");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--spectator-bond-policy"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── --element-accounting-policy (docs/design/candidate-time-element-accounting-gate-v0.md) ──

/// A single-line templates file carrying a deliberately defective retro
/// rule for test purposes only (same spirit as `candidate.rs`'s own
/// `debromination_retro_rule` unit-test fixture, now exercised through the
/// real CLI binary): drops the target's bromine entirely instead of
/// carrying it into a precursor fragment, a genuine target-element loss
/// `step_element_accounting` must catch.
fn write_debromination_templates_file() -> std::path::PathBuf {
    let test_name = std::thread::current()
        .name()
        .unwrap_or("unknown")
        .replace("::", "_");
    let path = std::env::temp_dir().join(format!(
        "renkin_element_accounting_cli_test_{}_{test_name}.smi",
        std::process::id()
    ));
    std::fs::write(&path, "[c:1]-[Br]>>[c:1]\t1\n").unwrap();
    path
}

const BROMOBENZENE_TARGET: &str = "Brc1ccccc1";

#[test]
fn element_accounting_policy_defaults_to_off() {
    let templates = write_debromination_templates_file();
    let v = run(&[
        "--target",
        BROMOBENZENE_TARGET,
        "--depth",
        "1",
        "--templates",
        templates.to_str().unwrap(),
        "--search-diagnostics",
    ]);
    std::fs::remove_file(&templates).ok();
    let sd = v.get("search_diagnostics").expect("flag was passed");
    assert_eq!(
        sd["element_accounting_gated_out"].as_array().unwrap().len(),
        0,
        "policy Off must never gate, even against a rule/target pair that would be caught if \
         enabled: {sd}"
    );
}

#[test]
fn element_accounting_policy_diagnostics_only_never_excludes() {
    let templates = write_debromination_templates_file();
    let v = run(&[
        "--target",
        BROMOBENZENE_TARGET,
        "--depth",
        "1",
        "--templates",
        templates.to_str().unwrap(),
        "--element-accounting-policy",
        "diagnostics-only",
        "--search-diagnostics",
    ]);
    std::fs::remove_file(&templates).ok();
    let sd = v.get("search_diagnostics").expect("flag was passed");
    assert_eq!(
        sd["element_accounting_gated_out"].as_array().unwrap().len(),
        0,
        "diagnostics-only must never exclude a candidate"
    );
}

#[test]
fn element_accounting_policy_gated_excludes_the_known_defect() {
    let templates = write_debromination_templates_file();
    let v = run(&[
        "--target",
        BROMOBENZENE_TARGET,
        "--depth",
        "1",
        "--templates",
        templates.to_str().unwrap(),
        "--element-accounting-policy",
        "gated",
        "--search-diagnostics",
    ]);
    std::fs::remove_file(&templates).ok();
    let sd = v.get("search_diagnostics").expect("flag was passed");
    let gated_out = sd["element_accounting_gated_out"].as_array().unwrap();
    assert_eq!(
        gated_out.len(),
        1,
        "the known-defective candidate must be excluded under Gated: {sd}"
    );
    // target_smiles is the search's own canonical form, not necessarily
    // byte-identical to the CLI's input string.
    assert!(
        gated_out[0]["target_smiles"]
            .as_str()
            .unwrap()
            .contains("Br")
    );
    assert!(
        !gated_out[0]["precursor_smiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn element_accounting_policy_invalid_value_is_hard_error() {
    let out = std::process::Command::new(bin())
        .args(["--target", "CCO", "--element-accounting-policy", "bogus"])
        .output()
        .expect("failed to spawn renkin");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--element-accounting-policy"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn element_accounting_policy_missing_value_is_hard_error() {
    let out = std::process::Command::new(bin())
        .args(["--target", "CCO", "--element-accounting-policy"])
        .output()
        .expect("failed to spawn renkin");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--element-accounting-policy"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
