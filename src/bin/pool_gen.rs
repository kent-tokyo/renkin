#![forbid(unsafe_code)]

//! Issue #101 Phase 3B: candidate-pool generation driver.
//!
//! `src/pool_export.rs` deliberately has no driver of its own ("this
//! module writes rows/manifests; it does not decide *which* targets to
//! run... that is a driver's responsibility, kept out of this crate
//! deliberately" -- its own module doc). This binary is that driver: reads
//! a `{group_id, target_id}` group list (produced by
//! `scripts/generate_real_labels.py --groups-output` /
//! `scripts/generate_train_val_labels.py --{train,val}-groups-output`,
//! deliberately never a labels file -- a pool-generation driver must never
//! see ground truth, only the proposal/label separation the whole
//! candidate-pool design rests on), runs `propose_one_step(Exhaustive)`
//! for each group, and writes the candidate JSONL, group/target index
//! JSONL, and `PoolManifest` `src/pool_export.rs` defines.
//!
//! `--limit N` takes the first N groups (input file order, so the run is
//! deterministic and reproducible) -- for 100/500-target feasibility
//! staging (Issue #101 Phase 3B/3C), not a formal-scale run.
//!
//! Usage:
//!   cargo build --release --bin renkin-pool-gen
//!   ./target/release/renkin-pool-gen \
//!       --groups data/reranker_groups_uspto50k_test.jsonl \
//!       --templates data/templates_extracted_500.smi \
//!       --pool-output data/pool_test_100.jsonl \
//!       --groups-output data/groups_test_100.jsonl \
//!       --manifest-output data/manifest_test_100.json \
//!       --limit 100

use renkin::candidate::{
    CandidateProposalContext, ProposalConfig, propose_phase_nanos, reset_propose_phase_nanos,
};
use renkin::chem_env::{load_rules_from_file, mol_from_smiles};
use renkin::pool_export::{
    PoolProvenance, build_manifest, candidate_rows_for_pool, target_pool_record_for_failure,
    target_pool_record_for_pool, target_pool_record_for_target_id_mismatch, write_jsonl,
    write_target_pool_jsonl,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter};
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct GroupInput {
    group_id: String,
    target_id: String,
}

fn arg_value(flag: &str, default: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

fn arg_opt(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn sha256_of_file(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    format!("sha256:{}", renkin::sha256_hex(Sha256::digest(&bytes)))
}

/// Parses `chematic`'s pinned version out of `Cargo.lock` (`[[package]]
/// name = "chematic"` ... `version = "X.Y.Z"`) rather than reporting
/// renkin's own crate version under a `chematic_version` field, which
/// would be exactly the kind of silently-wrong provenance `PoolProvenance`
/// exists to prevent.
fn chematic_version_from_lockfile(lockfile: &str) -> String {
    let mut lines = lockfile.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() == "[[package]]" {
            let mut name = None;
            let mut version = None;
            while let Some(&next) = lines.peek() {
                if next.trim() == "[[package]]" || next.trim().is_empty() {
                    break;
                }
                let next = lines.next().unwrap();
                if let Some(v) = next
                    .strip_prefix("name = \"")
                    .and_then(|s| s.strip_suffix('"'))
                {
                    name = Some(v.to_string());
                } else if let Some(v) = next
                    .strip_prefix("version = \"")
                    .and_then(|s| s.strip_suffix('"'))
                {
                    version = Some(v.to_string());
                }
            }
            if name.as_deref() == Some("chematic") {
                return version.unwrap_or_default();
            }
        }
    }
    String::new()
}

fn percentile(sorted: &[usize], pct: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() - 1) as f64 * pct).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn percentile_f64(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * pct).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn main() {
    let start = Instant::now();

    let groups_path = arg_value("--groups", "data/reranker_groups_uspto50k_test.jsonl");
    let templates_path = arg_value("--templates", "data/templates_extracted_500.smi");
    let pool_output = arg_value("--pool-output", "data/pool_gen_output.jsonl");
    let groups_output = arg_value("--groups-output", "data/pool_gen_groups.jsonl");
    let manifest_output = arg_value("--manifest-output", "data/pool_gen_manifest.json");
    let limit: Option<usize> =
        arg_opt("--limit").map(|s| s.parse().expect("--limit must be an integer"));

    let group_inputs: Vec<GroupInput> = {
        let file = File::open(&groups_path).unwrap_or_else(|e| panic!("open {groups_path}: {e}"));
        let mut rows: Vec<GroupInput> = BufReader::new(file)
            .lines()
            .map(|l| l.unwrap())
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(&l).unwrap_or_else(|e| panic!("parse {l:?}: {e}")))
            .collect();
        if let Some(n) = limit {
            rows.truncate(n);
        }
        rows
    };
    let n_targets_requested = group_inputs.len();

    let rules = load_rules_from_file(&templates_path);
    eprintln!("loaded {} rules from {templates_path}", rules.len());
    eprintln!("processing {n_targets_requested} group(s) from {groups_path}");

    let ctx = CandidateProposalContext::new(&rules, false);
    let config = ProposalConfig::default();
    let templates_by_id = renkin::candidate::index_rules_by_template_id(&rules)
        .expect("rules must have consistent template_id -> rule mapping");
    reset_propose_phase_nanos();

    let mut candidate_rows = Vec::new();
    let mut group_records = Vec::new();
    let mut candidate_counts: Vec<usize> = Vec::new();
    let mut n_parse_failed = 0usize;
    let mut n_zero_candidate = 0usize;
    let mut n_target_id_mismatch = 0usize;
    // Per-target propose_one_step wall-clock, for p50/p95 proposal-cost
    // reporting (Phase B.1 speed gate) -- driver-level timing, not gated
    // behind perf-instrumentation, since this loop is a one-shot CLI tool,
    // not the search hot path.
    let mut per_target_seconds: Vec<f64> = Vec::new();

    for (i, g) in group_inputs.iter().enumerate() {
        if i % 500 == 0 && i > 0 {
            eprintln!("  {i}/{n_targets_requested}...");
        }
        // The group's own target_id is the canonical SMILES text an upstream
        // label generator computed. propose_one_step re-derives target_id
        // internally via its own canonicalization call, which normally just
        // reconfirms that same canonical form -- but for a rare molecule
        // this CAN disagree (observed, root-caused in Phase 3D.5: to_canonical
        // is not a pure function of the graph -- a Molecule rebuilt via
        // clear_atom_maps/MoleculeBuilder and a Molecule parsed fresh from
        // that same rebuild's own canonical SMILES text can land on two
        // different, individually-stable canonical forms for the same
        // molecule; no atom maps are involved in the second, divergent
        // step). Never trust pool.target_id silently here -- compare
        // it against the caller's own g.target_id and reject the group
        // (not just "note" it) on any mismatch, so this class of defect is
        // caught at export time instead of surfacing later as an opaque
        // load_split_manifest/label_and_split_rows failure.
        match mol_from_smiles(&g.target_id) {
            Err(_) => {
                n_parse_failed += 1;
                group_records.push(target_pool_record_for_failure(&g.group_id, &g.target_id));
            }
            Ok(target_mol) => {
                let t_target = Instant::now();
                let result = ctx.propose_one_step(&g.group_id, &g.target_id, &config);
                per_target_seconds.push(t_target.elapsed().as_secs_f64());
                match result {
                    Err(e) => {
                        n_parse_failed += 1;
                        eprintln!("  {}: propose_one_step error: {e}", g.group_id);
                        group_records
                            .push(target_pool_record_for_failure(&g.group_id, &g.target_id));
                    }
                    Ok(pool) if pool.target_id != g.target_id => {
                        n_target_id_mismatch += 1;
                        eprintln!(
                            "  {}: target_id mismatch -- requested {:?}, propose_one_step derived {:?}",
                            g.group_id, g.target_id, pool.target_id
                        );
                        group_records.push(target_pool_record_for_target_id_mismatch(
                            &g.group_id,
                            &g.target_id,
                        ));
                    }
                    Ok(pool) => {
                        if pool.candidates.is_empty() {
                            n_zero_candidate += 1;
                        }
                        candidate_counts.push(pool.candidates.len());
                        group_records.push(target_pool_record_for_pool(&pool));
                        let rows =
                            candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);
                        candidate_rows.extend(rows);
                    }
                }
            }
        }
    }

    candidate_rows.sort_by(|a, b| {
        (a.group_id.as_str(), a.candidate_id.as_str())
            .cmp(&(b.group_id.as_str(), b.candidate_id.as_str()))
    });

    let pool_file =
        File::create(&pool_output).unwrap_or_else(|e| panic!("create {pool_output}: {e}"));
    let candidate_jsonl_sha256 =
        write_jsonl(&candidate_rows, BufWriter::new(pool_file)).expect("write pool jsonl");

    let groups_file =
        File::create(&groups_output).unwrap_or_else(|e| panic!("create {groups_output}: {e}"));
    let target_group_index_sha256 =
        write_target_pool_jsonl(&group_records, BufWriter::new(groups_file))
            .expect("write group index jsonl");

    let renkin_git_commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let provenance = PoolProvenance {
        renkin_git_commit,
        cargo_lock_sha256: sha256_of_file("Cargo.lock"),
        chematic_version: chematic_version_from_lockfile(
            &std::fs::read_to_string("Cargo.lock").unwrap_or_default(),
        ),
        target_input_sha256: sha256_of_file(&groups_path),
        stock_source: None,
        embedded_fallback_used: false,
        export_config: serde_json::json!({
            "groups_path": groups_path,
            "templates_path": templates_path,
            "limit": limit,
            "proposal_mode": "exhaustive",
        }),
    };

    let manifest = build_manifest(
        &candidate_rows,
        &candidate_jsonl_sha256,
        &group_records,
        &target_group_index_sha256,
        &rules,
        &config.mode,
        None,
        provenance,
    )
    .expect("build manifest");

    let manifest_json = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
    std::fs::write(&manifest_output, &manifest_json)
        .unwrap_or_else(|e| panic!("write {manifest_output}: {e}"));

    candidate_counts.sort_unstable();
    per_target_seconds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let elapsed = start.elapsed();
    let phase_nanos = propose_phase_nanos();

    let feasibility_summary = serde_json::json!({
        "n_groups_requested": n_targets_requested,
        "n_groups_parse_failed": n_parse_failed,
        "n_groups_target_id_mismatch": n_target_id_mismatch,
        "n_groups_zero_candidates": n_zero_candidate,
        "n_candidate_rows": candidate_rows.len(),
        "candidates_per_group_p50": percentile(&candidate_counts, 0.50),
        "candidates_per_group_p90": percentile(&candidate_counts, 0.90),
        "candidates_per_group_p95": percentile(&candidate_counts, 0.95),
        "candidates_per_group_max": candidate_counts.last().copied().unwrap_or(0),
        "wall_clock_seconds": elapsed.as_secs_f64(),
        "pool_output": pool_output,
        "groups_output": groups_output,
        "manifest_output": manifest_output,
        "candidate_jsonl_sha256": candidate_jsonl_sha256,
        "target_group_index_sha256": target_group_index_sha256,
        "proposal_mode": "exhaustive",
        // Only non-zero with --features perf-instrumentation; see
        // candidate::propose_phase_nanos doc for what each bucket covers.
        "propose_phase_seconds": {
            "select": phase_nanos.select as f64 / 1e9,
            "raw_propose": phase_nanos.raw_propose as f64 / 1e9,
            "merge": phase_nanos.merge as f64 / 1e9,
        },
        "proposal_seconds_per_target_p50": percentile_f64(&per_target_seconds, 0.50),
        "proposal_seconds_per_target_p95": percentile_f64(&per_target_seconds, 0.95),
        "proposal_seconds_per_target_max": per_target_seconds.last().copied().unwrap_or(0.0),
    });
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&feasibility_summary).unwrap()
    );
}
