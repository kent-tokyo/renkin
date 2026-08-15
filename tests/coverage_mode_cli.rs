//! Process-level tests for `--search-mode coverage` (Phase 41.18B): spawn
//! the real `renkin` binary and check actual stdout/stderr/exit code, not
//! just library internals -- mirrors `tests/search_diagnostics_cli.rs`'s
//! convention.
//!
//! Deliberately does not use `data/phase_a5_template_scaling/templates/
//! templates_2000.smi` (the real Phase B.2 coverage template set) here --
//! that run takes ~10-20s per invocation even in release mode, too slow to
//! run twice per test across a dozen tests in CI. Manual verification
//! against the real 2,000-template set and a real Phase B.1/B.2 target
//! (`uspto50k_val#L2628`, the fastest known newly-solved target in the
//! committed decision-run data) is recorded in the PR description instead.
//!
//! Uses `tests/fixtures/coverage_mode_templates.smi` -- a small, committed,
//! self-contained fixture (see that file's own header comment for
//! provenance) -- rather than `data/templates_extracted_500.smi`, which is
//! excluded from `cargo package` via the pre-existing
//! `data/templates_extracted*.smi` glob. `tests/fixtures/**` is NOT
//! excluded, so these integration tests remain self-contained in a
//! published crate tarball, not just a git checkout (verified: `cargo
//! package --list` includes both this file and the fixture).

use std::io::Write;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn renkin")
}

fn run_json(args: &[&str]) -> serde_json::Value {
    let out = run(args);
    assert!(
        out.status.success(),
        "renkin exited non-zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout must be valid JSON ({e}): {out:?}"))
}

const BUILDING_BLOCK: &str = "CC(=O)O"; // acetic acid: depth-0 building block
// Unsolved by default_rules() alone at depth=2/beam_width=100, solved once
// the fixture template file loads (either as --templates or
// --coverage-templates) -- see tests/fixtures/coverage_mode_templates.smi's
// own header comment for exactly which real extracted-template lines this
// is and why. NOT solvable by default_rules() alone at deeper/unlimited-
// beam settings either (unlike coverage_mode.rs's own unit-test fixture
// choice for a *different* target, which had to switch configs for that
// reason) -- always paired with --depth 2 --beam-width 100 in this file.
const STAGE1_UNSOLVED_AT_FIXTURE: &str = "O=C1CCC(=O)N1c1ccccc1";
// Unsolved by default_rules() even with the fixture template added --
// exercises the "both stages fail" no-route JSON branch.
const BOTH_STAGES_UNSOLVED: &str = "c1ccc2c(c1)c1ccccc1c1ccccc21"; // pyrene
const FIXTURE_TEMPLATES: &str = "tests/fixtures/coverage_mode_templates.smi";

const COVERAGE_MODE_KEYS: &[&str] = &[
    "search_mode",
    "selected_stage",
    "stage2_invoked",
    "stage1_timeout",
    "stage2_timeout",
    "stage1_elapsed_ms",
    "stage2_elapsed_ms",
    "total_elapsed_ms",
];

fn assert_no_coverage_mode_keys(v: &serde_json::Value, context: &str) {
    for key in COVERAGE_MODE_KEYS {
        assert!(
            v.get(key).is_none(),
            "standard mode must never emit {key} ({context}): {v}"
        );
    }
}

#[test]
fn standard_mode_route_found_output_has_no_coverage_mode_keys() {
    let v = run_json(&[
        "--target",
        BUILDING_BLOCK,
        "--depth",
        "2",
        "--max-routes",
        "1",
    ]);
    assert_no_coverage_mode_keys(&v, "route-found branch");
    // Exact pre-existing key set, unchanged.
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "joint_success_probability",
            "routes",
            "routes_found",
            "target"
        ]
    );
}

// Requirement: standard-mode output is exactly byte-compatible. An earlier
// version of this test file only pinned the route-found branch's key set;
// review confirmed (by mutation -- adding all 7 coverage-mode keys
// unconditionally to the no-routes JSON branch in src/main.rs) that
// nothing in the workspace test suite would have caught a leak on this
// branch. This test closes that gap.
#[test]
fn standard_mode_no_route_found_output_has_no_coverage_mode_keys() {
    let v = run_json(&[
        "--target",
        BOTH_STAGES_UNSOLVED,
        "--depth",
        "1",
        "--max-routes",
        "1",
    ]);
    assert_eq!(v["routes_found"], 0);
    assert_no_coverage_mode_keys(&v, "no-route-found branch");
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["diagnostics", "routes", "routes_found", "target"]
    );
}

#[test]
fn invalid_search_mode_fails() {
    let out = run(&["--target", BUILDING_BLOCK, "--search-mode", "bogus"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--search-mode"), "stderr: {stderr}");
}

#[test]
fn missing_coverage_templates_path_fails_before_search() {
    let out = run(&["--target", BUILDING_BLOCK, "--search-mode", "coverage"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--coverage-templates"), "stderr: {stderr}");
}

#[test]
fn nonexistent_coverage_templates_path_fails_before_search() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        "/nonexistent/renkin_coverage_mode_test_path.smi",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does not exist or is not readable"),
        "stderr: {stderr}"
    );
}

// Requirement: an unreadable (not just missing) coverage-templates path
// fails loud, with a message describing a *read* failure, not "contains
// no valid templates" -- a different, misleading failure mode an earlier
// version of this test did not actually exercise (it tested a nonexistent
// path, already covered by the test above). Uses invalid UTF-8 bytes
// rather than `chmod 000`: deterministic regardless of who runs the test
// (root, and some CI runners, can still read a mode-000 file; no user can
// make `std::fs::read_to_string` accept non-UTF-8 content).
#[test]
fn unreadable_coverage_templates_path_reports_a_read_failure_not_missing_templates() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "renkin_coverage_mode_cli_test_invalid_utf8_{}.smi",
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&[0xFF, 0xFE, 0x00, 0xFF, 0xD8, 0x00]).unwrap();
    }
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        path.to_str().unwrap(),
    ]);
    let _ = std::fs::remove_file(&path);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("could not be read as valid UTF-8"),
        "stderr must describe a read failure: {stderr}"
    );
    assert!(
        !stderr.contains("contains no valid templates"),
        "must not be misreported as the empty-templates case: {stderr}"
    );
}

#[test]
fn coverage_flags_in_standard_mode_fail() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--coverage-templates",
        FIXTURE_TEMPLATES,
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--search-mode coverage"),
        "stderr: {stderr}"
    );

    let out = run(&["--target", BUILDING_BLOCK, "--coverage-timeout-secs", "10"]);
    assert!(!out.status.success());
}

#[test]
fn coverage_timeout_zero_fails() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        FIXTURE_TEMPLATES,
        "--coverage-timeout-secs",
        "0",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--coverage-timeout-secs"),
        "stderr: {stderr}"
    );
}

#[test]
fn coverage_timeout_non_integer_fails() {
    for bad in ["abc", "12.5", "-3"] {
        let out = run(&[
            "--target",
            BUILDING_BLOCK,
            "--search-mode",
            "coverage",
            "--coverage-templates",
            FIXTURE_TEMPLATES,
            "--coverage-timeout-secs",
            bad,
        ]);
        assert!(!out.status.success(), "expected failure for {bad:?}");
    }
}

#[test]
fn bond_index_with_coverage_mode_fails_loud() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        FIXTURE_TEMPLATES,
        "--bond-index",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bond-index") || stderr.contains("bond_index"),
        "stderr: {stderr}"
    );
}

// Requirement: an active --ring-context-policy fails loud in coverage
// mode. Uses a deliberately nonexistent --ring-context-sidecar path: the
// combination must be rejected by flag presence *before* this process
// ever tries to load a real sidecar file, so this test needs no real
// ring-context fixture asset at all -- exactly the point of moving this
// check ahead of the sidecar-loading code in src/main.rs. An earlier
// version of this rejection had zero test coverage anywhere despite an
// (empty) unit test function claiming otherwise.
#[test]
fn ring_context_policy_with_coverage_mode_fails_loud() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        FIXTURE_TEMPLATES,
        "--ring-context-policy",
        "conservative",
        "--ring-context-sidecar",
        "/nonexistent/renkin_coverage_mode_test_sidecar.json",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does not support an active --ring-context-policy"),
        "must be the coverage-mode combination rejection specifically: {stderr}"
    );
    assert!(
        !stderr.contains("failed to load ring-context sidecar"),
        "must be rejected by the coverage-mode combination check, before ever attempting to \
         load the (deliberately nonexistent) sidecar file -- that would be a different error \
         message (the pre-existing --ring-context-sidecar load-failure path): {stderr}"
    );
}

// Requirement: an ONNX --scorer fails loud in coverage mode. Only
// meaningful (the --scorer flag only exists at all) when built with the
// nn-scoring feature -- CI's default `cargo test --workspace` doesn't
// enable it, so this runs as part of `cargo test --features nn-scoring`
// instead. Same "nonexistent path, never loaded" shape as the
// ring-context test above -- needs no real .onnx fixture.
#[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
#[test]
fn onnx_scorer_with_coverage_mode_fails_loud() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        FIXTURE_TEMPLATES,
        "--scorer",
        "/nonexistent/renkin_coverage_mode_test_scorer.onnx",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--scorer") || stderr.contains("scorer"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("scorer load error"),
        "must be rejected by the coverage-mode combination check, before ever attempting to \
         load the (deliberately nonexistent) .onnx file: {stderr}"
    );
}

#[test]
fn stage1_solved_reports_stage2_invoked_false() {
    let v = run_json(&[
        "--target",
        BUILDING_BLOCK,
        "--depth",
        "2",
        "--max-routes",
        "1",
        "--search-mode",
        "coverage",
        "--coverage-templates",
        FIXTURE_TEMPLATES,
    ]);
    assert_eq!(v["routes_found"], 1);
    assert_eq!(v["search_mode"], "coverage");
    assert_eq!(v["selected_stage"], "stage1");
    assert_eq!(v["stage2_invoked"], false);
    assert!(v.get("stage2_elapsed_ms").is_none() || v["stage2_elapsed_ms"].is_null());
}

#[test]
fn stage1_unsolved_reports_stage2_invoked_true() {
    let v = run_json(&[
        "--target",
        STAGE1_UNSOLVED_AT_FIXTURE,
        "--depth",
        "2",
        "--max-routes",
        "1",
        "--beam-width",
        "100",
        "--search-mode",
        "coverage",
        "--coverage-templates",
        FIXTURE_TEMPLATES,
    ]);
    assert_eq!(v["selected_stage"], "stage2");
    assert_eq!(v["stage2_invoked"], true);
    assert_eq!(v["routes_found"], 1);
    assert!(v["stage2_elapsed_ms"].as_f64().unwrap() >= 0.0);
    assert!(v["total_elapsed_ms"].as_f64().unwrap() >= v["stage1_elapsed_ms"].as_f64().unwrap());
}

#[test]
fn stage1_and_stage2_both_unsolved_reports_no_routes_with_coverage_fields() {
    // Exercises the routes.is_empty() JSON branch (raw json!(), not the
    // Output struct) with coverage-mode fields attached.
    let v = run_json(&[
        "--target",
        BOTH_STAGES_UNSOLVED,
        "--depth",
        "2",
        "--max-routes",
        "1",
        "--beam-width",
        "100",
        "--search-mode",
        "coverage",
        "--coverage-templates",
        FIXTURE_TEMPLATES,
    ]);
    assert_eq!(v["routes_found"], 0);
    assert_eq!(v["search_mode"], "coverage");
    assert_eq!(v["selected_stage"], "stage2");
    assert_eq!(v["stage2_invoked"], true);
    assert!(
        v.get("diagnostics").is_some(),
        "pre-existing diagnostics block must still be present: {v}"
    );
}
