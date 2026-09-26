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
