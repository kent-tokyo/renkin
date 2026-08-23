//! Tool-neutral, multi-route audit report: the shared pipeline behind both
//! `renkin audit-route` (`src/main.rs`, native/CLI) and the playground's
//! `audit_route` WASM export (`src/wasm.rs`, browser). Neither caller
//! duplicates format-detection/parsing/manifest logic -- both call
//! [`build_audit_route_report`] with whatever route-JSON text they already
//! have in hand (a file already read, or a pasted/uploaded string), and get
//! back the identical report shape either way. See `crate::bridge` module
//! docs for the wider parity contract this module participates in.
//!
//! Deliberately excludes anything caller-specific: no filesystem access, no
//! gzip decompression (native-only, via `flate2` -- a real AiZynthFinder
//! `.json.gz` batch export needs it, a browser paste/upload never does), no
//! human-readable text formatting. `src/main.rs::run_audit_route` and
//! `src/wasm.rs::audit_route` each own that on their own side.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::bridge::aizynthfinder::{AzfNode, normalize_aizynthfinder_route};
use crate::bridge::audit::{self, AuditPolicy, AuditReport, AuditStatus};
use crate::bridge::route_graph::normalize_renkin_route;
use crate::bridge::synplanner::{normalize_synplanner_route, parse_synplanner_routes};
use crate::bridge::syntheseus::{SyntheseusRouteV1, normalize_syntheseus_route};
use crate::chem_env::RetroRule;
use crate::search;

/// Minimal `#[derive(Deserialize)]` view of RENKIN's own `--format json`
/// route output (`main.rs`'s `Output` struct is `Serialize`-only, by
/// design -- see `crate::bridge` module docs for why round-tripping through
/// a purpose-built partial type is preferred over adding `Deserialize` to
/// the search-output types themselves). Declares only the fields
/// [`normalize_renkin_route`] actually reads; every other field in a real
/// RENKIN JSON file (`score`, `confidence`, `atom_economy`, ...) is
/// silently ignored by serde, not an error.
#[derive(Deserialize)]
struct AuditRouteInput {
    target: String,
    #[serde(default)]
    routes: Vec<AuditRouteEntry>,
}

#[derive(Deserialize)]
struct AuditRouteEntry {
    steps: Vec<AuditRouteStepInput>,
    #[serde(default)]
    building_blocks: Vec<String>,
}

#[derive(Deserialize)]
struct AuditRouteStepInput {
    target: String,
    precursors: Vec<String>,
    template_id: String,
}

/// Rebuilds a `search::Route` from the minimal parsed input -- every field
/// [`normalize_renkin_route`] doesn't read (`rule`, `depth`, `score`,
/// `confidence`, `atom_economy_status`, ...) is defaulted, mirroring the
/// same defaulting convention `bridge::route_graph`'s and `bridge::audit`'s
/// own test fixtures already use for hand-built routes.
fn route_from_audit_input(entry: AuditRouteEntry) -> search::Route {
    search::Route {
        steps: entry
            .steps
            .into_iter()
            .map(|s| search::ReactionStep {
                rule: String::new(),
                template_id: s.template_id,
                target: s.target,
                precursors: s.precursors,
                conditions: None,
                atom_economy: None,
                atom_economy_raw_percent: None,
                atom_economy_status: search::AtomEconomyStatus::NotEvaluable,
                step_confidence: 1.0,
                procedure_hint: None,
                reaction_family: None,
                metadata_source: None,
                metadata_scope: None,
                evidence: None,
            })
            .collect(),
        depth: 0,
        score: 0.0,
        building_blocks: entry.building_blocks,
        confidence: 0.0,
        convergency: 0.0,
        success_probability: 0.0,
        route_cost: 0.0,
    }
}

/// Parses a plain `.smi`-style stock listing (`SMILES<whitespace>name` per
/// line, `#`-comments and blank lines skipped -- the same convention as
/// `data/building_blocks.smi`, mirrored from `ChemEnv::load`'s own
/// line-parsing) into the canonical-SMILES set [`audit::audit`]'s
/// `configured_stock` expects. Uses plain `to_canonical`
/// (`chem_env::canonical_smiles`), NOT `ChemEnv`'s specialized
/// `canonical_stock_identity` -- `bridge::route_graph::canonicalize` (which
/// produces every `RouteNode::canonical_smiles` this gets compared against)
/// uses plain `to_canonical` too, and the two canonicalizations are a
/// documented non-invariant of each other, so this must match whichever one
/// `bridge` itself uses internally, not `ChemEnv`'s. Unparseable lines are
/// skipped, not a hard error -- an audit should still run against whatever
/// of the stock text *did* parse, rather than refusing to audit at all over
/// one bad line. Operates on an in-memory string (not a path) so both the
/// CLI's `--stock <PATH>` (after reading the file) and the browser's
/// pasted/uploaded stock text share this exact parsing, not two copies of
/// it.
pub fn parse_stock_text(content: &str) -> HashSet<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .filter_map(|smi| crate::chem_env::mol_from_smiles(smi).ok())
        .map(|m| crate::chem_env::to_canonical(&m))
        .collect()
}

/// v0.27.0 "Reproducible Route Audit": records what was audited and under
/// what conditions, so the same audit can be reproduced/verified later.
/// `report_schema_version`/`source_format` duplicate the pre-existing flat
/// [`AuditRouteReport`] fields of the same meaning below -- an explicit
/// design choice (both were named in the v0.27.0 spec), not an oversight;
/// the flat fields are kept for backward compatibility, not deprecated yet.
#[derive(Debug, Serialize)]
pub struct AuditManifest {
    renkin_version: &'static str,
    report_schema_version: u32,
    source_format: &'static str,
    /// Always `null` today: no adapter in this codebase captures a
    /// self-reported source-tool version from route input yet (`RouteSource`
    /// is a bare `Renkin`/`AiZynthFinder` enum with no version field) --
    /// genuinely unknown, not a placeholder for a future removal.
    source_version: Option<String>,
    input_sha256: String,
    /// `None` when no stock was given -- distinct from "unknown", stock
    /// validation genuinely did not run.
    stock_sha256: Option<String>,
    /// The actual [`AuditPolicy`] this report was derived under (v0.29.0
    /// Audit Policy Profiles). Policy only ever changes how `status` is
    /// derived -- every finding is reported in full regardless, matching
    /// what `audit-route`/the playground's Audit tab have always done.
    policy: &'static str,
}

/// Hashes the route-input text actually parsed and audited (already
/// decompressed/decoded by whichever caller owns that), not any incidental
/// on-disk encoding -- a gzip vs. plain copy of identical JSON content
/// hashes identically. Mirrors `ChemEnv::content_sha256`'s own "hash what
/// was actually used, not incidental encoding" reasoning.
fn input_content_sha256(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    format!("sha256:{}", crate::sha256_hex(digest))
}

/// Hashes the canonicalized stock set actually loaded and checked against
/// (sorted + length-prefixed, so it's order-independent and unambiguous) --
/// same recipe as `ChemEnv::content_sha256`.
fn stock_set_sha256(stock: &HashSet<String>) -> String {
    let mut sorted: Vec<&str> = stock.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-audit-manifest-stock-v1\0");
    hasher.update((sorted.len() as u64).to_be_bytes());
    for smi in sorted {
        hasher.update((smi.len() as u64).to_be_bytes());
        hasher.update(smi.as_bytes());
    }
    format!("sha256:{}", crate::sha256_hex(hasher.finalize()))
}

#[derive(Debug, Serialize)]
pub struct AuditRouteReport {
    /// Pre-existing field, kept for backward compatibility -- see
    /// [`AuditManifest`]'s doc comment for why this duplicates
    /// `audit_manifest.report_schema_version`.
    schema_version: u32,
    /// Pre-existing field, kept for backward compatibility -- see
    /// [`AuditManifest`]'s doc comment for why this duplicates
    /// `audit_manifest.source_format`.
    source_format: &'static str,
    pub audit_manifest: AuditManifest,
    pub summary: AuditRouteSummary,
    pub routes: Vec<AuditReport>,
}

#[derive(Debug, Serialize, Default)]
pub struct AuditRouteSummary {
    pub routes_total: usize,
    pub pass: usize,
    pub fail: usize,
    pub partial: usize,
}

impl AuditRouteSummary {
    fn record(&mut self, status: AuditStatus) {
        match status {
            AuditStatus::Pass => self.pass += 1,
            AuditStatus::Fail => self.fail += 1,
            AuditStatus::Partial => self.partial += 1,
        }
        self.routes_total += 1;
    }
}

/// One row of a real `aizynthcli` batch output file (`--output out.json.gz`
/// over a multi-target `--smiles targets.smi` run): Pandas
/// `to_json(orient="table")`, `{"schema": {...}, "data": [...]}`, one row
/// per target. `trees` is declared `"type": "string"` in the `schema`
/// block (a Pandas quirk for object-dtype columns) but is a real nested
/// JSON array in `data` itself, confirmed against a real capture -- see
/// `tests/fixtures/aizynthfinder/v4.4.1/PROVENANCE.md`. Every other schema
/// column (`search_time`, `is_solved`, `profiling`, ...) is ignored here,
/// same forward-compatible convention as [`AzfNode`].
#[derive(Deserialize)]
struct AzfBatchOutput {
    data: Vec<AzfBatchRow>,
}

#[derive(Deserialize)]
struct AzfBatchRow {
    #[serde(default)]
    trees: Vec<AzfNode>,
}

enum AuditRouteFormat {
    Renkin,
    AiZynthFinderSingle,
    AiZynthFinderBatch,
    Syntheseus,
    SynPlanner,
}

/// A real SynPlanner `write_routes_json` export: a top-level JSON object
/// whose keys all parse as non-negative integers (route IDs) and whose
/// values are themselves objects with `"type": "mol"` at their root --
/// confirmed against real SynPlanner 1.6.0 output (both hand-constructed
/// and real MCTS-searched, see `docs/design/synplanner-adapter-v1.md` §3.2
/// and §7 item 1). Only the internal `{route_id: RouteNode}` shape is
/// recognized here, not the separate `--export_routes` public-contract
/// wrapper (`{target_smiles: [RouteNode, ...]}`) -- see
/// `bridge::synplanner` module docs for why that's a deliberate, tracked
/// scope boundary rather than a silent gap.
fn looks_like_synplanner_export(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    !map.is_empty()
        && map.keys().all(|k| k.parse::<u64>().is_ok())
        && map
            .values()
            .all(|v| v.get("type").and_then(|t| t.as_str()) == Some("mol"))
}

/// `format: "auto"`'s sniff: RENKIN's own shape is a top-level object with
/// `target`+`routes`; a real AiZynthFinder single-target `aizynthcli
/// --output trees.json` is a top-level array of route dicts (each a `"type":
/// "mol"` root node); a real batch output is Pandas' `"schema"`+`"data"`
/// object; a `syntheseus-route-v1` document is a top-level object with
/// `"source_tool": "syntheseus"`; a real SynPlanner export is a top-level
/// object keyed by route-ID integers, each value a `"type": "mol"` root
/// node -- checked ahead of the RENKIN `target`+`routes` check and the
/// AiZynthFinder-batch check since all are top-level objects and this is a
/// more specific signal (per `docs/design/synplanner-adapter-v1.md` §3.2).
/// Anything else is an error, never a guess.
fn detect_audit_route_format(value: &serde_json::Value) -> anyhow::Result<AuditRouteFormat> {
    use anyhow::bail;
    match value {
        serde_json::Value::Array(items) => {
            if items.is_empty() || items[0].get("type").and_then(|t| t.as_str()) == Some("mol") {
                Ok(AuditRouteFormat::AiZynthFinderSingle)
            } else {
                bail!(
                    "renkin audit-route: --format auto could not identify this top-level JSON array (expected AiZynthFinder route dicts, each with \"type\": \"mol\")"
                )
            }
        }
        serde_json::Value::Object(map) if looks_like_synplanner_export(map) => {
            Ok(AuditRouteFormat::SynPlanner)
        }
        serde_json::Value::Object(map)
            if map.contains_key("schema") && map.contains_key("data") =>
        {
            Ok(AuditRouteFormat::AiZynthFinderBatch)
        }
        serde_json::Value::Object(map)
            if map.get("source_tool").and_then(|v| v.as_str()) == Some("syntheseus") =>
        {
            Ok(AuditRouteFormat::Syntheseus)
        }
        serde_json::Value::Object(map)
            if map.contains_key("target") && map.contains_key("routes") =>
        {
            Ok(AuditRouteFormat::Renkin)
        }
        _ => bail!(
            "renkin audit-route: --format auto could not identify this input -- recognized shapes are RENKIN (\"target\"+\"routes\" object), AiZynthFinder single-target (top-level array), AiZynthFinder batch (Pandas \"schema\"+\"data\" object), Syntheseus (\"source_tool\": \"syntheseus\" object), SynPlanner (top-level object keyed by route-ID integers). Pass --format explicitly if this is a supported shape auto-detection doesn't recognize."
        ),
    }
}

/// Audits every route found in `content` (already-decoded JSON text) and
/// returns the same report shape `renkin audit-route --output json` and the
/// playground's Audit tab both produce -- the single shared entry point
/// described in this module's own doc comment.
///
/// `format`: `"auto" | "renkin" | "aizynthfinder"`, same vocabulary as the
/// CLI's `--format` flag. `stock`: canonical SMILES of the stock actually
/// configured for this audit, or `None` for "no stock to check against"
/// (left `not_evaluable`, never force-passed -- see [`audit::audit`]'s own
/// doc comment). `rules`: RENKIN's own rule corpus, needed to resolve a
/// RENKIN-sourced step's `template_id` for forward validation.
///
/// Backward-compatible `AuditPolicy::Standard` wrapper around
/// [`build_audit_route_report_with_policy`] -- this function's signature
/// was already published (crates.io v0.28.0) by the time v0.29.0 Audit
/// Policy Profiles added the policy parameter, so it stays unchanged.
pub fn build_audit_route_report(
    content: &str,
    format: &str,
    stock: Option<&HashSet<String>>,
    rules: &[RetroRule],
) -> anyhow::Result<AuditRouteReport> {
    build_audit_route_report_with_policy(content, format, stock, rules, AuditPolicy::Standard)
}

/// Same as [`build_audit_route_report`], with an explicit [`AuditPolicy`]
/// controlling only how each route's `status` is derived -- every route's
/// `findings` stay the full, undiminished result regardless of policy; see
/// [`audit::audit_with_policy`]'s doc comment.
pub fn build_audit_route_report_with_policy(
    content: &str,
    format: &str,
    stock: Option<&HashSet<String>>,
    rules: &[RetroRule],
    policy: AuditPolicy,
) -> anyhow::Result<AuditRouteReport> {
    use anyhow::{Context, bail};

    if ![
        "auto",
        "renkin",
        "aizynthfinder",
        "syntheseus",
        "synplanner",
    ]
    .contains(&format)
    {
        bail!(
            "renkin audit-route: unsupported --format {format:?} (only auto|renkin|aizynthfinder|syntheseus|synplanner supported)"
        );
    }

    let value: serde_json::Value =
        serde_json::from_str(content).context("input: not valid JSON")?;

    let resolved_format = match format {
        "renkin" => AuditRouteFormat::Renkin,
        "aizynthfinder" => match &value {
            serde_json::Value::Array(_) => AuditRouteFormat::AiZynthFinderSingle,
            serde_json::Value::Object(map) if map.contains_key("data") => {
                AuditRouteFormat::AiZynthFinderBatch
            }
            _ => bail!(
                "renkin audit-route: --format aizynthfinder given but input isn't a recognized AiZynthFinder shape (top-level array, or Pandas \"schema\"+\"data\" object)"
            ),
        },
        "syntheseus" => AuditRouteFormat::Syntheseus,
        "synplanner" => AuditRouteFormat::SynPlanner,
        _ => detect_audit_route_format(&value)?,
    };

    let mut summary = AuditRouteSummary::default();
    let mut reports = Vec::new();
    let source_format = match resolved_format {
        AuditRouteFormat::Renkin => {
            let input: AuditRouteInput = serde_json::from_value(value)
                .context("input: not a recognized RENKIN route JSON")?;
            for entry in input.routes {
                let route = route_from_audit_input(entry);
                let outcome = normalize_renkin_route(&route, &input.target);
                let report = audit::audit_with_policy(&outcome, stock, Some(rules), policy);
                summary.record(report.status);
                reports.push(report);
            }
            "renkin"
        }
        AuditRouteFormat::AiZynthFinderSingle => {
            let routes: Vec<AzfNode> = serde_json::from_value(value)
                .context("input: not a recognized AiZynthFinder route JSON")?;
            for node in &routes {
                let outcome = normalize_aizynthfinder_route(node);
                let report = audit::audit_with_policy(&outcome, stock, Some(rules), policy);
                summary.record(report.status);
                reports.push(report);
            }
            "aizynthfinder"
        }
        AuditRouteFormat::AiZynthFinderBatch => {
            let batch: AzfBatchOutput = serde_json::from_value(value)
                .context("input: not a recognized AiZynthFinder batch output")?;
            for row in &batch.data {
                for node in &row.trees {
                    let outcome = normalize_aizynthfinder_route(node);
                    let report = audit::audit_with_policy(&outcome, stock, Some(rules), policy);
                    summary.record(report.status);
                    reports.push(report);
                }
            }
            "aizynthfinder"
        }
        AuditRouteFormat::Syntheseus => {
            let input: SyntheseusRouteV1 = serde_json::from_value(value)
                .context("input: not a recognized syntheseus-route-v1 JSON")?;
            let outcome = normalize_syntheseus_route(&input);
            let report = audit::audit_with_policy(&outcome, stock, Some(rules), policy);
            summary.record(report.status);
            reports.push(report);
            "syntheseus"
        }
        AuditRouteFormat::SynPlanner => {
            let routes = parse_synplanner_routes(value)
                .context("input: not a recognized SynPlanner write_routes_json export")?;
            for node in routes.values() {
                let outcome = normalize_synplanner_route(node);
                let report = audit::audit_with_policy(&outcome, stock, Some(rules), policy);
                summary.record(report.status);
                reports.push(report);
            }
            "synplanner"
        }
    };

    let manifest = AuditManifest {
        renkin_version: env!("CARGO_PKG_VERSION"),
        report_schema_version: 1,
        source_format,
        source_version: None,
        input_sha256: input_content_sha256(content),
        stock_sha256: stock.map(stock_set_sha256),
        policy: policy.as_str(),
    };

    Ok(AuditRouteReport {
        schema_version: 1,
        source_format,
        audit_manifest: manifest,
        summary,
        routes: reports,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RENKIN_FIXTURE: &str = r#"{
        "target": "CCOC(=O)c1ccccc1",
        "routes": [{
            "steps": [{
                "target": "CCOC(=O)c1ccccc1",
                "precursors": ["CCO", "O=C(O)c1ccccc1"],
                "template_id": "t1"
            }],
            "building_blocks": ["CCO", "O=C(O)c1ccccc1"]
        }]
    }"#;

    #[test]
    fn renkin_fixture_audits_as_partial_without_stock() {
        let rules: Vec<RetroRule> = Vec::new();
        let report =
            build_audit_route_report(RENKIN_FIXTURE, "auto", None, &rules).expect("audits");
        assert_eq!(report.summary.routes_total, 1);
        assert_eq!(report.summary.partial, 1);
        assert_eq!(report.audit_manifest.source_format, "renkin");
        assert!(report.audit_manifest.stock_sha256.is_none());
    }

    #[test]
    fn unsupported_format_is_rejected() {
        let rules: Vec<RetroRule> = Vec::new();
        let err = build_audit_route_report(RENKIN_FIXTURE, "bogus", None, &rules).unwrap_err();
        assert!(err.to_string().contains("unsupported --format"));
    }

    #[test]
    fn ambiguous_input_is_rejected_not_guessed() {
        let rules: Vec<RetroRule> = Vec::new();
        let err = build_audit_route_report("{}", "auto", None, &rules).unwrap_err();
        assert!(err.to_string().contains("could not identify"));
    }

    #[test]
    fn parse_stock_text_skips_comments_and_blanks() {
        let stock = parse_stock_text("# comment\nCCO ethanol\n\nO=C(O)c1ccccc1 benzoic\n");
        assert_eq!(stock.len(), 2);
    }

    fn load_synplanner_fixture(name: &str) -> String {
        let path = format!(
            "{}/tests/fixtures/synplanner/v1.6.0/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn synplanner_real_fixture_auto_detects_and_audits() {
        let content = load_synplanner_fixture("real_planning_route_2step.json");
        let rules: Vec<RetroRule> = Vec::new();
        let report = build_audit_route_report(&content, "auto", None, &rules).expect("audits");
        assert_eq!(report.audit_manifest.source_format, "synplanner");
        assert_eq!(report.summary.routes_total, 1);
    }

    #[test]
    fn synplanner_explicit_format_matches_auto_detected_result() {
        let content = load_synplanner_fixture("route_3_full_fields.json");
        let rules: Vec<RetroRule> = Vec::new();
        let auto = build_audit_route_report(&content, "auto", None, &rules).expect("auto audits");
        let explicit = build_audit_route_report(&content, "synplanner", None, &rules)
            .expect("explicit format audits");
        assert_eq!(auto.audit_manifest.source_format, "synplanner");
        assert_eq!(
            auto.routes[0].normalized_route_sha256,
            explicit.routes[0].normalized_route_sha256
        );
    }

    #[test]
    fn synplanner_detection_does_not_collide_with_renkin_shape() {
        // RENKIN's own {"target": ..., "routes": [...]} object must never be
        // misdetected as a SynPlanner {route_id: RouteNode} export, and vice
        // versa -- looks_like_synplanner_export requires every key to parse
        // as an integer, which "target"/"routes" never do.
        let rules: Vec<RetroRule> = Vec::new();
        let report =
            build_audit_route_report(RENKIN_FIXTURE, "auto", None, &rules).expect("audits");
        assert_eq!(report.audit_manifest.source_format, "renkin");
    }

    #[test]
    fn synplanner_detection_requires_every_key_to_parse_as_an_integer() {
        let mixed_keys = r#"{"1": {"type": "mol", "smiles": "CCO", "in_stock": true}, "route_a": {"type": "mol", "smiles": "CCO", "in_stock": true}}"#;
        let value: serde_json::Value = serde_json::from_str(mixed_keys).unwrap();
        let map = value.as_object().unwrap();
        assert!(!looks_like_synplanner_export(map));
    }
}
