#![forbid(unsafe_code)]

/// renkin-forward CLI
///
/// Usage:
///   renkin-forward predict --reactants "CC(=O)O" "CCO" [--templates file.smi] [--max-results 5] [--report]
///   renkin-forward validate --route-json '{"steps":[...]}' [--templates file.smi]
///
/// Output: JSON to stdout, nothing else. Template load summary, warnings,
/// and diagnostics go to stderr.
use anyhow::{Result, bail};
use renkin::chem_env::default_rules;
use renkin_forward::{
    ForwardPredictConfig, ForwardPrediction, load_templates_strict, predict_products,
    predict_products_detailed,
};

const TOP_LEVEL_HELP: &str = "renkin-forward — template-based forward reaction prediction\n\
\n\
Usage:\n  \
renkin-forward predict --reactants <SMILES>... [--templates <path>] [--max-results N] [--report]\n  \
renkin-forward validate --route-json <JSON> [--templates <path>] [--max-results N]\n  \
renkin-forward --help\n  \
renkin-forward --version\n\
\n\
Run `renkin-forward predict --help` or `renkin-forward validate --help` for subcommand options.";

const PREDICT_HELP: &str = "renkin-forward predict — forward-apply reversible SMIRKS templates to reactants\n\
\n\
Usage:\n  \
renkin-forward predict --reactants <SMILES>... [--templates <path>] [--max-results N] [--report]\n\
\n\
Options:\n  \
--reactants <SMILES>...   One or more reactant SMILES (required)\n  \
--templates <path>        Additional SMIRKS template file (hard error if missing/unreadable/empty)\n  \
--max-results N           Maximum candidates to return (default 5, must be > 0)\n  \
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

fn print_version() {
    println!(
        "renkin-forward {} (a sub-crate of the renkin workspace; this is NOT the renkin package version)",
        env!("CARGO_PKG_VERSION")
    );
}

/// Parses `--max-results N`, requiring a well-formed positive integer.
fn parse_max_results(raw: &str) -> Result<usize> {
    let n: usize = raw.parse().map_err(|_| {
        anyhow::anyhow!("invalid --max-results value {raw:?}: expected a positive integer")
    })?;
    if n == 0 {
        bail!("--max-results must be greater than 0, got 0");
    }
    Ok(n)
}

struct ParsedArgs {
    reactants: Vec<String>,
    route_json: Option<String>,
    templates_path: Option<String>,
    max_results: usize,
    report: bool,
}

/// Strict argument parser shared by `predict`/`validate`: unknown options,
/// missing option values, and invalid integers are all hard errors, never
/// silently ignored or defaulted.
fn parse_args(subcommand: &str, args: &[String]) -> Result<ParsedArgs> {
    let mut reactants: Vec<String> = Vec::new();
    let mut route_json: Option<String> = None;
    let mut templates_path: Option<String> = None;
    let mut max_results: usize = 5;
    let mut report = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--reactants" => {
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
            "--route-json" => {
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
            "--max-results" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--max-results requires a value"))?;
                max_results = parse_max_results(v)?;
            }
            "--report" => {
                report = true;
            }
            "--help" | "-h" => {
                println!(
                    "{}",
                    match subcommand {
                        "predict" => PREDICT_HELP,
                        "validate" => VALIDATE_HELP,
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
        route_json,
        templates_path,
        max_results,
        report,
    })
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
    if subcommand != "predict" && subcommand != "validate" {
        bail!(
            "unknown subcommand {subcommand:?}. Use 'predict', 'validate', '--help', or '--version'."
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
            if parsed.report {
                let config = ForwardPredictConfig {
                    max_results: parsed.max_results,
                    ..Default::default()
                };
                let report = predict_products_detailed(&refs, &rules, &config)?;
                for w in &report.warnings {
                    eprintln!("warning[{}]: {}", w.code, w.message);
                }
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let predictions: Vec<ForwardPrediction> =
                    predict_products(&refs, &rules, parsed.max_results)?;
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
                let target = step["target"].as_str().unwrap_or("");
                let prec_refs: Vec<&str> = step["precursors"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();

                let full_config = ForwardPredictConfig {
                    max_results: usize::MAX,
                    ..Default::default()
                };
                let full_report = predict_products_detailed(&prec_refs, &rules, &full_config)?;

                let target_canon = renkin::chem_env::mol_from_smiles(target)
                    .ok()
                    .map(|m| chematic::smiles::canonical_smiles(&m))
                    .unwrap_or_else(|| target.to_string());
                let verified = full_report
                    .candidates
                    .iter()
                    .any(|c| c.products.contains(&target_canon));

                let preds: Vec<ForwardPrediction> =
                    predict_products(&prec_refs, &rules, parsed.max_results)?;

                results.push(serde_json::json!({
                    "step_index": idx,
                    "target": target,
                    "verified": verified,
                    "top_predictions": preds,
                }));
            }
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        _ => unreachable!("validated above"),
    }
    Ok(())
}
