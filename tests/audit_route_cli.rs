//! Process-level tests for `renkin audit-route` (RENKIN Bridge PR5): spawn
//! the real `renkin` binary twice per test -- once to generate a real route
//! JSON file via the standard search path, once to audit it -- so this
//! exercises the actual round-trip through the CLI's real `--format json`
//! output shape, not a hand-authored fixture that could silently drift from
//! what the CLI really emits. Mirrors `tests/coverage_mode_cli.rs`'s
//! spawn-the-real-binary convention.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn renkin")
}

/// A path under the system temp dir unique to this call -- `cargo test`
/// runs tests in parallel *threads within one process*, so `std::process::id()`
/// alone collides across concurrently-running tests; combine it with a
/// per-call atomic counter instead.
fn unique_temp_path(label: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "renkin_audit_route_{label}_{}_{n}.json",
        std::process::id()
    ))
}

/// Generates a real route-search JSON output file for `target` and returns
/// its path. `CCOC(=O)c1ccccc1` at `--depth 1` reliably solves via
/// `co_aliphatic_cleavage` (a SMIRKS-based hand-crafted rule, so its steps
/// are forward-replayable) against the repo's own `data/building_blocks.smi`
/// default stock -- same target/depth already established as fast and
/// reliable elsewhere in this Bridge program's own tests.
fn generate_route_fixture() -> std::path::PathBuf {
    let out = run(&["--target", "CCOC(=O)c1ccccc1", "--depth", "1"]);
    assert!(
        out.status.success(),
        "route generation must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = unique_temp_path("fixture");
    std::fs::write(&path, &out.stdout).expect("failed to write route fixture");
    path
}

#[test]
fn passes_when_stock_and_forward_replay_succeed() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--stock",
        "data/building_blocks.smi",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "a Pass/Fail/Partial verdict must never be a nonzero exit: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["source_format"], "renkin");
    assert!(report["summary"]["routes_total"].as_u64().unwrap() > 0);
    assert!(
        report["routes"][0]["stock_validation"]["status"] == "pass",
        "{report}"
    );
    std::fs::remove_file(&route_path).ok();
}

/// v0.27.0 "Reproducible Route Audit": `audit_manifest` must record real,
/// verifiable provenance -- not just be present. Recomputes the expected
/// `input_sha256` independently (same recipe as `input_content_sha256` in
/// `main.rs`, over the exact bytes on disk that were audited) rather than
/// only checking the field exists, so a manifest hashing the wrong thing
/// (or a hardcoded placeholder) would fail this test.
#[test]
fn audit_manifest_records_verifiable_reproducibility_metadata() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--stock",
        "data/building_blocks.smi",
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    let manifest = &report["audit_manifest"];

    assert_eq!(manifest["report_schema_version"], report["schema_version"]);
    assert_eq!(manifest["source_format"], report["source_format"]);
    assert_eq!(manifest["source_version"], serde_json::Value::Null);
    assert_eq!(manifest["policy"], "standard");
    assert!(
        !manifest["renkin_version"].as_str().unwrap_or("").is_empty(),
        "{report}"
    );

    let raw_bytes = std::fs::read(&route_path).unwrap();
    let expected_input_sha256 = format!("sha256:{}", sha256_hex(&raw_bytes));
    assert_eq!(manifest["input_sha256"], expected_input_sha256, "{report}");

    let stock_sha256 = manifest["stock_sha256"]
        .as_str()
        .expect("stock_sha256 must be present when --stock was given");
    assert!(stock_sha256.starts_with("sha256:"), "{report}");

    std::fs::remove_file(&route_path).ok();
}

#[test]
fn stock_sha256_is_null_without_stock_flag() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(
        report["audit_manifest"]["stock_sha256"],
        serde_json::Value::Null,
        "{report}"
    );
    std::fs::remove_file(&route_path).ok();
}

/// v0.27.0 P0 item 4: same input audited twice must produce identical
/// output. Audits the SAME already-generated route file twice (rather than
/// regenerating the route each time), so this isolates `audit-route`'s own
/// determinism from the separate question of search determinism.
#[test]
fn auditing_the_same_input_twice_is_byte_identical() {
    let route_path = generate_route_fixture();
    let args = [
        "audit-route",
        route_path.to_str().unwrap(),
        "--stock",
        "data/building_blocks.smi",
        "--output",
        "json",
    ];
    let out1 = run(&args);
    let out2 = run(&args);
    assert!(out1.status.success() && out2.status.success());
    assert_eq!(
        out1.stdout, out2.stdout,
        "auditing identical input twice must be byte-identical"
    );
    std::fs::remove_file(&route_path).ok();
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn partial_without_stock_reports_stock_not_provided_not_a_silent_pass() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(
        report["routes"][0]["stock_validation"]["status"],
        "not_evaluable"
    );
    assert_eq!(
        report["routes"][0]["stock_validation"]["reason"],
        "stock_not_provided"
    );
    assert_ne!(report["routes"][0]["status"], "pass");
    std::fs::remove_file(&route_path).ok();
}

#[test]
fn human_output_is_readable_and_stdout_only_carries_the_report() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--stock",
        "data/building_blocks.smi",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("routes audited"), "{stdout}");
    assert!(stdout.contains("route 1/"), "{stdout}");
    // stdout must never contain raw JSON braces in human mode -- confirms
    // the human/json output paths are genuinely distinct, not one always
    // printing JSON regardless of --output.
    assert!(!stdout.trim_start().starts_with('{'), "{stdout}");
    std::fs::remove_file(&route_path).ok();
}

#[test]
fn empty_routes_input_is_handled_gracefully() {
    // A real CLI "no routes found" JSON shape, not hand-authored: a
    // deliberately absurd target with a tiny depth guarantees zero routes.
    let out = run(&[
        "--target",
        "c1ccccc1CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC",
        "--depth",
        "1",
    ]);
    assert!(out.status.success());
    let path = unique_temp_path("empty");
    std::fs::write(&path, &out.stdout).unwrap();

    let out = run(&["audit-route", path.to_str().unwrap(), "--output", "json"]);
    assert!(out.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["summary"]["routes_total"], 0);
    assert_eq!(report["routes"].as_array().unwrap().len(), 0);
    std::fs::remove_file(&path).ok();
}

#[test]
fn rejects_unsupported_format() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--format",
        "aizynthfinder",
    ]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--format"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&route_path).ok();
}

#[test]
fn rejects_unreadable_path() {
    let out = run(&["audit-route", "/nonexistent/path/route.json"]);
    assert!(!out.status.success());
}

#[test]
fn rejects_malformed_json_with_context() {
    // RENKIN Bridge PR6: format auto-detection parses a generic
    // `serde_json::Value` first, so syntactically invalid JSON now fails at
    // that stage with "not valid JSON" -- before any format-specific
    // (RENKIN/AiZynthFinder) deserialize ever runs.
    let path = unique_temp_path("malformed");
    std::fs::write(&path, "not json").unwrap();
    let out = run(&["audit-route", path.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not valid JSON"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn rejects_valid_json_that_fails_the_renkin_schema_with_context() {
    // Valid JSON with the top-level shape `detect_audit_route_format` reads
    // as RENKIN (`target` + `routes` keys present), but `routes` isn't the
    // expected array-of-route-entries shape -- must fail at the
    // RENKIN-specific deserialize step with schema-specific context, not a
    // generic JSON-syntax error.
    let path = unique_temp_path("bad_renkin_schema");
    std::fs::write(&path, r#"{"target": "CCO", "routes": "not an array"}"#).unwrap();
    let out = run(&["audit-route", path.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not a recognized RENKIN route JSON"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&path).ok();
}

/// v0.27.0 P0 item 2 (adapter conformance): unknown/future fields at every
/// level of a RENKIN route JSON must be silently ignored, never a parse
/// error -- the same forward-compatibility contract
/// `bridge::aizynthfinder`'s own module docs already state for the
/// AiZynthFinder side (`AzfNode` has no `deny_unknown_fields` either).
/// Confirmed empirically (not just asserted from reading the derive), by
/// process-spawning the real CLI against real extra fields at the
/// top-level, route-entry, and step levels simultaneously.
#[test]
fn unknown_extra_fields_in_renkin_input_are_tolerated_not_rejected() {
    let path = unique_temp_path("unknown_fields");
    std::fs::write(
        &path,
        r#"{
            "target": "CCO",
            "a_field_from_a_future_renkin_version": 123,
            "routes": [{
                "steps": [{
                    "target": "CCO",
                    "precursors": ["CC=O"],
                    "template_id": "test_reduction",
                    "an_unexpected_step_field": true
                }],
                "building_blocks": ["CC=O"],
                "an_unexpected_entry_field": ["x", "y"]
            }]
        }"#,
    )
    .unwrap();
    let out = run(&["audit-route", path.to_str().unwrap(), "--output", "json"]);
    assert!(
        out.status.success(),
        "unknown fields must not be a parse error: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["summary"]["routes_total"], 1, "{report}");
    std::fs::remove_file(&path).ok();
}

/// v0.27.0 P0 item 2: valid JSON that matches none of the recognized
/// shapes (RENKIN, AiZynthFinder single-target, AiZynthFinder batch) must
/// be a clean, explicit `--format auto` detection failure -- never a guess
/// at which format was probably meant. Distinct from
/// `rejects_valid_json_that_fails_the_renkin_schema_with_context` above:
/// that case DOES match the RENKIN shape (has `target`+`routes` keys) and
/// fails at the RENKIN-specific deserialize step; this one matches nothing
/// at the detection step itself.
#[test]
fn rejects_ambiguous_input_matching_no_known_format() {
    let path = unique_temp_path("ambiguous_format");
    std::fs::write(&path, r#"{"foo": "bar"}"#).unwrap();
    let out = run(&["audit-route", path.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("could not identify"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&path).ok();
}

// ── v0.29.0 Audit Policy Profiles (PR2: CLI --policy) ────────────────────

#[test]
fn rejects_unsupported_policy() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--policy",
        "bogus",
    ]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--policy"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&route_path).ok();
}

/// Same not-evaluable-only fixture as
/// `partial_without_stock_reports_stock_not_provided_not_a_silent_pass`
/// (no `--stock` given at all) -- confirms `--policy strict` hardens this
/// from `partial` to `fail`, per the published policy table, while
/// `informational`/`standard` (the default) both stay `partial`.
#[test]
fn policy_strict_hardens_not_evaluable_only_to_fail() {
    let route_path = generate_route_fixture();
    for (policy, expected) in [
        (None, "partial"),
        (Some("informational"), "partial"),
        (Some("standard"), "partial"),
        (Some("strict"), "fail"),
    ] {
        let mut args = vec![
            "audit-route",
            route_path.to_str().unwrap(),
            "--output",
            "json",
        ];
        if let Some(p) = policy {
            args.push("--policy");
            args.push(p);
        }
        let out = run(&args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: serde_json::Value =
            serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
        assert_eq!(
            report["routes"][0]["status"], expected,
            "policy {policy:?}: {report}"
        );
        assert_eq!(
            report["audit_manifest"]["policy"],
            policy.unwrap_or("standard"),
            "audit_manifest.policy must record the policy actually used"
        );
    }
    std::fs::remove_file(&route_path).ok();
}

/// A gating finding present (stock configured but missing the real leaf)
/// -- confirms `--policy informational` softens `fail` to `partial` while
/// `standard` (the default) and `strict` both stay `fail`.
#[test]
fn policy_informational_softens_gating_finding_to_partial() {
    let route_path = generate_route_fixture();
    let empty_stock_path = unique_temp_path("empty_stock");
    std::fs::write(&empty_stock_path, "CCO ethanol_only_not_the_real_leaves\n").unwrap();

    for (policy, expected) in [
        (None, "fail"),
        (Some("informational"), "partial"),
        (Some("standard"), "fail"),
        (Some("strict"), "fail"),
    ] {
        let mut args = vec![
            "audit-route",
            route_path.to_str().unwrap(),
            "--stock",
            empty_stock_path.to_str().unwrap(),
            "--output",
            "json",
        ];
        if let Some(p) = policy {
            args.push("--policy");
            args.push(p);
        }
        let out = run(&args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: serde_json::Value =
            serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
        assert_eq!(
            report["routes"][0]["status"], expected,
            "policy {policy:?}: {report}"
        );
        // Findings must be identical regardless of policy -- only status differs.
        assert!(
            report["routes"][0]["findings"]
                .as_array()
                .is_some_and(|f| !f.is_empty()),
            "expected at least one gating finding: {report}"
        );
    }
    std::fs::remove_file(&route_path).ok();
    std::fs::remove_file(&empty_stock_path).ok();
}

// ── Syntheseus Bridge, Phase 2 (v0.30.0) -- real, committed Phase 0
// fixtures, not hand-authored JSON. See
// tests/fixtures/syntheseus/0.7.2/PROVENANCE.md for exact provenance. ──

#[test]
fn syntheseus_explicit_format_audits_the_real_linear_fixture() {
    let out = run(&[
        "audit-route",
        "tests/fixtures/syntheseus/0.7.2/linear_two_leaf_route.json",
        "--format",
        "syntheseus",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["audit_manifest"]["source_format"], "syntheseus");
    assert_eq!(report["audit_manifest"]["source_version"], "0.7.2");
    assert_eq!(report["routes"][0]["source"], "syntheseus");
    // No stock given -> not_evaluable -> partial, never a silent pass.
    assert_eq!(report["routes"][0]["status"], "partial");
}

#[test]
fn syntheseus_auto_detected_via_source_tool_field() {
    let out = run(&[
        "audit-route",
        "tests/fixtures/syntheseus/0.7.2/linear_two_leaf_route.json",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["audit_manifest"]["source_format"], "syntheseus");
}

#[test]
fn syntheseus_stock_matching_the_real_leaves_passes_stock_validation() {
    let stock_path = unique_temp_path("syntheseus_stock");
    std::fs::write(&stock_path, "CCO ethanol\nO=C(O)c1ccccc1 benzoic_acid\n").unwrap();
    let out = run(&[
        "audit-route",
        "tests/fixtures/syntheseus/0.7.2/linear_two_leaf_route.json",
        "--format",
        "syntheseus",
        "--stock",
        stock_path.to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["routes"][0]["stock_validation"]["status"], "pass");
    std::fs::remove_file(&stock_path).ok();
}

#[test]
fn syntheseus_convergent_fixtures_ambiguous_leaf_fails_with_two_findings() {
    // Fixture B's own CC leaf genuinely has no is_purchasable claim --
    // duplicated once per parent by build()'s duplication-on-flatten
    // (docs/design/syntheseus-bridge-v0.md sec 7.1's resolved open
    // question), so it must surface twice, not be deduplicated away.
    let out = run(&[
        "audit-route",
        "tests/fixtures/syntheseus/0.7.2/convergent_route.json",
        "--format",
        "syntheseus",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["routes"][0]["status"], "fail");
    let codes: Vec<&str> = report["routes"][0]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        vec!["ambiguous_leaf_status", "ambiguous_leaf_status"],
        "{report}"
    );
}

// ── SynPlanner Bridge, Phase 1 PR3 -- real, committed fixtures from both
// Phase 0 (hand-constructed chython reactions run through SynPlanner's own
// real exporter) and Phase 1 PR1.5 (a real CPU-only MCTS-searched planning
// run through the real `synplan planning` CLI end to end). See
// tests/fixtures/synplanner/v1.6.0/{PROVENANCE.md,real_planning_route.PROVENANCE.md}
// for exact provenance. ──

#[test]
fn synplanner_explicit_format_audits_the_real_two_step_planning_route() {
    let out = run(&[
        "audit-route",
        "tests/fixtures/synplanner/v1.6.0/real_planning_route_2step.json",
        "--format",
        "synplanner",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["audit_manifest"]["source_format"], "synplanner");
    assert_eq!(report["routes"][0]["source"], "syn_planner");
    // No stock given -> not_evaluable -> partial, never a silent pass.
    assert_eq!(report["routes"][0]["status"], "partial");
}

#[test]
fn synplanner_auto_detected_via_route_id_keyed_object_shape() {
    let out = run(&[
        "audit-route",
        "tests/fixtures/synplanner/v1.6.0/real_planning_route_1step.json",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["audit_manifest"]["source_format"], "synplanner");
}

#[test]
fn synplanner_stock_matching_the_real_leaves_passes_stock_validation() {
    let stock_path = unique_temp_path("synplanner_stock");
    // Both real precursors of route "2" (aspirin via acetic-anhydride
    // acylation of salicylic acid) in real_planning_route_1step.json.
    std::fs::write(
        &stock_path,
        "CC(=O)OC(C)=O acetic_anhydride\nO=C(O)c1ccccc1O salicylic_acid\n",
    )
    .unwrap();
    let out = run(&[
        "audit-route",
        "tests/fixtures/synplanner/v1.6.0/real_planning_route_1step.json",
        "--format",
        "synplanner",
        "--stock",
        stock_path.to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["routes"][0]["stock_validation"]["status"], "pass");
    std::fs::remove_file(&stock_path).ok();
}

#[test]
fn synplanner_real_planning_reactions_genuinely_pass_forward_validation() {
    // The headline Phase 1 PR1.5 finding, reconfirmed here at the CLI
    // level: unlike AiZynthFinder/Syntheseus routes (always not_evaluable
    // in this codebase today), a real MCTS-searched SynPlanner route's
    // atom-mapped reaction smiles genuinely replays -- both steps PASS,
    // not just "isn't reported as missing_atom_mapping".
    let out = run(&[
        "audit-route",
        "tests/fixtures/synplanner/v1.6.0/real_planning_route_2step.json",
        "--format",
        "synplanner",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    let steps = report["routes"][0]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "{report}");
    for step in steps {
        assert_eq!(step["forward_validation"]["status"], "pass", "{report}");
    }
}

#[test]
fn synplanner_synthetic_missing_in_stock_field_is_ambiguous_leaf_not_guessed() {
    // Per docs/design/synplanner-adapter-v1.md sec 7 item 6's resolved
    // decision: real SynPlanner output never omits in_stock (confirmed at
    // 167-route scale in Phase 1 PR1.5), but the parser must still handle
    // a genuinely-missing field defensively rather than assume it can't
    // happen. This JSON is hand-authored and NOT real SynPlanner output --
    // unlike every other test in this section, it isn't sliced from a
    // committed fixture, deliberately, so it's never mistaken for one.
    let route_path = unique_temp_path("synplanner_synthetic_ambiguous");
    std::fs::write(
        &route_path,
        r#"{"1":{"type":"mol","smiles":"CCO","in_stock":false,"children":[{"type":"reaction","smiles":"[C:1][O:2]>>[C:1].[O:2]","children":[{"type":"mol","smiles":"CC"},{"type":"mol","smiles":"O"}]}]}}"#,
    )
    .unwrap();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--format",
        "synplanner",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["routes"][0]["status"], "fail", "{report}");
    let codes: Vec<&str> = report["routes"][0]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"ambiguous_leaf_status"), "{report}");
    std::fs::remove_file(&route_path).ok();
}
