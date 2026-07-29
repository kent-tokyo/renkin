//! Reproducible apply_retro/run_reactants performance-regression gate.
//!
//! Runs a fixed target corpus through `find_routes` with a pinned config and
//! reports, per target: elapsed time, `apply_retro` call count (always
//! available), `run_reactants`-level counters (only when built with
//! `--features perf-instrumentation`, else zero), route/candidate outcome, and
//! any error. Also emits SHA-256 provenance for every input file plus run
//! metadata (chematic pin, release/thread/OS info, warmup/measured counts) so
//! two runs (e.g. baseline vs. fix branch) can be compared without ambiguity
//! about what was actually measured.
//!
//! Not a pass/fail CI check (wall-clock varies run to run and machine to
//! machine) -- a deterministic data generator for a human/PR-body comparison.
//! See `artifacts/perf_root_cause/` for the investigation this gate exists to
//! re-run, and `next_gate.json` for one prior gate-criteria proposal.
//!
//! Usage:
//!   cargo run --release --example apply_retro_perf_gate -- \
//!     --targets artifacts/perf_root_cause/gate_inputs/probe_30_v2.smi \
//!     --templates data/templates_extracted_5000.smi \
//!     --stock data/building_blocks.smi \
//!     --depth 5 --beam-width 100 --warmup 1 --label "fix-branch"
//!
//! With run_reactants-level counters:
//!   cargo run --release --features perf-instrumentation --example apply_retro_perf_gate -- ...

use std::time::Instant;

use anyhow::{Context, Result, bail};
use renkin::chem_env::{
    ChemEnv, apply_retro_call_count, load_rules_from_file, reset_apply_retro_call_count,
};
use renkin::search::{SearchConfig, find_routes};
use sha2::{Digest, Sha256};

fn sha256_file(path: &str) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

/// The `[[package]]` fields this gate's provenance depends on, so a reader
/// doesn't have to trust a loose "pin" string -- these come straight from
/// Cargo.lock's own record of exactly what was resolved and downloaded.
#[derive(Debug, Clone, PartialEq)]
struct PackageMetadata {
    version: String,
    source: String,
    checksum: String,
}

/// Extract the content between the first pair of double quotes on a line
/// like `version = "0.8.0"`.
fn extract_quoted(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Find the single `[[package]]` block for `name` in a Cargo.lock's text and
/// return its `version`/`source`/`checksum` fields. Hard errors (rather than
/// falling back to "unknown") if the package isn't found, appears more than
/// once, or is missing any of the three fields -- a permanent regression
/// gate must not silently keep measuring against an unrecorded dependency.
fn parse_package_metadata(cargo_lock: &str, name: &str) -> Result<PackageMetadata> {
    let mut found: Option<PackageMetadata> = None;
    for block in cargo_lock.split("[[package]]").skip(1) {
        let block_name = block
            .lines()
            .find(|l| l.trim_start().starts_with("name ="))
            .and_then(extract_quoted);
        if block_name.as_deref() != Some(name) {
            continue;
        }
        if found.is_some() {
            bail!("Cargo.lock has more than one [[package]] block for `{name}`");
        }
        let version = block
            .lines()
            .find(|l| l.trim_start().starts_with("version ="))
            .and_then(extract_quoted)
            .with_context(|| format!("[[package]] `{name}` is missing a version field"))?;
        let source = block
            .lines()
            .find(|l| l.trim_start().starts_with("source ="))
            .and_then(extract_quoted)
            .with_context(|| format!("[[package]] `{name}` is missing a source field"))?;
        let checksum = block
            .lines()
            .find(|l| l.trim_start().starts_with("checksum ="))
            .and_then(extract_quoted)
            .with_context(|| format!("[[package]] `{name}` is missing a checksum field"))?;
        found = Some(PackageMetadata {
            version,
            source,
            checksum,
        });
    }
    found.with_context(|| format!("no [[package]] block found for `{name}` in Cargo.lock"))
}

#[cfg(feature = "perf-instrumentation")]
fn run_reactants_calls_delta() -> u64 {
    chematic_rxn::perf_counters::snapshot().run_reactants_calls
}
#[cfg(not(feature = "perf-instrumentation"))]
fn run_reactants_calls_delta() -> u64 {
    0
}
#[cfg(feature = "perf-instrumentation")]
fn reset_run_reactants_calls() {
    chematic_rxn::perf_counters::reset();
}
#[cfg(not(feature = "perf-instrumentation"))]
fn reset_run_reactants_calls() {}

#[derive(Debug, Clone, PartialEq)]
struct Args {
    targets: String,
    templates: String,
    stock: String,
    depth: u32,
    beam_width: usize,
    warmup: usize,
    label: String,
}

/// Parse a positive (non-zero) integer option value, hard-erroring with the
/// option name and the exact bad value on anything else -- a reproducibility
/// gate must not silently fall back to a default on a typo'd flag.
fn parse_positive_int<T>(option: &str, raw: &[String], i: usize) -> Result<T>
where
    T: std::str::FromStr + PartialEq + Default,
{
    let value = raw
        .get(i + 1)
        .with_context(|| format!("--{option} requires a value"))?;
    let parsed: T = value
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid --{option} value {value:?}: expected an integer"))?;
    if parsed == T::default() {
        bail!("invalid --{option} value {value:?}: must be a positive integer, got 0");
    }
    Ok(parsed)
}

fn parse_args_from(raw: &[String]) -> Result<Args> {
    let mut a = Args {
        targets: String::new(),
        templates: String::new(),
        stock: String::new(),
        depth: 5,
        beam_width: 100,
        warmup: 1,
        label: "unlabeled".to_string(),
    };
    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "--targets" => {
                a.targets = raw
                    .get(i + 1)
                    .with_context(|| "--targets requires a value")?
                    .clone();
                i += 2;
            }
            "--templates" => {
                a.templates = raw
                    .get(i + 1)
                    .with_context(|| "--templates requires a value")?
                    .clone();
                i += 2;
            }
            "--stock" => {
                a.stock = raw
                    .get(i + 1)
                    .with_context(|| "--stock requires a value")?
                    .clone();
                i += 2;
            }
            "--depth" => {
                a.depth = parse_positive_int("depth", raw, i)?;
                i += 2;
            }
            "--beam-width" => {
                a.beam_width = parse_positive_int("beam-width", raw, i)?;
                i += 2;
            }
            "--warmup" => {
                // --warmup 0 is explicitly allowed (skip the warmup pass
                // entirely), unlike depth/beam-width which must be positive.
                let value = raw
                    .get(i + 1)
                    .with_context(|| "--warmup requires a value")?;
                a.warmup = value.parse().map_err(|_| {
                    anyhow::anyhow!("invalid --warmup value {value:?}: expected an integer")
                })?;
                i += 2;
            }
            "--label" => {
                a.label = raw
                    .get(i + 1)
                    .with_context(|| "--label requires a value")?
                    .clone();
                i += 2;
            }
            other => bail!("unknown argument: {other}"),
        }
    }
    if a.targets.is_empty() || a.templates.is_empty() || a.stock.is_empty() {
        bail!("--targets, --templates, and --stock are all required");
    }
    Ok(a)
}

fn parse_args() -> Result<Args> {
    parse_args_from(&std::env::args().collect::<Vec<_>>())
}

fn main() -> Result<()> {
    let args = parse_args()?;

    let targets_sha256 = sha256_file(&args.targets)?;
    let templates_sha256 = sha256_file(&args.templates)?;
    let stock_sha256 = sha256_file(&args.stock)?;
    let cargo_lock_sha256 = sha256_file(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))?;
    let cargo_lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))?;
    let chematic_metadata = parse_package_metadata(&cargo_lock, "chematic")?;

    let env = ChemEnv::load(&args.stock)?;
    let stock_compound_count = env.bb_count();
    // ChemEnv::load reads exactly the path given (no fallback logic in this
    // crate) -- an explicit stock path was always used, so this is a fact
    // about this run, not an assumption.
    let embedded_fallback_used = false;

    let rules = load_rules_from_file(&args.templates);
    if rules.is_empty() {
        bail!(
            "loaded 0 templates from {} -- refusing to run a gate against an empty rule set",
            args.templates
        );
    }

    let targets: Vec<String> = std::fs::read_to_string(&args.targets)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().next().unwrap_or(l).to_string())
        .collect();

    let config = SearchConfig {
        max_depth: args.depth,
        max_routes: 5,
        beam_width: args.beam_width,
        verbose: false,
        ..Default::default()
    };

    let thread_count = std::env::var("RAYON_NUM_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        });

    // Warmup (not recorded): touches page cache / lazy-initialized state so the
    // first measured target isn't penalized for one-time setup cost.
    for smiles in targets.iter().take(args.warmup) {
        let _ = find_routes(smiles, &env, &rules, &config);
    }

    let mut per_target = Vec::with_capacity(targets.len());
    let overall_start = Instant::now();
    for (idx, smiles) in targets.iter().enumerate() {
        reset_apply_retro_call_count();
        reset_run_reactants_calls();
        let t0 = Instant::now();
        let outcome = find_routes(smiles, &env, &rules, &config);
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let apply_retro_calls = apply_retro_call_count();
        let run_reactants_calls = run_reactants_calls_delta();

        let record = match outcome {
            Ok((routes, stats)) => serde_json::json!({
                "index": idx,
                "target": smiles,
                "elapsed_ms": elapsed_ms,
                "apply_retro_calls": apply_retro_calls,
                "run_reactants_calls": run_reactants_calls,
                "matched_templates": stats.matched_templates,
                "retro_cache_hits": stats.retro_cache_hits,
                "retro_cache_misses": stats.retro_cache_misses,
                "nodes_expanded": stats.nodes_expanded,
                "routes_found": routes.len(),
                "solved": !routes.is_empty(),
                "best_route_depth": routes.first().map(|r| r.depth),
                "best_route_building_blocks": routes.first().map(|r| r.building_blocks.clone()),
                "error": null,
            }),
            Err(e) => serde_json::json!({
                "index": idx,
                "target": smiles,
                "elapsed_ms": elapsed_ms,
                "apply_retro_calls": apply_retro_calls,
                "run_reactants_calls": run_reactants_calls,
                "matched_templates": null,
                "retro_cache_hits": null,
                "retro_cache_misses": null,
                "nodes_expanded": null,
                "routes_found": 0,
                "solved": false,
                "best_route_depth": null,
                "best_route_building_blocks": null,
                "error": e.to_string(),
            }),
        };
        per_target.push(record);
    }
    let total_elapsed_s = overall_start.elapsed().as_secs_f64();

    let solved_count = per_target.iter().filter(|r| r["solved"] == true).count();
    let mut elapsed_ms: Vec<f64> = per_target
        .iter()
        .map(|r| r["elapsed_ms"].as_f64().unwrap_or(0.0))
        .collect();
    elapsed_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| -> f64 {
        if elapsed_ms.is_empty() {
            return 0.0;
        }
        let idx = ((elapsed_ms.len() as f64 - 1.0) * p).round() as usize;
        elapsed_ms[idx.min(elapsed_ms.len() - 1)]
    };

    let report = serde_json::json!({
        "label": args.label,
        "run_metadata": {
            "chematic": {
                "version": chematic_metadata.version,
                "source": chematic_metadata.source,
                "checksum": chematic_metadata.checksum,
            },
            "cargo_lock_sha256": cargo_lock_sha256,
            "targets_file": args.targets,
            "targets_sha256": targets_sha256,
            "templates_file": args.templates,
            "templates_sha256": templates_sha256,
            "stock_file": args.stock,
            "stock_sha256": stock_sha256,
            "stock_compound_count": stock_compound_count,
            "embedded_fallback_used": embedded_fallback_used,
            "rules_loaded": rules.len(),
            "release_mode": !cfg!(debug_assertions),
            "thread_count": thread_count,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "depth": args.depth,
            "beam_width": args.beam_width,
            "warmup_runs": args.warmup,
            "measured_runs": targets.len(),
        },
        "aggregate": {
            "total_elapsed_s": total_elapsed_s,
            "solved_count": solved_count,
            "target_count": targets.len(),
            "elapsed_ms_p50": pct(0.50),
            "elapsed_ms_p90": pct(0.90),
            "elapsed_ms_p95": pct(0.95),
            "elapsed_ms_max": elapsed_ms.last().copied().unwrap_or(0.0),
        },
        "per_target": per_target,
    });

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        std::iter::once("apply_retro_perf_gate".to_string())
            .chain(v.iter().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn valid_arguments() {
        let a = parse_args_from(&args(&[
            "--targets",
            "t.smi",
            "--templates",
            "r.smi",
            "--stock",
            "s.smi",
            "--depth",
            "7",
            "--beam-width",
            "50",
            "--warmup",
            "2",
            "--label",
            "my-run",
        ]))
        .unwrap();
        assert_eq!(a.targets, "t.smi");
        assert_eq!(a.templates, "r.smi");
        assert_eq!(a.stock, "s.smi");
        assert_eq!(a.depth, 7);
        assert_eq!(a.beam_width, 50);
        assert_eq!(a.warmup, 2);
        assert_eq!(a.label, "my-run");
    }

    #[test]
    fn missing_targets_value() {
        let err = parse_args_from(&args(&[
            "--templates",
            "r.smi",
            "--stock",
            "s.smi",
            "--targets",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("--targets requires a value"));
    }

    #[test]
    fn invalid_depth() {
        let err = parse_args_from(&args(&[
            "--targets",
            "t.smi",
            "--templates",
            "r.smi",
            "--stock",
            "s.smi",
            "--depth",
            "abc",
        ]))
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--depth"), "{msg}");
        assert!(msg.contains("abc"), "{msg}");
    }

    #[test]
    fn zero_depth() {
        let err = parse_args_from(&args(&[
            "--targets",
            "t.smi",
            "--templates",
            "r.smi",
            "--stock",
            "s.smi",
            "--depth",
            "0",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("--depth"));
    }

    #[test]
    fn invalid_beam_width() {
        let err = parse_args_from(&args(&[
            "--targets",
            "t.smi",
            "--templates",
            "r.smi",
            "--stock",
            "s.smi",
            "--beam-width",
            "wide",
        ]))
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--beam-width"), "{msg}");
        assert!(msg.contains("wide"), "{msg}");
    }

    #[test]
    fn zero_beam_width() {
        let err = parse_args_from(&args(&[
            "--targets",
            "t.smi",
            "--templates",
            "r.smi",
            "--stock",
            "s.smi",
            "--beam-width",
            "0",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("--beam-width"));
    }

    #[test]
    fn invalid_warmup() {
        let err = parse_args_from(&args(&[
            "--targets",
            "t.smi",
            "--templates",
            "r.smi",
            "--stock",
            "s.smi",
            "--warmup",
            "none",
        ]))
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--warmup"), "{msg}");
        assert!(msg.contains("none"), "{msg}");
    }

    #[test]
    fn zero_warmup_is_allowed() {
        let a = parse_args_from(&args(&[
            "--targets",
            "t.smi",
            "--templates",
            "r.smi",
            "--stock",
            "s.smi",
            "--warmup",
            "0",
        ]))
        .unwrap();
        assert_eq!(a.warmup, 0);
    }

    #[test]
    fn unknown_argument() {
        let err = parse_args_from(&args(&[
            "--targets",
            "t.smi",
            "--templates",
            "r.smi",
            "--stock",
            "s.smi",
            "--bogus",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("unknown argument: --bogus"));
    }

    const FIXTURE_LOCK: &str = r#"
# This file is automatically @generated by Cargo.
version = 4

[[package]]
name = "anyhow"
version = "1.0.104"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "deadbeef"

[[package]]
name = "chematic"
version = "0.8.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc123"
dependencies = [
 "chematic-core",
]

[[package]]
name = "chematic-core"
version = "0.8.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "def456"
"#;

    #[test]
    fn package_metadata_extracts_chematic_fields() {
        let m = parse_package_metadata(FIXTURE_LOCK, "chematic").unwrap();
        assert_eq!(m.version, "0.8.0");
        assert_eq!(
            m.source,
            "registry+https://github.com/rust-lang/crates.io-index"
        );
        assert_eq!(m.checksum, "abc123");
    }

    #[test]
    fn package_metadata_does_not_match_prefixed_name() {
        // "chematic-core" must not be mistaken for "chematic".
        let m = parse_package_metadata(FIXTURE_LOCK, "chematic-core").unwrap();
        assert_eq!(m.checksum, "def456");
    }

    #[test]
    fn package_metadata_missing_package_is_hard_error() {
        let err = parse_package_metadata(FIXTURE_LOCK, "does-not-exist").unwrap_err();
        assert!(err.to_string().contains("does-not-exist"));
    }

    #[test]
    fn package_metadata_missing_field_is_hard_error() {
        let lock = r#"
[[package]]
name = "chematic"
version = "0.8.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let err = parse_package_metadata(lock, "chematic").unwrap_err();
        assert!(err.to_string().contains("checksum"));
    }

    #[test]
    fn package_metadata_duplicate_block_is_hard_error() {
        let lock = format!(
            "{FIXTURE_LOCK}\n[[package]]\nname = \"chematic\"\nversion = \"0.8.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"other\"\n"
        );
        let err = parse_package_metadata(&lock, "chematic").unwrap_err();
        assert!(err.to_string().contains("more than one"));
    }
}
