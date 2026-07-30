#![forbid(unsafe_code)]

/// renkin-forward CLI
///
/// Usage:
///   renkin-forward predict --reactants "CC(=O)O" "CCO" [--templates file.smi] [--max-results 5] [--report]
///   renkin-forward validate --route-json '{"steps":[...]}' [--templates file.smi]
///   renkin-forward enumerate --reactant "CC(=O)O" [--partners partners.smi] [--templates file.smi]
///   renkin-forward hints --reactants "Brc1ccccc1" [--templates file.smi] [--max-hints 50]
///   renkin-forward benchmark --corpus corpus.jsonl --output-rows rows.jsonl [--output-report report.json]
///
/// Output: JSON to stdout, nothing else, EXCEPT `benchmark`, whose row-level
/// output always goes to the required `--output-rows` file (never stdout --
/// it can be large) and whose aggregate report goes to `--output-report` if
/// given, stdout otherwise. Template load summary, warnings, and
/// diagnostics go to stderr for every subcommand.
use anyhow::{Result, bail};
use renkin::chem_env::default_rules;
use renkin_forward::bench::{BenchOutcome, TemplateSource, run_benchmark};
use renkin_forward::hints::{HintGenerationConfig, generate_retrieval_hints};
use renkin_forward::{
    ForwardEnumerationConfig, ForwardPredictConfig, ForwardPrediction, enumerate_products_detailed,
    legacy_predictions_from_report, load_partners_strict, load_templates_strict,
    predict_products_detailed, sha256_hex_of_file,
};

const TOP_LEVEL_HELP: &str = "renkin-forward — template-based forward reaction prediction\n\
\n\
Usage:\n  \
renkin-forward predict --reactants <SMILES>... [--templates <path>] [--max-results N] [--report]\n  \
renkin-forward validate --route-json <JSON> [--templates <path>] [--max-results N]\n  \
renkin-forward enumerate --reactant <SMILES> [--partners <path>] [--templates <path>] [--max-results N]\n  \
renkin-forward hints --reactants <SMILES>... [--templates <path>] [--max-hints N]\n  \
renkin-forward benchmark --corpus <path> --output-rows <path> [--output-report <path>]\n                          \
[--template-source embedded|file|train-extracted] [--templates <path>]\n  \
renkin-forward --help\n  \
renkin-forward --version\n\
\n\
Run `renkin-forward predict --help`, `renkin-forward validate --help`,\n\
`renkin-forward enumerate --help`, `renkin-forward hints --help`, or\n\
`renkin-forward benchmark --help` for subcommand options.";

const PREDICT_HELP: &str = "renkin-forward predict — forward-apply reversible SMIRKS templates to reactants\n\
\n\
Usage:\n  \
renkin-forward predict --reactants <SMILES>... [--templates <path>] [--max-results N] [--report]\n\
\n\
Options:\n  \
--reactants <SMILES>...   One or more reactant SMILES (required)\n  \
--templates <path>        Additional SMIRKS template file (hard error if missing/unreadable/empty)\n  \
--max-results N           Without --report: maximum legacy prediction records returned, after\n                            \
per-source expansion. With --report: maximum merged candidates included in\n                            \
the report. Same flag, different meaning depending on --report. Must be > 0\n                            \
(default 5).\n  \
--report                  Emit a full ForwardPredictionReport instead of the legacy array\n\
\n\
Without --report, output is a JSON array of {template, products, weight} (products may repeat\n\
across entries when several templates converge on the same set, or one template matches more\n\
than once). `weight` is a ranking signal only, not a calibrated probability.\n\
With --report, output is a versioned ForwardPredictionReport with full candidate provenance,\n\
deterministic ranking, and structured stats/warnings.";

const VALIDATE_HELP: &str = "renkin-forward validate — check whether forward prediction reproduces a route's targets\n\
\n\
Usage:\n  \
renkin-forward validate --route-json <JSON> [--templates <path>] [--max-results N]\n\
\n\
--route-json accepts a bare route object ({\"steps\":[...]}) or a full find_routes\n\
output ({\"routes\":[{\"steps\":[...]}]}); omit --route-json to read JSON from stdin instead.\n\
\n\
Options:\n  \
--route-json <JSON>       Route JSON (or pipe it via stdin)\n  \
--templates <path>        Additional SMIRKS template file (hard error if missing/unreadable/empty)\n  \
--max-results N           Cap on displayed top_predictions per step (default 5, must be > 0);\n                            \
`verified` itself is always computed over the full, untruncated candidate set.";

const ENUMERATE_HELP: &str = "renkin-forward enumerate — discover forward products from one known reactant\n\
\n\
Usage:\n  \
renkin-forward enumerate --reactant <SMILES> [--partners <path>] [--templates <path>]\n                          \
[--max-results N] [--max-partners-per-template N] [--max-combinations N]\n\
\n\
Options:\n  \
--reactant <SMILES>              The one known reactant (required)\n  \
--partners <path>                Explicit partner SMILES library for binary-template slots.\n                                    \
Required to enumerate any binary (two-reactant) template; omit for\n                                    \
unary-template discovery only (hard error if the path is missing/\n                                    \
unreadable/empty/all-malformed)\n  \
--templates <path>                Additional SMIRKS template file (hard error if missing/unreadable/empty)\n  \
--max-results N                   Maximum merged candidates returned. Must be > 0 (default 5)\n  \
--max-partners-per-template N     Cap on partners tried per (template, slot). Must be > 0 (default 50)\n  \
--max-combinations N              Global cap on (template, slot, partner) combinations attempted\n                                    \
across the whole run. Must be > 0 (default 2000)\n\
\n\
Bounded, template-guided enumeration, not a generative predictor: unary templates apply directly;\n\
binary templates try the known reactant in each compatible slot and search --partners for the\n\
other. Templates needing 2+ missing partners are reported as unsupported, never silently skipped.\n\
No conditions, catalyst, yield, or reaction-success probability -- proposal_score is a ranking\n\
signal only. Output is a versioned ForwardEnumerationReport with full candidate provenance.";

const HINTS_HELP: &str = "renkin-forward hints — partner-free forward retrieval hints from known reactants\n\
\n\
Usage:\n  \
renkin-forward hints --reactants <SMILES>... [--templates <path>]\n                       \
[--max-hints N] [--max-matches-per-slot N] [--max-assignments-per-template N]\n\
\n\
Options:\n  \
--reactants <SMILES>...           One or more known reactants (required)\n  \
--templates <path>                Additional SMIRKS template file (hard error if missing/unreadable/empty)\n  \
--max-hints N                     Maximum merged hints returned. Must be > 0 (default 50)\n  \
--max-matches-per-slot N          Cap on reported match sites per (template, slot). Must be > 0 (default 20)\n  \
--max-assignments-per-template N  Cap on injective known-reactant/slot assignments enumerated\n                                    \
per template. Must be > 0 (default 200)\n\
\n\
Static, partner-free template analysis for search/retrieval, not a generative predictor:\n\
never calls run_reactants and never invents partner molecules. Reports, per compatible\n\
template, which slot(s) the known reactant(s) occupy, the exact SMARTS query for every\n\
still-missing partner slot, the bond-forming/breaking delta, and a query pattern (never a\n\
concrete SMILES) for the product. No conditions, catalyst, yield, or reaction-success\n\
probability, and no claim that a hint corresponds to a real, literature-verified reaction.\n\
Output is a versioned ForwardRetrievalHintReport.";

const BENCHMARK_HELP: &str = "renkin-forward benchmark — deterministic forward-prediction benchmark harness (issue #61 PR A)\n\
\n\
Usage:\n  \
renkin-forward benchmark --corpus <path> --output-rows <path> [--output-report <path>]\n                            \
[--template-source embedded|file|train-extracted] [--templates <path>]\n\
\n\
Options:\n  \
--corpus <path>            JSONL benchmark corpus, one reaction per line (required; see\n                               \
docs/guides/forward-benchmark.md for the schema). Never bundled by this repo --\n                               \
user-supplied, like the ORD evidence-import corpus.\n  \
--output-rows <path>       Where to write the row-level JSONL, one row per reaction\n                               \
(required; never printed to stdout -- can be large)\n  \
--output-report <path>     Where to write the aggregate-metrics JSON report. If omitted,\n                               \
the report is printed to stdout instead (this subcommand's only stdout output)\n  \
--template-source <mode>   'embedded' (default): embedded default rules only.\n                               \
'file': ONLY the rules in --templates (never merged with embedded defaults).\n                               \
'train-extracted': same mechanics as 'file'; labels the run as having used\n                               \
templates the caller extracted from the train split only -- this harness\n                               \
cannot verify that claim, see the guide.\n                               \
'scorer-conditioned' is named by the frozen protocol (Phase 0 mode 4) but is\n                               \
not implemented until a reranker exists (issue #61 Phase 3/4) -- rejected\n                               \
here, not silently downgraded to another mode.\n  \
--templates <path>         Required when --template-source is 'file' or 'train-extracted';\n                               \
hard error if given under 'embedded' (see docs)\n\
\n\
Deterministic and leakage-safe: every reaction is split train/val/test by a SHA-256 hash of\n\
its canonical reactant multiset (or an explicit corpus-supplied group_key), never by the\n\
accepted products -- see the guide for why. Conditional and end-to-end accuracy are always\n\
reported separately, never conflated. Repeated runs on the same corpus/rules produce\n\
byte-identical output except each row's elapsed_ms and the report's latency_ms percentiles.";

fn print_version() {
    println!(
        "renkin-forward {} (a sub-crate of the renkin workspace; this is NOT the renkin package version)",
        env!("CARGO_PKG_VERSION")
    );
}

/// Parses a `--flag N` style option, requiring a well-formed positive
/// integer. Shared by every numeric-limit flag (`--max-results`,
/// `--max-partners-per-template`, `--max-combinations`) so each gets the
/// same strict, named-in-the-error validation.
fn parse_positive_usize(flag: &str, raw: &str) -> Result<usize> {
    let n: usize = raw.parse().map_err(|_| {
        anyhow::anyhow!("invalid {flag} value {raw:?}: expected a positive integer")
    })?;
    if n == 0 {
        bail!("{flag} must be greater than 0, got 0");
    }
    Ok(n)
}

struct ParsedArgs {
    reactants: Vec<String>,
    reactant: Option<String>,
    route_json: Option<String>,
    templates_path: Option<String>,
    partners_path: Option<String>,
    max_results: usize,
    max_partners_per_template: usize,
    max_combinations: usize,
    max_hints: usize,
    max_matches_per_slot: usize,
    max_assignments_per_template: usize,
    report: bool,
    corpus_path: Option<String>,
    output_rows_path: Option<String>,
    output_report_path: Option<String>,
    template_source: String,
}

/// Strict argument parser shared by `predict`/`validate`: unknown options,
/// missing option values, and invalid integers are all hard errors, never
/// silently ignored or defaulted. Each subcommand also has its own option
/// allowlist -- `--reactants`/`--report` only make sense for `predict`,
/// `--route-json` only for `validate` -- so an option valid for the *other*
/// subcommand (e.g. `predict --route-json`, `validate --reactants`) is
/// itself an unknown-option hard error, not silently accepted or ignored.
fn parse_args(subcommand: &str, args: &[String]) -> Result<ParsedArgs> {
    let mut reactants: Vec<String> = Vec::new();
    let mut reactant: Option<String> = None;
    let mut route_json: Option<String> = None;
    let mut templates_path: Option<String> = None;
    let mut partners_path: Option<String> = None;
    let mut max_results: usize = 5;
    let mut max_partners_per_template: usize = 50;
    let mut max_combinations: usize = 2000;
    let hints_defaults = HintGenerationConfig::default();
    let mut max_hints: usize = hints_defaults.max_hints;
    let mut max_matches_per_slot: usize = hints_defaults.max_matches_per_slot;
    let mut max_assignments_per_template: usize = hints_defaults.max_assignments_per_template;
    let mut report = false;
    let mut corpus_path: Option<String> = None;
    let mut output_rows_path: Option<String> = None;
    let mut output_report_path: Option<String> = None;
    let mut template_source = "embedded".to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--reactants" if subcommand == "predict" || subcommand == "hints" => {
                i += 1;
                let start = i;
                while i < args.len() && !args[i].starts_with("--") {
                    reactants.push(args[i].clone());
                    i += 1;
                }
                if i == start {
                    bail!("--reactants requires at least one SMILES value");
                }
                continue;
            }
            "--reactant" if subcommand == "enumerate" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--reactant requires a value"))?;
                reactant = Some(v.clone());
            }
            "--route-json" if subcommand == "validate" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--route-json requires a value"))?;
                route_json = Some(v.clone());
            }
            "--templates" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--templates requires a value"))?;
                templates_path = Some(v.clone());
            }
            "--partners" if subcommand == "enumerate" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--partners requires a value"))?;
                partners_path = Some(v.clone());
            }
            "--max-results" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--max-results requires a value"))?;
                max_results = parse_positive_usize("--max-results", v)?;
            }
            "--max-partners-per-template" if subcommand == "enumerate" => {
                i += 1;
                let v = args.get(i).ok_or_else(|| {
                    anyhow::anyhow!("--max-partners-per-template requires a value")
                })?;
                max_partners_per_template = parse_positive_usize("--max-partners-per-template", v)?;
            }
            "--max-combinations" if subcommand == "enumerate" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--max-combinations requires a value"))?;
                max_combinations = parse_positive_usize("--max-combinations", v)?;
            }
            "--max-hints" if subcommand == "hints" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--max-hints requires a value"))?;
                max_hints = parse_positive_usize("--max-hints", v)?;
            }
            "--max-matches-per-slot" if subcommand == "hints" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--max-matches-per-slot requires a value"))?;
                max_matches_per_slot = parse_positive_usize("--max-matches-per-slot", v)?;
            }
            "--max-assignments-per-template" if subcommand == "hints" => {
                i += 1;
                let v = args.get(i).ok_or_else(|| {
                    anyhow::anyhow!("--max-assignments-per-template requires a value")
                })?;
                max_assignments_per_template =
                    parse_positive_usize("--max-assignments-per-template", v)?;
            }
            "--report" if subcommand == "predict" => {
                report = true;
            }
            "--corpus" if subcommand == "benchmark" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--corpus requires a value"))?;
                corpus_path = Some(v.clone());
            }
            "--output-rows" if subcommand == "benchmark" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--output-rows requires a value"))?;
                output_rows_path = Some(v.clone());
            }
            "--output-report" if subcommand == "benchmark" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--output-report requires a value"))?;
                output_report_path = Some(v.clone());
            }
            "--template-source" if subcommand == "benchmark" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--template-source requires a value"))?;
                template_source = v.clone();
            }
            "--help" | "-h" => {
                println!(
                    "{}",
                    match subcommand {
                        "predict" => PREDICT_HELP,
                        "validate" => VALIDATE_HELP,
                        "enumerate" => ENUMERATE_HELP,
                        "hints" => HINTS_HELP,
                        "benchmark" => BENCHMARK_HELP,
                        _ => TOP_LEVEL_HELP,
                    }
                );
                std::process::exit(0);
            }
            other => bail!("unknown option {other:?} for '{subcommand}'"),
        }
        i += 1;
    }

    Ok(ParsedArgs {
        reactants,
        reactant,
        route_json,
        templates_path,
        partners_path,
        max_results,
        max_partners_per_template,
        max_combinations,
        max_hints,
        max_matches_per_slot,
        max_assignments_per_template,
        report,
        corpus_path,
        output_rows_path,
        output_report_path,
        template_source,
    })
}

/// Strictly validates and extracts one route-JSON step's `target` and
/// `precursors`, with the step index and offending field name in every
/// error -- a step that isn't an object, or has a missing/wrong-type/empty
/// field, is a hard error, never silently coerced or dropped (a
/// `filter_map`/`as_str` pattern would drop a malformed precursor instead of
/// rejecting the whole step).
fn parse_step(idx: usize, step: &serde_json::Value) -> Result<(String, Vec<String>)> {
    if !step.is_object() {
        bail!("step {idx}: expected a JSON object, got {step}");
    }

    let target = step["target"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("step {idx}: missing or non-string 'target' field"))?;
    if target.is_empty() {
        bail!("step {idx}: 'target' must not be empty");
    }

    let precursors_json = step["precursors"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("step {idx}: missing or non-array 'precursors' field"))?;
    if precursors_json.is_empty() {
        bail!("step {idx}: 'precursors' must not be empty");
    }

    let mut precursors = Vec::with_capacity(precursors_json.len());
    for (p_idx, p) in precursors_json.iter().enumerate() {
        let s = p
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("step {idx}: precursors[{p_idx}] is not a string"))?;
        if s.is_empty() {
            bail!("step {idx}: precursors[{p_idx}] must not be empty");
        }
        precursors.push(s.to_string());
    }

    Ok((target.to_string(), precursors))
}

/// Loads the embedded default rules plus, if given, an explicit external
/// template file (hard error if that path is missing/unreadable/empty --
/// see [`load_templates_strict`]). Prints a stderr summary distinguishing
/// the two sources.
fn load_rules(templates_path: Option<&str>) -> Result<Vec<renkin::chem_env::RetroRule>> {
    let mut rules = default_rules();
    let embedded_count = rules.len();
    eprintln!("Loaded {embedded_count} embedded default template(s)");
    if let Some(path) = templates_path {
        let external = load_templates_strict(path)?;
        eprintln!("Loaded {} external template(s) from {path}", external.len());
        rules.extend(external);
    }
    Ok(rules)
}

/// Runs `benchmark`: loads the corpus and exactly one rule set (per
/// `--template-source`), scores every reaction, then writes the row-level
/// JSONL to `--output-rows` (always a file -- can be large) and the
/// aggregate report to `--output-report` if given, stdout otherwise.
fn run_benchmark_subcommand(parsed: &ParsedArgs) -> Result<()> {
    let corpus_path = parsed
        .corpus_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("benchmark requires --corpus <path>"))?;
    let output_rows_path = parsed
        .output_rows_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("benchmark requires --output-rows <path>"))?;
    let template_source = TemplateSource::parse(&parsed.template_source)?;

    let BenchOutcome { rows, report } = run_benchmark(
        corpus_path,
        template_source,
        parsed.templates_path.as_deref(),
    )?;

    let mut rows_jsonl = String::new();
    for row in &rows {
        rows_jsonl.push_str(&serde_json::to_string(row)?);
        rows_jsonl.push('\n');
    }
    std::fs::write(output_rows_path, rows_jsonl)
        .map_err(|e| anyhow::anyhow!("failed to write --output-rows {output_rows_path:?}: {e}"))?;
    eprintln!("Wrote {} row(s) to {output_rows_path}", rows.len());

    let report_json = serde_json::to_string_pretty(&report)?;
    match parsed.output_report_path.as_deref() {
        Some(path) => {
            std::fs::write(path, &report_json)
                .map_err(|e| anyhow::anyhow!("failed to write --output-report {path:?}: {e}"))?;
            eprintln!("Wrote aggregate report to {path}");
        }
        None => println!("{report_json}"),
    }

    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("{TOP_LEVEL_HELP}");
        std::process::exit(2);
    }

    match args[1].as_str() {
        "--help" | "-h" => {
            println!("{TOP_LEVEL_HELP}");
            return Ok(());
        }
        "--version" | "-V" => {
            print_version();
            return Ok(());
        }
        _ => {}
    }

    let subcommand = args[1].as_str();
    if !["predict", "validate", "enumerate", "hints", "benchmark"].contains(&subcommand) {
        bail!(
            "unknown subcommand {subcommand:?}. Use 'predict', 'validate', 'enumerate', 'hints', 'benchmark', '--help', or '--version'."
        );
    }

    let parsed = parse_args(subcommand, &args[2..])?;

    // `benchmark` loads its own rule set via `TemplateSource` (Phase 0: an
    // explicit template source must never be silently mixed with the
    // embedded defaults) -- it deliberately does NOT go through `load_rules`
    // below, which always extends the embedded set and is correct only for
    // predict/validate/enumerate.
    if subcommand == "benchmark" {
        return run_benchmark_subcommand(&parsed);
    }

    let rules = load_rules(parsed.templates_path.as_deref())?;

    match subcommand {
        "predict" => {
            if parsed.reactants.is_empty() {
                bail!("predict requires --reactants <SMILES>...");
            }
            let refs: Vec<&str> = parsed.reactants.iter().map(|s| s.as_str()).collect();
            // Exactly one prediction pass regardless of --report: with
            // --report, --max-results caps the merged candidates directly
            // (config.max_results below); without it, the full candidate
            // set is generated and --max-results instead caps the flat
            // legacy record list *after* per-source expansion (see
            // `legacy_predictions_from_report`) -- the same two numbers
            // would otherwise silently mean different things at the same
            // call site.
            let config = ForwardPredictConfig {
                max_results: if parsed.report {
                    parsed.max_results
                } else {
                    usize::MAX
                },
                ..Default::default()
            };
            let report = predict_products_detailed(&refs, &rules, &config)?;
            for w in &report.warnings {
                eprintln!("warning[{}]: {}", w.code, w.message);
            }
            if parsed.report {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let predictions: Vec<ForwardPrediction> =
                    legacy_predictions_from_report(&report, parsed.max_results);
                println!("{}", serde_json::to_string_pretty(&predictions)?);
            }
        }
        "validate" => {
            let json_str: String = match parsed.route_json {
                Some(s) => s,
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf.trim().to_string()
                }
            };
            if json_str.is_empty() {
                bail!("validate requires --route-json <JSON> or JSON piped via stdin");
            }
            let v: serde_json::Value = serde_json::from_str(&json_str)
                .map_err(|e| anyhow::anyhow!("invalid JSON: {e}"))?;

            let steps = if let Some(arr) = v["steps"].as_array() {
                arr
            } else if let Some(route) = v["routes"].as_array().and_then(|r| r.first()) {
                route["steps"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("first route has no 'steps' array"))?
            } else {
                bail!("JSON must contain a 'steps' array or a 'routes[0].steps' array");
            };

            let mut results: Vec<serde_json::Value> = Vec::new();
            for (idx, step) in steps.iter().enumerate() {
                let (target, precursors) = parse_step(idx, step)?;
                let prec_refs: Vec<&str> = precursors.iter().map(|s| s.as_str()).collect();

                // One prediction pass per step: `verified` and
                // `top_predictions` are both derived from this same
                // `full_report`, not from two separate template-application
                // passes over the (potentially large) rule set.
                let full_config = ForwardPredictConfig {
                    max_results: usize::MAX,
                    ..Default::default()
                };
                let full_report = predict_products_detailed(&prec_refs, &rules, &full_config)?;
                for w in &full_report.warnings {
                    eprintln!("warning[{}]: {}", w.code, w.message);
                }

                let target_canon = renkin::chem_env::mol_from_smiles(&target)
                    .ok()
                    .map(|m| chematic::smiles::canonical_smiles(&m))
                    .unwrap_or_else(|| target.clone());
                let verified = full_report
                    .candidates
                    .iter()
                    .any(|c| c.products.contains(&target_canon));

                let preds: Vec<ForwardPrediction> =
                    legacy_predictions_from_report(&full_report, parsed.max_results);

                results.push(serde_json::json!({
                    "step_index": idx,
                    "target": target,
                    "verified": verified,
                    "top_predictions": preds,
                }));
            }
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        "enumerate" => {
            let reactant = parsed
                .reactant
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("enumerate requires --reactant <SMILES>"))?;

            let (partner_outcome, partners_file_sha256) = match parsed.partners_path.as_deref() {
                Some(path) => {
                    let outcome = load_partners_strict(path)?;
                    if outcome.skipped_malformed > 0 {
                        eprintln!(
                            "warning[partner_lines_skipped_malformed]: {} line(s) in {path} \
                             could not be parsed as SMILES and were skipped",
                            outcome.skipped_malformed
                        );
                    }
                    eprintln!(
                        "Loaded {} partner record(s) from {path}",
                        outcome.records.len()
                    );
                    (Some(outcome), Some(sha256_hex_of_file(path)?))
                }
                None => (None, None),
            };
            let templates_file_sha256 = match parsed.templates_path.as_deref() {
                Some(path) => Some(sha256_hex_of_file(path)?),
                None => None,
            };

            let config = ForwardEnumerationConfig {
                max_results: parsed.max_results,
                max_partners_per_template: parsed.max_partners_per_template,
                max_combinations: parsed.max_combinations,
                ..Default::default()
            };
            let partner_slice = partner_outcome.as_ref().map(|o| o.records.as_slice());
            let mut report = enumerate_products_detailed(reactant, partner_slice, &rules, &config)?;
            report.stats.templates_file_sha256 = templates_file_sha256;
            report.stats.partners_file_sha256 = partners_file_sha256;
            report.stats.partner_records_skipped_malformed =
                partner_outcome.as_ref().map_or(0, |o| o.skipped_malformed);
            report.stats.partner_diagnostics_returned =
                partner_outcome.as_ref().map_or(0, |o| o.diagnostics.len());
            report.stats.partner_diagnostics_truncated = partner_outcome
                .as_ref()
                .is_some_and(|o| o.diagnostics_truncated);
            if let Some(outcome) = partner_outcome {
                report.partner_load_warnings = outcome.diagnostics;
            }

            for w in &report.warnings {
                eprintln!("warning[{}]: {}", w.code, w.message);
            }
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        "hints" => {
            if parsed.reactants.is_empty() {
                bail!("hints requires --reactants <SMILES>...");
            }
            let refs: Vec<&str> = parsed.reactants.iter().map(|s| s.as_str()).collect();
            let config = HintGenerationConfig {
                max_hints: parsed.max_hints,
                max_matches_per_slot: parsed.max_matches_per_slot,
                max_assignments_per_template: parsed.max_assignments_per_template,
            };
            let report = generate_retrieval_hints(&refs, &rules, &config)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        _ => unreachable!("validated above"),
    }
    Ok(())
}
