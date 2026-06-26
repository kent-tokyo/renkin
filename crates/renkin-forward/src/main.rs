#![forbid(unsafe_code)]

/// renkin-forward CLI
///
/// Usage:
///   renkin-forward predict --reactants "CC(=O)O" "CCO" [--templates file.smi] [--max-results 5]
///   renkin-forward validate --route-json '{"steps":[...]}' [--templates file.smi]
///
/// Output: JSON to stdout.
use anyhow::{Result, bail};
use renkin::chem_env::{default_rules, load_rules_from_file};
use renkin_forward::{ForwardPrediction, predict_products};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        bail!(
            "Usage:\n  \
             renkin-forward predict --reactants <SMILES>... [--templates <path>] [--max-results N]\n  \
             renkin-forward validate --route-json <JSON>    [--templates <path>] [--max-results N]"
        );
    }

    let subcommand = args[1].as_str();
    let mut reactants: Vec<String> = Vec::new();
    let mut route_json: Option<String> = None;
    let mut templates_path: Option<String> = None;
    let mut max_results: usize = 5;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--reactants" => {
                i += 1;
                while i < args.len() && !args[i].starts_with("--") {
                    reactants.push(args[i].clone());
                    i += 1;
                }
                continue;
            }
            "--route-json" => {
                i += 1;
                if i < args.len() {
                    route_json = Some(args[i].clone());
                }
            }
            "--templates" => {
                i += 1;
                if i < args.len() {
                    templates_path = Some(args[i].clone());
                }
            }
            "--max-results" => {
                i += 1;
                if i < args.len() {
                    max_results = args[i].parse().unwrap_or(5);
                }
            }
            _ => {}
        }
        i += 1;
    }

    let mut rules = default_rules();
    if let Some(ref path) = templates_path {
        rules.extend(load_rules_from_file(path));
        eprintln!(
            "Loaded {} templates from {path}",
            rules.len() - default_rules().len()
        );
    }

    match subcommand {
        "predict" => {
            if reactants.is_empty() {
                bail!("predict requires --reactants <SMILES>...");
            }
            let refs: Vec<&str> = reactants.iter().map(|s| s.as_str()).collect();
            let predictions: Vec<ForwardPrediction> = predict_products(&refs, &rules, max_results)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            println!("{}", serde_json::to_string_pretty(&predictions)?);
        }
        "validate" => {
            let json_str: String = match route_json {
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
            // Parse route JSON as a Value to extract steps.
            // Accepts both a route object {"steps":[...]} and the full find_routes output
            // {"routes":[{"steps":[...]}]} so that piping renkin output works directly.
            let v: serde_json::Value =
                serde_json::from_str(&json_str).map_err(|e| anyhow::anyhow!("invalid JSON: {e}"))?;

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

                let preds: Vec<ForwardPrediction> =
                    predict_products(&prec_refs, &rules, max_results)
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

                let target_canon = renkin::chem_env::mol_from_smiles(target)
                    .ok()
                    .map(|m| chematic::smiles::canonical_smiles(&m))
                    .unwrap_or_else(|| target.to_string());
                let verified = preds.iter().any(|p| p.products.contains(&target_canon));

                results.push(serde_json::json!({
                    "step_index": idx,
                    "target": target,
                    "verified": verified,
                    "top_predictions": preds,
                }));
            }
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        _ => {
            bail!("unknown subcommand '{subcommand}'. Use 'predict' or 'validate'.");
        }
    }
    Ok(())
}
