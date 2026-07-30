#![forbid(unsafe_code)]

/// renkin-forward CLI
///
/// Usage:
///   renkin-forward predict --reactants "CC(=O)O" "CCO" [--templates file.smi] [--max-results 5] [--report]
///   renkin-forward validate --route-json '{"steps":[...]}' [--templates file.smi]
///   renkin-forward enumerate --reactant "CC(=O)O" [--partners partners.smi] [--templates file.smi]
///
/// Output: JSON to stdout, nothing else. Template load summary, warnings,
/// and diagnostics go to stderr.
use anyhow::{Result, bail};
use renkin::chem_env::default_rules;
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
renkin-forward --help\n  \
renkin-forward --version\n\
\n\
Run `renkin-forward predict --help`, `renkin-forward validate --help`, or\n\
`renkin-forward enumerate --help` for subcommand options.";

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
    report: bool,
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
    let mut report = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--reactants" if subcommand == "predict" => {
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
            "--report" if subcommand == "predict" => {
                report = true;
            }
            "--help" | "-h" => {
                println!(
                    "{}",
                    match subcommand {
                        "predict" => PREDICT_HELP,
                        "validate" => VALIDATE_HELP,
                        "enumerate" => ENUMERATE_HELP,
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
        report,
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
    if subcommand != "predict" && subcommand != "validate" && subcommand != "enumerate" {
        bail!(
            "unknown subcommand {subcommand:?}. Use 'predict', 'validate', 'enumerate', '--help', or '--version'."
        );
    }

    let parsed = parse_args(subcommand, &args[2..])?;
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
        _ => unreachable!("validated above"),
    }
    Ok(())
}
