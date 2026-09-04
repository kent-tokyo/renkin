//! Process-level tests for `renkin stock import` and `renkin doctor stock`
//! (v0.36.0 Phase 2 PR 2): spawn the real `renkin` binary so exit codes,
//! stdout/stderr separation, and on-disk artifacts are exercised exactly as
//! a third-party caller would see them, not just the in-process
//! `build_stock_doctor_report`/`stock_import_cli` logic already covered by
//! `src/main.rs`'s own `stock_import_cli_tests`. Mirrors
//! `tests/audit_route_cli.rs`'s spawn-the-real-binary convention.

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
/// per-call atomic counter (same convention as `tests/audit_route_cli.rs`).
fn unique_temp_path(label: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "renkin_stock_import_cli_{label}_{}_{n}",
        std::process::id()
    ))
}

fn unique_temp_path_with_extension(label: &str, extension: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "renkin_stock_import_cli_{label}_{}_{n}.{extension}",
        std::process::id()
    ))
}

fn write_input(label: &str, content: &str) -> std::path::PathBuf {
    let path = unique_temp_path(label);
    std::fs::write(&path, content).unwrap();
    path
}

// ── renkin stock compile ────────────────────────────────────────────────

#[test]
fn compiled_stock_loads_through_the_normal_building_blocks_flag() {
    let input = write_input(
        "compile_in.smi",
        "CCO ethanol\nOCC duplicate\nCC(=O)O acetic\n",
    );
    let output = unique_temp_path_with_extension("compile_out", "rstock");
    let compile = run(&[
        "stock",
        "compile",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&compile.stdout).unwrap();
    assert_eq!(summary["molecule_count"], 2);
    assert_eq!(summary["duplicate_rows"], 1);

    let search = run(&[
        "--target",
        "CCO",
        "--depth",
        "0",
        "--building-blocks",
        output.to_str().unwrap(),
    ]);
    assert!(
        search.status.success(),
        "{}",
        String::from_utf8_lossy(&search.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&search.stdout).unwrap();
    assert!(result["routes_found"].as_u64().unwrap() >= 1);
    assert_eq!(result["routes"][0]["steps"].as_array().unwrap().len(), 0);

    std::fs::remove_file(input).ok();
    std::fs::remove_file(output).ok();
}

#[test]
fn compile_refuses_existing_output_without_force() {
    let input = write_input("compile_refuse_in.smi", "CCO\n");
    let output = unique_temp_path_with_extension("compile_refuse_out", "rstock");
    std::fs::write(&output, "keep me\n").unwrap();
    let result = run(&[
        "stock",
        "compile",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("already exists"));
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "keep me\n");

    std::fs::remove_file(input).ok();
    std::fs::remove_file(output).ok();
}

#[test]
fn compile_fail_on_rejection_writes_auditable_artifact_then_fails() {
    let input = write_input("compile_reject_in.smi", "CCO\nnot(valid(((\n");
    let output = unique_temp_path_with_extension("compile_reject_out", "rstock");
    let result = run(&[
        "stock",
        "compile",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--fail-on-rejection",
    ]);
    assert!(!result.status.success());
    assert!(output.exists());
    assert!(String::from_utf8_lossy(&result.stderr).contains("1 rows were rejected"));

    std::fs::remove_file(input).ok();
    std::fs::remove_file(output).ok();
}

// ── renkin stock import ──────────────────────────────────────────────────

#[test]
fn import_round_trip_produces_valid_output_and_manifest() {
    let input = write_input(
        "roundtrip_in",
        "CCO ethanol\nOCC dup_of_ethanol\nCC(=O)O acetic\nnot(valid(((\n",
    );
    let output = unique_temp_path("roundtrip_out.smi");
    let manifest = unique_temp_path("roundtrip_out.manifest.json");

    let out = run(&[
        "stock",
        "import",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "test-fixture",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let summary: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(summary["manifest"]["input_rows"], 4);
    assert_eq!(summary["manifest"]["accepted_rows"], 3);
    assert_eq!(summary["manifest"]["rejected_rows"], 1);
    assert_eq!(summary["manifest"]["unique_structures"], 2);
    assert_eq!(summary["manifest"]["duplicate_rows"], 1);
    assert_eq!(summary["manifest"]["source"]["label"], "test-fixture");

    assert!(output.exists());
    assert!(manifest.exists());
    let stock_text = std::fs::read_to_string(&output).unwrap();
    assert_eq!(stock_text.lines().count(), 2);

    // Warnings go to stderr, never stdout.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("rejected"), "{stderr}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("warning:"));

    std::fs::remove_file(&input).ok();
    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&manifest).ok();
}

#[test]
fn repeated_invocation_is_byte_identical() {
    let input = write_input("repeat_in", "CCO ethanol\nCC(=O)O acetic\n");
    let output1 = unique_temp_path("repeat_out1.smi");
    let manifest1 = unique_temp_path("repeat_out1.manifest.json");
    let output2 = unique_temp_path("repeat_out2.smi");
    let manifest2 = unique_temp_path("repeat_out2.manifest.json");

    for (output, manifest) in [(&output1, &manifest1), (&output2, &manifest2)] {
        let out = run(&[
            "stock",
            "import",
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--source-label",
            "test-fixture",
        ]);
        assert!(out.status.success());
    }

    assert_eq!(
        std::fs::read(&output1).unwrap(),
        std::fs::read(&output2).unwrap(),
        "same input/options must produce byte-identical output stock"
    );
    // Manifests differ only in nothing -- same importer_version, same run,
    // so they must be fully byte-identical too (no timestamp field exists
    // to make them differ).
    assert_eq!(
        std::fs::read(&manifest1).unwrap(),
        std::fs::read(&manifest2).unwrap(),
        "same input/options must produce byte-identical manifests"
    );

    for p in [&input, &output1, &manifest1, &output2, &manifest2] {
        std::fs::remove_file(p).ok();
    }
}

#[test]
fn existing_output_is_refused_without_force() {
    let input = write_input("refuse_in", "CCO ethanol\n");
    let output = unique_temp_path("refuse_out.smi");
    let manifest = unique_temp_path("refuse_out.manifest.json");
    std::fs::write(&output, "pre-existing content\n").unwrap();

    let out = run(&[
        "stock",
        "import",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "test-fixture",
    ]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already exists"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Refused before any write -- the pre-existing content must be untouched.
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "pre-existing content\n"
    );
    assert!(!manifest.exists());

    std::fs::remove_file(&input).ok();
    std::fs::remove_file(&output).ok();
}

#[test]
fn existing_manifest_is_refused_without_force() {
    let input = write_input("refuse_manifest_in", "CCO ethanol\n");
    let output = unique_temp_path("refuse_manifest_out.smi");
    let manifest = unique_temp_path("refuse_manifest_out.manifest.json");
    std::fs::write(&manifest, "pre-existing manifest\n").unwrap();

    let out = run(&[
        "stock",
        "import",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "test-fixture",
    ]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already exists"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Refused before any write -- neither artifact touched, output never created.
    assert_eq!(
        std::fs::read_to_string(&manifest).unwrap(),
        "pre-existing manifest\n"
    );
    assert!(!output.exists());

    std::fs::remove_file(&input).ok();
    std::fs::remove_file(&manifest).ok();
}

#[test]
fn force_overwrites_existing_output() {
    let input = write_input("force_in", "CCO ethanol\n");
    let output = unique_temp_path("force_out.smi");
    let manifest = unique_temp_path("force_out.manifest.json");
    std::fs::write(&output, "pre-existing content\n").unwrap();

    let out = run(&[
        "stock",
        "import",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "test-fixture",
        "--force",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(
        std::fs::read_to_string(&output).unwrap(),
        "pre-existing content\n"
    );

    std::fs::remove_file(&input).ok();
    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&manifest).ok();
}

#[test]
fn same_input_and_output_path_is_rejected() {
    let path = write_input("same_path", "CCO ethanol\n");
    let manifest = unique_temp_path("same_path.manifest.json");

    let out = run(&[
        "stock",
        "import",
        "--input",
        path.to_str().unwrap(),
        "--output",
        path.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "test-fixture",
    ]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("same path"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Rejected before any write -- input file must be untouched.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "CCO ethanol\n");

    std::fs::remove_file(&path).ok();
}

#[test]
fn unparseable_rows_do_not_fail_by_default() {
    let input = write_input("unparseable_default", "CCO ethanol\nnot(valid(((\n");
    let output = unique_temp_path("unparseable_default_out.smi");
    let manifest = unique_temp_path("unparseable_default_out.manifest.json");

    let out = run(&[
        "stock",
        "import",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "test-fixture",
    ]);
    assert!(
        out.status.success(),
        "rejected rows alone must not fail the run by default: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    std::fs::remove_file(&input).ok();
    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&manifest).ok();
}

#[test]
fn fail_on_rejection_causes_nonzero_exit_but_keeps_artifacts() {
    let input = write_input("fail_on_rejection_in", "CCO ethanol\nnot(valid(((\n");
    let output = unique_temp_path("fail_on_rejection_out.smi");
    let manifest = unique_temp_path("fail_on_rejection_out.manifest.json");

    let out = run(&[
        "stock",
        "import",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "test-fixture",
        "--fail-on-rejection",
    ]);
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--fail-on-rejection"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The artifacts must still be written -- a policy failure, not an
    // import failure.
    assert!(output.exists(), "output artifact must still be written");
    assert!(manifest.exists(), "manifest artifact must still be written");

    std::fs::remove_file(&input).ok();
    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&manifest).ok();
}

#[test]
fn missing_required_flag_is_a_usage_error() {
    let out = run(&["stock", "import", "--source-label", "x"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--input"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── renkin doctor stock ──────────────────────────────────────────────────

fn import_fixture(label: &str, input_content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let input = write_input(&format!("{label}_in"), input_content);
    let output = unique_temp_path(&format!("{label}_out.smi"));
    let manifest = unique_temp_path(&format!("{label}_out.manifest.json"));
    let out = run(&[
        "stock",
        "import",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "doctor-fixture",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&input).ok();
    (output, manifest)
}

#[test]
fn doctor_stock_pass_exits_zero() {
    let (output, manifest) = import_fixture("doctor_pass", "CCO ethanol\nCC(=O)O acetic\n");

    let out = run(&[
        "doctor",
        "stock",
        "--stock",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("PASS") || stdout.contains("WARN"),
        "{stdout}"
    );
    assert!(!stdout.trim_start().starts_with('{'), "{stdout}");

    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&manifest).ok();
}

#[test]
fn doctor_stock_output_json_is_valid_and_reports_pass_overall_with_full_provenance() {
    let input = write_input("doctor_json_in", "CCO ethanol\nCC(=O)O acetic\n");
    let output = unique_temp_path("doctor_json_out.smi");
    let manifest = unique_temp_path("doctor_json_out.manifest.json");
    let import_out = run(&[
        "stock",
        "import",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "doctor-fixture",
        "--source-revision",
        "rev-1",
        "--license",
        "CC0",
    ]);
    assert!(import_out.status.success());

    let out = run(&[
        "doctor",
        "stock",
        "--stock",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["overall"], "pass", "{report}");
    let checks = report["checks"].as_array().unwrap();
    assert!(checks.iter().all(|c| c["severity"] == "pass"), "{report}");
    let names: Vec<&str> = checks.iter().map(|c| c["name"].as_str().unwrap()).collect();
    for expected in [
        "schema_version",
        "output_hash",
        "input_hash",
        "manifest_arithmetic",
        "stock_line_count",
        "reimport_idempotency",
        "normalization_contract",
        "importer_version",
        "source_provenance",
    ] {
        assert!(
            names.contains(&expected),
            "missing check {expected}: {report}"
        );
    }

    std::fs::remove_file(&input).ok();
    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&manifest).ok();
}

#[test]
fn doctor_stock_tampered_manifest_exits_one() {
    let (output, manifest) = import_fixture("doctor_fail", "CCO ethanol\n");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    value["output_sha256"] = serde_json::json!(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    );
    std::fs::write(&manifest, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let out = run(&[
        "doctor",
        "stock",
        "--stock",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(1));
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["overall"], "fail", "{report}");

    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&manifest).ok();
}

#[test]
fn doctor_stock_missing_manifest_flag_exits_two() {
    let out = run(&["doctor", "stock", "--stock", "data/building_blocks.smi"]);
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--manifest"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn doctor_stock_unreadable_stock_file_exits_two() {
    let manifest = unique_temp_path("doctor_unreadable.manifest.json");
    std::fs::write(&manifest, "{}").unwrap();
    let out = run(&[
        "doctor",
        "stock",
        "--stock",
        "/nonexistent/path/stock.smi",
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    std::fs::remove_file(&manifest).ok();
}

/// Real, already-committed `data/building_blocks.smi` (449 rows, 402
/// unique, 3 unparseable, 44 in-file duplicates -- see
/// `docs/design/stock-import-v0.md` §4) round-tripped through `stock
/// import` then `doctor stock`, read-only. Confirms the doctor's most
/// aggressive check -- `reimport_idempotency`, which re-canonicalizes the
/// entire output and requires a byte-identical, zero-rejection result --
/// actually holds on RENKIN's real production stock, not just small
/// hand-written fixtures. `data/building_blocks.smi` itself is never
/// written to; only read as `--input`.
#[test]
fn doctor_stock_on_the_real_building_blocks_fixture_passes_every_check_but_provenance() {
    let output = unique_temp_path("real_bb_out.smi");
    let manifest = unique_temp_path("real_bb_out.manifest.json");
    let import_out = run(&[
        "stock",
        "import",
        "--input",
        "data/building_blocks.smi",
        "--output",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--source-label",
        "repo data/building_blocks.smi",
    ]);
    assert!(
        import_out.status.success(),
        "{}",
        String::from_utf8_lossy(&import_out.stderr)
    );

    let out = run(&[
        "doctor",
        "stock",
        "--stock",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--input",
        "data/building_blocks.smi",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(
        report["overall"], "warn",
        "expected only the source_provenance WARN (no --source-revision/--license given): {report}"
    );
    for check in report["checks"].as_array().unwrap() {
        let name = check["name"].as_str().unwrap();
        if name == "source_provenance" {
            assert_eq!(check["severity"], "warn", "{report}");
        } else {
            assert_eq!(
                check["severity"], "pass",
                "check {name} was not pass: {report}"
            );
        }
    }

    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&manifest).ok();
}

#[test]
fn doctor_stock_rejects_unsupported_output_flag() {
    let (output, manifest) = import_fixture("doctor_bad_output_flag", "CCO ethanol\n");
    let out = run(&[
        "doctor",
        "stock",
        "--stock",
        output.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        "bogus",
    ]);
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&manifest).ok();
}
