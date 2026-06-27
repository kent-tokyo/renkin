#![forbid(unsafe_code)]

use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::chem_env;
use renkin::display;
use renkin::search::{self, SearchConfig};

use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Serialize)]
struct Output {
    target: String,
    routes_found: usize,
    routes: Vec<search::Route>,
    /// P(at least one route succeeds) = 1 − Π(1 − route.success_probability).
    joint_success_probability: f64,
}

// ..Default::default() is needed when nn-scoring feature is enabled (adds nn_scorer field).
// When the feature is off, all fields are explicit, making the spread redundant — suppress lint.
#[allow(clippy::needless_update)]
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Subcommand dispatch: `renkin stock <cmd> ...`
    if args.get(1).map(|s| s.as_str()) == Some("stock") {
        return run_stock(&args[2..]);
    }

    let mut target: Option<String> = None;
    let mut max_depth: u32 = 5;
    let mut bb_path: Option<String> = None;
    let mut templates_path: Option<String> = None;
    let mut max_routes: usize = 5;
    let mut beam_width: usize = 0;
    let mut format: String = "json".to_string();
    let mut avoid_elements: String = String::new();
    let mut require_elements: String = String::new();
    let mut verbose = false;
    let mut bond_index = false;
    let mut bb_prices_path: Option<String> = None;
    let mut stock_path: Option<String> = None;
    let mut objectives_spec: String = "cost:min,success_probability:max,steps:min".to_string();
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    let mut scorer_path: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--target" | "-t" => {
                i += 1;
                if i < args.len() {
                    target = Some(args[i].clone());
                }
            }
            "--depth" | "-d" => {
                i += 1;
                if i < args.len() {
                    max_depth = args[i].parse().unwrap_or(5);
                }
            }
            "--building-blocks" | "-b" => {
                i += 1;
                if i < args.len() {
                    bb_path = Some(args[i].clone());
                }
            }
            "--templates" => {
                i += 1;
                if i < args.len() {
                    templates_path = Some(args[i].clone());
                }
            }
            "--max-routes" | "-n" => {
                i += 1;
                if i < args.len() {
                    max_routes = args[i].parse().unwrap_or(5);
                }
            }
            "--beam-width" | "-w" => {
                i += 1;
                if i < args.len() {
                    beam_width = args[i].parse().unwrap_or(0);
                }
            }
            "--format" | "-f" => {
                i += 1;
                if i < args.len() {
                    format = args[i].clone();
                }
            }
            "--avoid-elements" | "-e" => {
                i += 1;
                if i < args.len() {
                    avoid_elements = args[i].clone();
                }
            }
            "--require-elements" | "-r" => {
                i += 1;
                if i < args.len() {
                    require_elements = args[i].clone();
                }
            }
            "--verbose" | "-v" => {
                verbose = true;
            }
            "--bond-index" => {
                bond_index = true;
            }
            "--bb-prices" => {
                i += 1;
                if i < args.len() {
                    bb_prices_path = Some(args[i].clone());
                }
            }
            "--stock" => {
                i += 1;
                if i < args.len() {
                    stock_path = Some(args[i].clone());
                }
            }
            "--objectives" => {
                i += 1;
                if i < args.len() {
                    objectives_spec = args[i].clone();
                }
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            "--scorer" => {
                i += 1;
                if i < args.len() {
                    scorer_path = Some(args[i].clone());
                }
            }
            _ => {}
        }
        i += 1;
    }

    let Some(target_smiles) = target else {
        bail!(
            "Usage: renkin --target <SMILES> [--depth <N>] [--max-routes <N>] \
             [--beam-width <N>] [--building-blocks <path>] [--templates <path>] \
             [--format json|tree|mermaid]\n\
             \n\
             Options:\n  \
             --target / -t      Target molecule SMILES\n  \
             --depth  / -d      Max retrosynthesis depth (default: 5)\n  \
             --max-routes / -n  Max routes to return (default: 5)\n  \
             --beam-width / -w  Beam search width, 0 = unlimited A* (default: 0)\n  \
             --building-blocks  Path to .smi file of commercial starting materials\n  \
             --templates        Path to extracted SMIRKS templates file (tab-separated)\n  \
             --format / -f      Output format: json (default), tree, mermaid\n  \
             --avoid-elements / -e  Comma-separated elements to ban from BBs (e.g. \"Br,I\")\n  \
             --require-elements / -r  Comma-separated elements each route must supply (e.g. \"B\")\n  \
             --verbose / -v         Print search statistics to stderr\n  \
             --bond-index           Bond-center template index: ~24%% faster, no accuracy loss\n  \
             --bb-prices <path>     CSV (SMILES,price_per_gram) for route cost scoring"
        );
    };

    // --stock overrides --building-blocks and --bb-prices
    let (env, bb_price_map) = if let Some(ref path) = stock_path {
        let entries = load_stock_csv(path);
        let smiles_owned: Vec<String> = entries.iter().map(|e| e.smiles.clone()).collect();
        let smiles_refs: Vec<&str> = smiles_owned.iter().map(|s| s.as_str()).collect();
        let stock_env = chem_env::ChemEnv::in_memory(&smiles_refs);
        let prices: std::collections::HashMap<String, f64> = entries
            .into_iter()
            .filter_map(|e| e.price_jpy.map(|p| (e.smiles, p)))
            .collect();
        (stock_env, Some(prices))
    } else {
        let env = match bb_path {
            Some(ref path) => chem_env::ChemEnv::load(path)?,
            None => chem_env::ChemEnv::load("data/building_blocks.smi")
                .unwrap_or_else(|_| chem_env::ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS)),
        };
        let prices = bb_prices_path.as_deref().map(load_prices);
        (env, prices)
    };

    let mut rules = chem_env::default_rules();
    if let Some(ref path) = templates_path {
        let extra = chem_env::load_rules_from_file(path);
        eprintln!("Loaded {} templates from {path}", extra.len());
        rules.extend(extra);
    }
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    let nn_scorer: Option<std::sync::Arc<renkin::scorer::nn::TemplateScorer>> =
        scorer_path.as_deref().map(|p| {
            let top_k = rules.len();
            let rules_offset = renkin::chem_env::default_rules().len();
            renkin::scorer::nn::TemplateScorer::from_path(p, top_k, rules_offset)
                .map(std::sync::Arc::new)
                .unwrap_or_else(|e| {
                    eprintln!("scorer load error: {e}");
                    std::process::exit(1)
                })
        });

    let config = SearchConfig {
        max_depth,
        max_routes,
        beam_width,
        forbidden_elements: chem_env::elem_symbols_to_mask(&avoid_elements),
        required_element_present: chem_env::elem_symbols_to_mask(&require_elements),
        verbose,
        bond_index,
        bb_price_map,
        #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
        nn_scorer,
        ..Default::default()
    };
    let (routes, stats) = search::find_routes(&target_smiles, &env, &rules, &config)?;

    match format.as_str() {
        "tree" => {
            println!("Target: {target_smiles}");
            println!("Routes found: {}\n", routes.len());
            for (i, route) in routes.iter().enumerate() {
                print!(
                    "{}",
                    display::format_route_tree(route, &target_smiles, i + 1)
                );
                println!();
            }
        }
        "mermaid" => {
            for (i, route) in routes.iter().enumerate() {
                println!(
                    "{}",
                    display::format_route_mermaid(route, &target_smiles, i + 1)
                );
            }
        }
        "explain" => {
            for (i, route) in routes.iter().enumerate() {
                print!("{}", display::explain_route(route, &target_smiles, i + 1));
            }
        }
        "compare" | "table" => {
            println!("{}", display::format_route_table(&routes));
        }
        "compare-json" => {
            #[derive(serde::Serialize)]
            struct RouteCompare {
                route_num: usize,
                steps: usize,
                depth: u32,
                confidence: f64,
                success_probability: f64,
                route_cost: f64,
                convergency: f64,
                families: Vec<String>,
            }
            let rows: Vec<RouteCompare> = routes
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let mut families: Vec<String> = Vec::new();
                    for step in &r.steps {
                        if let Some(f) = step.reaction_family.as_deref()
                            && !families.iter().any(|x| x == f)
                        {
                            families.push(f.to_string());
                        }
                    }
                    RouteCompare {
                        route_num: i + 1,
                        steps: r.steps.len(),
                        depth: r.depth,
                        confidence: r.confidence,
                        success_probability: r.success_probability,
                        route_cost: r.route_cost,
                        convergency: r.convergency,
                        families,
                    }
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        "pareto" => {
            let objs = parse_objectives(&objectives_spec);
            let front = pareto_front_indices(&routes, &objs);
            let obj_labels: Vec<String> = objs
                .iter()
                .map(|(f, d)| format!("{}:{}", f.as_str(), d.as_str()))
                .collect();
            #[derive(serde::Serialize)]
            struct ParetoRoute {
                route_num: usize,
                route_cost: f64,
                success_probability: f64,
                steps: usize,
                depth: u32,
                confidence: f64,
                convergency: f64,
                #[serde(skip_serializing_if = "Option::is_none")]
                tradeoff: Option<String>,
            }
            let front_routes: Vec<ParetoRoute> = front
                .iter()
                .map(|&idx| ParetoRoute {
                    route_num: idx + 1,
                    route_cost: routes[idx].route_cost,
                    success_probability: routes[idx].success_probability,
                    steps: routes[idx].steps.len(),
                    depth: routes[idx].depth,
                    confidence: routes[idx].confidence,
                    convergency: routes[idx].convergency,
                    tradeoff: tradeoff_label(idx, &front, &routes, &objs),
                })
                .collect();
            let out = serde_json::json!({
                "target": target_smiles,
                "routes_searched": routes.len(),
                "objectives": obj_labels,
                "pareto_front_size": front.len(),
                "pareto_front": front_routes,
                "dominated_count": routes.len() - front.len(),
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        _ => {
            if routes.is_empty() {
                let (causes, suggestions) = diagnose(&stats, max_depth);
                let out = serde_json::json!({
                    "target": target_smiles,
                    "routes_found": 0,
                    "routes": [],
                    "diagnostics": {
                        "nodes_expanded":    stats.nodes_expanded,
                        "max_depth_reached": stats.max_depth_reached,
                        "beam_limit_hit":    stats.beam_limit_hit,
                        "matched_templates": stats.matched_templates,
                        "stock_hits":        stats.stock_hits,
                        "likely_causes":     causes,
                        "suggestions":       suggestions,
                    }
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                let joint_success_probability = 1.0
                    - routes
                        .iter()
                        .map(|r| 1.0 - r.success_probability)
                        .product::<f64>();
                let output = Output {
                    target: target_smiles,
                    routes_found: routes.len(),
                    joint_success_probability,
                    routes,
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
        }
    }
    Ok(())
}

fn diagnose(stats: &search::SearchStats, max_depth: u32) -> (Vec<&'static str>, Vec<String>) {
    let mut causes: Vec<&'static str> = Vec::new();
    let mut suggestions: Vec<String> = Vec::new();
    if stats.stock_hits == 0 {
        causes.push("no matching building block in stock");
        suggestions.push("add a custom stock file with --building-blocks".to_string());
    }
    if stats.max_depth_reached {
        causes.push("search depth exhausted");
        suggestions.push(format!("try --depth {}", max_depth + 2));
    }
    if stats.beam_limit_hit {
        causes.push("beam width too narrow — candidates were pruned");
        suggestions.push("try --beam-width 200".to_string());
    }
    if stats.matched_templates < 5 {
        causes.push("few or no templates matched the target");
        suggestions.push("try --templates data/templates_extracted_50000.smi".to_string());
    }
    (causes, suggestions)
}

// ── Pareto / multi-objective support ──────────────────────────────────────

#[derive(Clone, Copy)]
enum ObjField {
    Cost,
    SuccessProb,
    Steps,
    Depth,
    Confidence,
    Convergency,
    AtomEconomy,
}

#[derive(Clone, Copy)]
enum ObjDir {
    Min,
    Max,
}

impl ObjField {
    fn as_str(self) -> &'static str {
        match self {
            ObjField::Cost => "cost",
            ObjField::SuccessProb => "success_probability",
            ObjField::Steps => "steps",
            ObjField::Depth => "depth",
            ObjField::Confidence => "confidence",
            ObjField::Convergency => "convergency",
            ObjField::AtomEconomy => "atom_economy",
        }
    }
}

impl ObjDir {
    fn as_str(self) -> &'static str {
        match self {
            ObjDir::Min => "min",
            ObjDir::Max => "max",
        }
    }
}

fn parse_objectives(spec: &str) -> Vec<(ObjField, ObjDir)> {
    spec.split(',')
        .filter_map(|part| {
            let (field, dir) = part.trim().split_once(':')?;
            let f = match field.trim() {
                "cost" => ObjField::Cost,
                "success_probability" | "success" => ObjField::SuccessProb,
                "steps" => ObjField::Steps,
                "depth" => ObjField::Depth,
                "confidence" => ObjField::Confidence,
                "convergency" => ObjField::Convergency,
                "atom_economy" | "atom_economy_avg" => ObjField::AtomEconomy,
                _ => return None,
            };
            let d = match dir.trim() {
                "min" => ObjDir::Min,
                "max" => ObjDir::Max,
                _ => return None,
            };
            Some((f, d))
        })
        .collect()
}

fn obj_value(route: &search::Route, field: ObjField) -> f64 {
    match field {
        ObjField::Cost => route.route_cost,
        ObjField::SuccessProb => route.success_probability,
        ObjField::Steps => route.steps.len() as f64,
        ObjField::Depth => route.depth as f64,
        ObjField::Confidence => route.confidence,
        ObjField::Convergency => route.convergency,
        ObjField::AtomEconomy => {
            let vals: Vec<f64> = route.steps.iter().filter_map(|s| s.atom_economy).collect();
            if vals.is_empty() {
                0.0
            } else {
                vals.iter().sum::<f64>() / vals.len() as f64
            }
        }
    }
}

/// Returns true if route `b` dominates route `a`
/// (b is no worse on all objectives, strictly better on at least one).
fn dominates(a: &search::Route, b: &search::Route, objs: &[(ObjField, ObjDir)]) -> bool {
    let mut all_no_worse = true;
    let mut any_better = false;
    for &(field, dir) in objs {
        let va = obj_value(a, field);
        let vb = obj_value(b, field);
        let (b_better, b_worse) = match dir {
            ObjDir::Min => (vb < va, vb > va),
            ObjDir::Max => (vb > va, vb < va),
        };
        if b_worse {
            all_no_worse = false;
        }
        if b_better {
            any_better = true;
        }
    }
    all_no_worse && any_better
}

fn pareto_front_indices(routes: &[search::Route], objs: &[(ObjField, ObjDir)]) -> Vec<usize> {
    (0..routes.len())
        .filter(|&i| !(0..routes.len()).any(|j| j != i && dominates(&routes[i], &routes[j], objs)))
        .collect()
}

fn tradeoff_label(
    idx: usize,
    front: &[usize],
    routes: &[search::Route],
    objs: &[(ObjField, ObjDir)],
) -> Option<String> {
    let mut labels: Vec<&'static str> = Vec::new();
    for &(field, dir) in objs {
        let my_val = obj_value(&routes[idx], field);
        let is_unique_best = front.iter().filter(|&&j| j != idx).all(|&j| {
            let other = obj_value(&routes[j], field);
            match dir {
                ObjDir::Min => my_val < other,
                ObjDir::Max => my_val > other,
            }
        });
        if is_unique_best {
            labels.push(match (field, dir) {
                (ObjField::Cost, ObjDir::Min) => "cheapest",
                (ObjField::SuccessProb, ObjDir::Max) => "most_reliable",
                (ObjField::Steps, ObjDir::Min) | (ObjField::Depth, ObjDir::Min) => "shortest",
                (ObjField::Confidence, ObjDir::Max) => "highest_confidence",
                (ObjField::Convergency, ObjDir::Max) => "most_convergent",
                (ObjField::AtomEconomy, ObjDir::Max) => "best_atom_economy",
                _ => continue,
            });
        }
    }
    if labels.is_empty() {
        None
    } else {
        Some(labels.join("_and_"))
    }
}

// ── Stock CSV support ──────────────────────────────────────────────────────

struct StockEntry {
    smiles: String,
    name: Option<String>,
    vendor: Option<String>,
    price_jpy: Option<f64>,
    hazard: Option<String>,
    available: bool,
}

/// Parse a stock CSV file.
/// Header (first non-comment line) and comment lines starting with `#` are skipped.
/// Columns: smiles, name, vendor, price_jpy, amount, hazard, available
fn load_stock_csv(path: &str) -> Vec<StockEntry> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut first = true;
    content
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            // skip header row
            if first {
                let is_header = l.to_ascii_lowercase().starts_with("smiles");
                first = false; // won't trigger again; closure captures mut ref
                if is_header {
                    return None;
                }
            }
            let cols: Vec<&str> = l.splitn(8, ',').collect();
            let smiles = cols.first()?.trim().to_string();
            if smiles.is_empty() {
                return None;
            }
            Some(StockEntry {
                smiles,
                name: cols
                    .get(1)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                vendor: cols
                    .get(2)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                price_jpy: cols.get(3).and_then(|s| s.trim().parse::<f64>().ok()),
                hazard: cols
                    .get(5)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                available: cols
                    .get(6)
                    .map(|s| s.trim().eq_ignore_ascii_case("true"))
                    .unwrap_or(true),
            })
        })
        .collect()
}

fn run_stock(args: &[String]) -> Result<()> {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");
    match cmd {
        "stats" => {
            let path = args.get(1).map(|s| s.as_str()).unwrap_or("data/stock.csv");
            let entries = load_stock_csv(path);
            if entries.is_empty() {
                println!("No entries found in {path}");
                return Ok(());
            }
            let available = entries.iter().filter(|e| e.available).count();
            let priced: Vec<f64> = entries.iter().filter_map(|e| e.price_jpy).collect();
            let (pmin, pmax) = if priced.is_empty() {
                ("—".to_string(), "—".to_string())
            } else {
                let mn = priced.iter().cloned().fold(f64::INFINITY, f64::min);
                let mx = priced.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                (format!("{mn:.0}"), format!("{mx:.0}"))
            };
            let mut hazards: Vec<&str> =
                entries.iter().filter_map(|e| e.hazard.as_deref()).collect();
            hazards.sort_unstable();
            hazards.dedup();
            println!("Stock: {path}");
            println!("  Entries   : {}", entries.len());
            println!("  Available : {available}");
            println!("  Priced    : {} / {}", priced.len(), entries.len());
            println!("  Price JPY : {pmin} – {pmax}");
            println!(
                "  Hazards   : {}",
                if hazards.is_empty() {
                    "none".to_string()
                } else {
                    hazards.join(", ")
                }
            );
            let mut vendors: Vec<&str> =
                entries.iter().filter_map(|e| e.vendor.as_deref()).collect();
            vendors.sort_unstable();
            vendors.dedup();
            if !vendors.is_empty() {
                println!("  Vendors   : {}", vendors.join(", "));
            }
        }
        "validate" => {
            let path = args.get(1).map(|s| s.as_str()).unwrap_or("data/stock.csv");
            let entries = load_stock_csv(path);
            let mut valid = 0usize;
            let mut invalid: Vec<String> = Vec::new();
            for e in &entries {
                if chem_env::mol_from_smiles(&e.smiles).is_ok() {
                    valid += 1;
                } else {
                    let label = e.name.as_deref().unwrap_or("?");
                    invalid.push(format!("{} ({})", e.smiles, label));
                }
            }
            println!("Valid: {valid}  Invalid: {}", invalid.len());
            for s in &invalid {
                println!("  INVALID SMILES: {s}");
            }
        }
        "coverage" => {
            let targets_path = args.get(1).map(|s| s.as_str()).unwrap_or("targets.smi");
            let stock_path = args.get(2).map(|s| s.as_str()).unwrap_or("data/stock.csv");
            let entries = load_stock_csv(stock_path);
            let stock_set: std::collections::HashSet<&str> =
                entries.iter().map(|e| e.smiles.as_str()).collect();
            let targets: Vec<String> = std::fs::read_to_string(targets_path)
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.split_whitespace().next().unwrap_or(l).to_string())
                .collect();
            let in_stock: Vec<&str> = targets
                .iter()
                .filter(|t| stock_set.contains(t.as_str()))
                .map(|t| t.as_str())
                .collect();
            println!(
                "Targets: {}  In stock: {}  Not in stock: {}",
                targets.len(),
                in_stock.len(),
                targets.len() - in_stock.len()
            );
        }
        _ => {
            println!("Usage: renkin stock <stats|validate|coverage> [args...]");
            println!("  stats <file.csv>                  — summary statistics");
            println!("  validate <file.csv>               — check SMILES validity");
            println!("  coverage <targets.smi> <file.csv> — check which targets are in stock");
        }
    }
    Ok(())
}

fn load_prices(path: &str) -> std::collections::HashMap<String, f64> {
    std::fs::read_to_string(path)
        .ok()
        .map(|content| {
            content
                .lines()
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .filter_map(|l| {
                    let (smiles, price) = l.split_once(',')?;
                    let price: f64 = price.trim().parse().ok()?;
                    Some((smiles.trim().to_string(), price))
                })
                .collect()
        })
        .unwrap_or_default()
}
