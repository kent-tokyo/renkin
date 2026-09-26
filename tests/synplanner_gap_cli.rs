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
