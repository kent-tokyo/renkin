//! Process-level tests for `renkin-forward benchmark` (issue #61 PR A).
//!
//! `tests/fixtures/forward_bench_corpus.jsonl` is a small, hand-authored,
//! synthetic corpus -- see `tests/fixtures/README.md`. Every expected
//! `failure_reason` below was derived empirically by running this exact
//! harness against that exact fixture, not assumed from chemistry
//! intuition -- see that guide/README for why.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin-forward")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn renkin-forward")
}

fn fixture_corpus_path() -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/forward_bench_corpus.jsonl");
    p.to_str().unwrap().to_string()
}

fn temp_path(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "renkin-forward-bench-cli-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_str().unwrap().to_string()
}

fn read_rows(path: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).expect("each row must be valid JSON"))
        .collect()
}

/// Recursively zeroes every `elapsed_ms`/`latency_ms` value in place -- the
/// harness's only documented non-deterministic fields (see
/// `docs/guides/forward-benchmark.md`'s determinism section).
fn strip_timing(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if k == "elapsed_ms" || k == "latency_ms" {
                    *v = serde_json::Value::Null;
                } else {
                    strip_timing(v);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                strip_timing(v);
            }
        }
        _ => {}
    }
}

#[test]
fn benchmark_help_succeeds() {
    let out = run(&["benchmark", "--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--corpus"));
    assert!(stdout.contains("--output-rows"));
    assert!(stdout.contains("--template-source"));
}

#[test]
fn top_level_help_mentions_benchmark() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("benchmark"));
}

#[test]
fn benchmark_requires_corpus_and_output_rows() {
    let out = run(&["benchmark"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--corpus"));

    let out = run(&["benchmark", "--corpus", &fixture_corpus_path()]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--output-rows"));
}

#[test]
fn benchmark_rejects_missing_corpus_file() {
    let out = run(&[
        "benchmark",
        "--corpus",
        "/no/such/corpus.jsonl",
        "--output-rows",
        &temp_path("rows-missing-corpus.jsonl"),
    ]);
    assert!(!out.status.success());
}

#[test]
fn benchmark_rejects_scorer_conditioned_template_source() {
    let out = run(&[
        "benchmark",
        "--corpus",
        &fixture_corpus_path(),
        "--output-rows",
        &temp_path("rows-scorer.jsonl"),
        "--template-source",
        "scorer-conditioned",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Phase 3"));
}

#[test]
fn benchmark_rejects_templates_path_under_embedded_source() {
    let out = run(&[
        "benchmark",
        "--corpus",
        &fixture_corpus_path(),
        "--output-rows",
        &temp_path("rows-embedded-with-templates.jsonl"),
        "--templates",
        "some/path.smi",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("embedded"));
}

#[test]
fn benchmark_file_source_requires_templates_path() {
    let out = run(&[
        "benchmark",
        "--corpus",
        &fixture_corpus_path(),
        "--output-rows",
        &temp_path("rows-file-no-templates.jsonl"),
        "--template-source",
        "file",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--templates"));
}

#[test]
fn benchmark_file_source_uses_only_the_given_file_not_embedded_defaults() {
    let templates_path = temp_path("two_rules.smi");
    std::fs::write(
        &templates_path,
        "[O:3]=[C:2]-[OH:1]>>C-[O:1]-[C:2]=[O:3]\t1293\n[O:3]=[C:2]-[OH:1]>>C-C-[O:1]-[C:2]=[O:3]\t1057\n",
    )
    .unwrap();
    let rows_path = temp_path("rows-file-source.jsonl");
    let report_path = temp_path("report-file-source.json");

    let out = run(&[
        "benchmark",
        "--corpus",
        &fixture_corpus_path(),
        "--output-rows",
        &rows_path,
        "--output-report",
        &report_path,
        "--template-source",
        "file",
        "--templates",
        &templates_path,
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    // Exactly the 2 rules in the file -- never embedded-defaults-plus-file.
    assert_eq!(report["provenance"]["rules_loaded"], 2);
    assert_eq!(report["provenance"]["template_source"], "file");
    assert!(report["provenance"]["rules_file_sha256"].is_string());
}

#[test]
fn benchmark_fixture_corpus_counts_and_failure_reasons_match_expectations() {
    let rows_path = temp_path("rows-fixture.jsonl");
    let report_path = temp_path("report-fixture.json");

    let out = run(&[
        "benchmark",
        "--corpus",
        &fixture_corpus_path(),
        "--output-rows",
        &rows_path,
        "--output-report",
        &report_path,
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    let corpus_stats = &report["corpus_stats"];
    assert_eq!(corpus_stats["total_lines"], 14);
    assert_eq!(corpus_stats["blank_lines_skipped"], 1);
    assert_eq!(corpus_stats["malformed_json"], 1);
    assert_eq!(corpus_stats["wrong_schema_version"], 0);
    assert_eq!(corpus_stats["unparseable_smiles"], 1);
    assert_eq!(corpus_stats["empty_reactants_or_products"], 0);
    assert_eq!(corpus_stats["duplicate_records_merged"], 2);
    assert_eq!(corpus_stats["reactions_loaded"], 9);
    assert_eq!(corpus_stats["warnings_truncated"], false);

    let rows = read_rows(&rows_path);
    assert_eq!(rows.len(), 10);

    let mut expected: BTreeMap<&str, &str> = BTreeMap::new();
    expected.insert("hit-top1-ester-formation", "hit_top1");
    expected.insert("hit-top5-not-top1-amine-acylation", "hit_top5");
    expected.insert("hit-top10-not-top5-phenol-acylation", "hit_top10");
    expected.insert("hit-beyond-10-diacylation", "hit_beyond_10");
    expected.insert("stereochemistry-hit-top10", "hit_top10");
    expected.insert("stereo-mismatch-diagnostic", "correct_absent_nonempty_pool");
    expected.insert(
        "correct-absent-nonempty-pool",
        "correct_absent_nonempty_pool",
    );
    expected.insert(
        "correct-absent-empty-pool-suzuki",
        "correct_absent_empty_pool",
    );
    expected.insert("single-reactant-no-match", "correct_absent_empty_pool");
    expected.insert("input-invalid-bad-smiles", "input_invalid");
    assert_eq!(expected.len(), 10);

    for row in &rows {
        let reaction_id = row["reaction_id"].as_str().unwrap();
        let expected_reason = expected
            .get(reaction_id)
            .unwrap_or_else(|| panic!("unexpected reaction_id in output: {reaction_id}"));
        assert_eq!(
            row["failure_reason"].as_str().unwrap(),
            *expected_reason,
            "reaction_id={reaction_id}"
        );
    }

    // The stereochemistry-ignored diagnostic: this row's exact
    // (stereochemistry-aware) accepted product is absent, but the
    // stereo-flattened comparison finds it -- "constitution right,
    // stereochemistry wrong", the exact signal this dimension exists for.
    let mismatch_row = rows
        .iter()
        .find(|r| r["reaction_id"] == "stereo-mismatch-diagnostic")
        .unwrap();
    assert_eq!(mismatch_row["correct_candidate_present"], false);
    assert_eq!(mismatch_row["stereochemistry_aware_hit"], false);
    assert_eq!(mismatch_row["stereochemistry_ignored_hit"], true);
    assert!(mismatch_row["best_correct_rank_stereo_ignored"].is_number());

    // Split assignment is deterministic and every row lands somewhere
    // sensible: "unknown" only for the one row whose input never
    // canonicalized.
    for row in &rows {
        let split = row["split"].as_str().unwrap();
        let is_input_invalid = row["failure_reason"] == "input_invalid";
        if is_input_invalid {
            assert_eq!(split, "unknown");
        } else {
            assert!(
                ["train", "val", "test"].contains(&split),
                "unexpected split {split:?}"
            );
        }
    }
}

#[test]
fn benchmark_is_deterministic_modulo_timing_fields() {
    let rows_path_a = temp_path("rows-det-a.jsonl");
    let rows_path_b = temp_path("rows-det-b.jsonl");
    let report_path_a = temp_path("report-det-a.json");
    let report_path_b = temp_path("report-det-b.json");

    for (rows_path, report_path) in [
        (&rows_path_a, &report_path_a),
        (&rows_path_b, &report_path_b),
    ] {
        let out = run(&[
            "benchmark",
            "--corpus",
            &fixture_corpus_path(),
            "--output-rows",
            rows_path,
            "--output-report",
            report_path,
        ]);
        assert!(out.status.success());
    }

    let mut rows_a: Vec<serde_json::Value> = read_rows(&rows_path_a);
    let mut rows_b: Vec<serde_json::Value> = read_rows(&rows_path_b);
    for row in rows_a.iter_mut().chain(rows_b.iter_mut()) {
        strip_timing(row);
    }
    assert_eq!(
        rows_a, rows_b,
        "rows must be byte-identical modulo elapsed_ms"
    );

    let mut report_a: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path_a).unwrap()).unwrap();
    let mut report_b: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path_b).unwrap()).unwrap();
    strip_timing(&mut report_a);
    strip_timing(&mut report_b);
    assert_eq!(
        report_a, report_b,
        "report must be byte-identical modulo elapsed_ms/latency_ms"
    );
}

#[test]
fn benchmark_without_output_report_prints_report_json_to_stdout_only() {
    let rows_path = temp_path("rows-stdout-report.jsonl");
    let out = run(&[
        "benchmark",
        "--corpus",
        &fixture_corpus_path(),
        "--output-rows",
        &rows_path,
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["overall"].is_object());
}
