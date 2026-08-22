//! v0.32.0 Phase 2B: AiZynthFinder version matrix. Confirms `renkin
//! audit-route --format aizynthfinder` handles real, artifact-captured
//! output from all three individually-verified versions (4.3.2, 4.4.0,
//! 4.4.1) identically -- same route shapes, same target molecules, same
//! search, so any divergence here would mean a real cross-version JSON
//! shape incompatibility, not a fixture inconsistency.
//!
//! Unlike the Syntheseus compat matrix (`.github/workflows/ci.yml`'s
//! `syntheseus-compat-matrix` job), this needs no live `aizynthfinder`
//! package installed anywhere: RENKIN never imports AiZynthFinder code,
//! it only parses AiZynthFinder's own JSON export format, which is
//! already frozen as committed fixtures (see each version directory's
//! `PROVENANCE.md`). So this runs as a normal `cargo test`, in the same
//! CI job as everything else -- no new CI job needed.
//!
//! Spawns the real `renkin` binary, same convention as
//! `tests/audit_route_cli.rs`.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin")
}

fn fixture_path(version: &str, filename: &str) -> String {
    format!(
        "{}/tests/fixtures/aizynthfinder/{version}/{filename}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn run_audit(path: &str, extra_args: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "audit-route",
        path,
        "--format",
        "aizynthfinder",
        "--output",
        "json",
    ];
    args.extend_from_slice(extra_args);
    let out = Command::new(bin())
        .args(&args)
        .output()
        .expect("failed to spawn renkin");
    assert!(
        out.status.success(),
        "audit-route must succeed for {path}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{path}: invalid JSON output: {e}"))
}

const VERSIONS: [&str; 3] = ["v4.3.2", "v4.4.0", "v4.4.1"];

#[test]
fn single_trees_without_stock_is_identical_partial_verdict_across_all_verified_versions() {
    for version in VERSIONS {
        let path = fixture_path(version, "single_trees.json");
        let report = run_audit(&path, &[]);
        let routes = report["routes"].as_array().expect("routes array");
        assert_eq!(routes.len(), 3, "{version}: expected 3 kept routes");
        for (i, route) in routes.iter().enumerate() {
            assert_eq!(
                route["status"], "partial",
                "{version}: route {i} must be partial (no stock configured)"
            );
            assert_eq!(
                route["stock_validation"]["status"], "not_evaluable",
                "{version}: route {i} stock_validation"
            );
            assert_eq!(
                route["stock_validation"]["reason"], "stock_not_provided",
                "{version}: route {i} stock_validation reason"
            );
            assert_eq!(
                route["route_tree_parseable"], true,
                "{version}: route {i} must parse structurally"
            );
        }
    }
}

/// With `data/building_blocks.smi` configured, route index 1 (the middle
/// of the 3 kept routes) resolves both its leaves and PASSes; routes 0 and
/// 2 have at least one leaf claimed purchasable that isn't actually in
/// this stock, so they FAIL with `LeafClaimedStockNotMatched` -- the same
/// pattern independently confirmed for all three versions (manually, via
/// the real CLI, before writing this test).
#[test]
fn single_trees_with_stock_produces_the_same_pass_fail_pattern_across_all_verified_versions() {
    let stock_path = format!("{}/data/building_blocks.smi", env!("CARGO_MANIFEST_DIR"));
    for version in VERSIONS {
        let path = fixture_path(version, "single_trees.json");
        let report = run_audit(&path, &["--stock", &stock_path]);
        let routes = report["routes"].as_array().expect("routes array");
        assert_eq!(routes.len(), 3, "{version}: expected 3 kept routes");

        assert_eq!(routes[0]["status"], "fail", "{version}: route 0 must fail");
        let findings0: Vec<&str> = routes[0]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["code"].as_str().unwrap())
            .collect();
        assert!(
            findings0.contains(&"leaf_claimed_stock_not_matched"),
            "{version}: route 0 findings: {findings0:?}"
        );

        assert_eq!(routes[1]["status"], "pass", "{version}: route 1 must pass");
        assert!(
            routes[1]["findings"].as_array().unwrap().is_empty(),
            "{version}: route 1 must have no findings"
        );

        assert_eq!(routes[2]["status"], "fail", "{version}: route 2 must fail");
        let findings2: Vec<&str> = routes[2]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["code"].as_str().unwrap())
            .collect();
        assert!(
            findings2.contains(&"leaf_claimed_stock_not_matched"),
            "{version}: route 2 findings: {findings2:?}"
        );
    }
}

#[test]
fn batch_output_gz_parses_identically_across_all_verified_versions() {
    for version in VERSIONS {
        let path = fixture_path(version, "batch_output.json.gz");
        let report = run_audit(&path, &[]);
        let routes = report["routes"].as_array().expect("routes array");
        // Each target keeps its first 2 trimmed routes (PROVENANCE.md) --
        // benzocaine (solved) + ibuprofen (not solved) = 4 total routes.
        assert_eq!(
            routes.len(),
            4,
            "{version}: expected 4 kept routes across 2 targets"
        );
        for (i, route) in routes.iter().enumerate() {
            assert_eq!(
                route["route_tree_parseable"], true,
                "{version}: batch route {i} must parse structurally"
            );
        }
    }
}
