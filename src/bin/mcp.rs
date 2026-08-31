//! renkin-mcp — MCP server for retrosynthesis via the Model Context Protocol.
//!
//! Transport: JSON-RPC 2.0 over stdio (one JSON object per line).
//! Register in Claude Desktop's `claude_desktop_config.json`:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "renkin": { "command": "/path/to/renkin-mcp" }
//!   }
//! }
//! ```
#![forbid(unsafe_code)]

use std::io::{self, BufRead, Write};

use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::chem_env::{self, elem_symbols_to_mask};
use renkin::display::{explain_route, format_route_tree};
use renkin::search::{self, Route, SearchConfig};
use renkin::validation::step_balanced;
use serde_json::{Value, json};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = msg["id"].clone();
        let method = msg["method"].as_str().unwrap_or("");

        // Notifications have no id and require no response.
        if method.starts_with("notifications/") {
            continue;
        }

        let result = match method {
            "initialize" => handle_initialize(),
            "tools/list" => handle_tools_list(),
            "tools/call" => handle_tools_call(&msg),
            _ => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": "Method not found"}
                });
                let _ = writeln!(out, "{resp}");
                let _ = out.flush();
                continue;
            }
        };

        let resp = json!({"jsonrpc": "2.0", "id": id, "result": result});
        let _ = writeln!(out, "{resp}");
        let _ = out.flush();
    }
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "renkin", "version": VERSION}
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "find_routes",
                "description": "Find retrosynthetic routes for a target molecule back to commercially available building blocks. Uses deterministic A* / AND-OR tree search with SMIRKS templates; supports standard search and opt-in progressive coverage search.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "smiles": {"type": "string", "description": "Target molecule SMILES"},
                        "depth": {"type": "integer", "description": "Max retrosynthesis depth (default: 5)"},
                        "max_routes": {"type": "integer", "description": "Routes to return (default: 5)"},
                        "avoid_elements": {"type": "string", "description": "Comma-separated elements to exclude from BBs (e.g. \"Br,I\")"},
                        "require_elements": {"type": "string", "description": "Elements that must appear in ≥1 building block (e.g. \"B\")"},
                        "search_mode": {"type": "string", "enum": ["standard", "coverage"], "description": "Search mode (default: standard). Coverage runs Stage 2 only when Stage 1 finds no route."},
                        "coverage_templates": {"type": "string", "description": "Stage-2 template file; required when search_mode is coverage. Invalid or empty files fail loudly."},
                        "coverage_timeout_secs": {"type": "integer", "minimum": 1, "description": "Optional cooperative Stage-2 timeout in seconds."}
                    },
                    "required": ["smiles"]
                }
            },
            {
                "name": "validate_route",
                "description": "Find the best retrosynthetic route for a target molecule and validate it: check atom balance of each step (target_MW ≤ Σ precursor_MW) and report confidence/probability scores.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "smiles": {"type": "string", "description": "Target molecule SMILES"},
                        "depth": {"type": "integer", "description": "Max search depth (default: 5)"}
                    },
                    "required": ["smiles"]
                }
            },
            {
                "name": "explain_route",
                "description": "Find retrosynthetic routes for a target and return a human-readable explanation of the top route(s): strengths, weaknesses, and per-step details derived from confidence, success_probability, atom_economy, and reaction_family.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "smiles": {"type": "string", "description": "Target molecule SMILES"},
                        "depth": {"type": "integer", "description": "Max search depth (default: 5)"},
                        "max_routes": {"type": "integer", "description": "Routes to explain (default: 1)"}
                    },
                    "required": ["smiles"]
                }
            },
            {
                "name": "find_pareto_routes",
                "description": "Find retrosynthetic routes for a target and return the Pareto-optimal subset across multiple objectives (route_cost, success_probability, steps, etc.). Each Pareto route is non-dominated — no other route is better on all objectives simultaneously.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "smiles": {"type": "string", "description": "Target molecule SMILES"},
                        "depth": {"type": "integer", "description": "Max search depth (default: 5)"},
                        "max_routes": {"type": "integer", "description": "Routes to search before computing Pareto front (default: 10)"},
                        "objectives": {"type": "string", "description": "Comma-separated objectives, e.g. \"cost:min,success_probability:max,steps:min\" (default)"}
                    },
                    "required": ["smiles"]
                }
            },
            {
                "name": "plan_with_constraints",
                "description": "Find retrosynthetic routes applying explicit constraints: avoid/require elements and building blocks, max steps/cost, confidence thresholds, required/avoided/preferred reaction families. Designed for LLM-driven synthesis planning (Project Ariadne style).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "smiles": {"type": "string", "description": "Target molecule SMILES"},
                        "depth": {"type": "integer", "description": "Max search depth (default: 5)"},
                        "max_routes": {"type": "integer", "description": "Max routes to return (default: 5)"},
                        "avoid_elements": {"type": "string", "description": "Comma-separated elements to ban from BBs (e.g. \"Br,I\")"},
                        "require_elements": {"type": "string", "description": "Elements that must appear in ≥1 BB (e.g. \"B\")"},
                        "avoid_building_blocks": {"type": "string", "description": "Comma-separated canonical building-block SMILES to exclude from route leaves"},
                        "require_building_blocks": {"type": "string", "description": "Comma-separated canonical building-block SMILES; each route must contain at least one"},
                        "max_steps": {"type": "integer", "description": "Maximum number of synthesis steps per route"},
                        "max_route_cost": {"type": "number", "description": "Maximum computed route cost (inclusive)"},
                        "min_confidence": {"type": "number", "description": "Minimum template confidence [0,1]"},
                        "min_success_probability": {"type": "number", "description": "Minimum route success probability [0,1]"},
                        "require_reaction_families": {"type": "string", "description": "Comma-separated reaction families; at least one must occur in each returned route"},
                        "avoid_reaction_families": {"type": "string", "description": "Comma-separated reaction families; routes containing any are excluded"},
                        "prefer_reaction_families": {"type": "string", "description": "Comma-separated reaction families to rank first (e.g. \"amide_coupling,suzuki_retro\")"}
                    },
                    "required": ["smiles"]
                }
            },
            {
                "name": "estimate_diversity",
                "description": "Find multiple retrosynthetic routes for a target molecule and report the route diversity score (1 - avg pairwise Jaccard similarity of building-block sets). Higher = more diverse options available.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "smiles": {"type": "string", "description": "Target molecule SMILES"},
                        "max_routes": {"type": "integer", "description": "Number of routes to compare (default: 5)"},
                        "depth": {"type": "integer", "description": "Max search depth (default: 5)"}
                    },
                    "required": ["smiles"]
                }
            },
            {
                "name": "diagnose_failure",
                "description": "Diagnose why no retrosynthetic route was found for a target molecule. Runs the search and analyses SearchStats to identify likely causes (depth exhausted, no matching templates, beam too narrow, no building block matches) and returns actionable suggestions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "smiles": {"type": "string", "description": "Target molecule SMILES"},
                        "depth": {"type": "integer", "description": "Max search depth (default: 5)"}
                    },
                    "required": ["smiles"]
                }
            }
        ]
    })
}

fn load_env_and_rules() -> (chem_env::ChemEnv, Vec<chem_env::RetroRule>) {
    let env = chem_env::ChemEnv::load("data/building_blocks.smi")
        .unwrap_or_else(|_| chem_env::ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS));
    let mut rules = chem_env::default_rules();
    // Load whichever template file is available (prefer larger set)
    for path in &[
        "data/templates_extracted_50000.smi",
        "data/templates_extracted_5000.smi",
    ] {
        if std::path::Path::new(path).is_file() {
            rules.extend(chem_env::load_rules_from_file(path));
            break;
        }
    }
    (env, rules)
}

fn handle_tools_call(msg: &Value) -> Value {
    let tool_name = msg["params"]["name"].as_str().unwrap_or("find_routes");
    let args = &msg["params"]["arguments"];
    let smiles = match args["smiles"].as_str() {
        Some(s) => s,
        None => return tool_error("missing required argument: smiles"),
    };
    match tool_name {
        "validate_route" => handle_validate_route(smiles, args),
        "estimate_diversity" => handle_estimate_diversity(smiles, args),
        "explain_route" => handle_explain_route(smiles, args),
        "find_pareto_routes" => handle_find_pareto_routes(smiles, args),
        "plan_with_constraints" => handle_plan_with_constraints(smiles, args),
        "diagnose_failure" => handle_diagnose_failure(smiles, args),
        _ => handle_find_routes(smiles, args),
    }
}

fn handle_find_routes(smiles: &str, args: &Value) -> Value {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let max_routes = args["max_routes"].as_u64().unwrap_or(5) as usize;
    let avoid = args["avoid_elements"].as_str().unwrap_or("");
    let require = args["require_elements"].as_str().unwrap_or("");
    let search_mode = args["search_mode"].as_str().unwrap_or("standard");
    if search_mode != "standard" && search_mode != "coverage" {
        return tool_error("invalid search_mode (expected standard or coverage)");
    }
    let coverage_path = args["coverage_templates"].as_str();
    let coverage_timeout = match args["coverage_timeout_secs"].as_u64() {
        Some(0) => return tool_error("coverage_timeout_secs must be a positive integer"),
        Some(seconds) => Some(std::time::Duration::from_secs(seconds)),
        None => None,
    };
    if search_mode == "standard" && (coverage_path.is_some() || coverage_timeout.is_some()) {
        return tool_error(
            "coverage_templates and coverage_timeout_secs require search_mode=coverage",
        );
    }
    let coverage_path = match (search_mode, coverage_path) {
        ("coverage", Some(path)) => path,
        ("coverage", None) => {
            return tool_error("search_mode=coverage requires coverage_templates");
        }
        _ => "",
    };

    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        forbidden_elements: elem_symbols_to_mask(avoid),
        required_element_present: elem_symbols_to_mask(require),
        ..Default::default()
    };

    let (routes, stats, coverage_summary) = if search_mode == "coverage" {
        let coverage_rules = match renkin::coverage_mode::load_coverage_rules(coverage_path) {
            Ok(rules) => rules,
            Err(e) => return tool_error(&format!("coverage template error: {e}")),
        };
        let result = match renkin::coverage_mode::run_coverage_mode(
            smiles,
            &env,
            &rules,
            &config,
            &coverage_rules,
            coverage_timeout,
        ) {
            Ok(result) => result,
            Err(e) => return tool_error(&format!("coverage search error: {e}")),
        };
        let summary = format!(
            "Search mode: coverage\nSelected stage: {:?}\nStage 2 invoked: {}\nStage 1 timeout: {}\nStage 2 timeout: {}\nStage 1 elapsed: {:.1} ms\nStage 2 elapsed: {}\nTotal elapsed: {:.1} ms\n\n",
            result.selected_stage,
            result.stage2_invoked,
            result.stage1_timeout,
            result.stage2_timeout,
            result.stage1_elapsed_ms,
            result
                .stage2_elapsed_ms
                .map(|elapsed| format!("{elapsed:.1} ms"))
                .unwrap_or_else(|| "not_run".to_string()),
            result.total_elapsed_ms,
        );
        (result.routes, result.stats, Some(summary))
    } else {
        let result = match search::find_routes(smiles, &env, &rules, &config) {
            Ok(r) => r,
            Err(e) => return tool_error(&format!("search error: {e}")),
        };
        (result.0, result.1, None)
    };

    let mut text = coverage_summary.unwrap_or_default();
    text.push_str(&format!(
        "Target: {smiles}\nRoutes found: {}\n\n",
        routes.len()
    ));
    if routes.is_empty() {
        text.push_str(&format!(
            "No routes found (nodes expanded: {}). Try increasing depth, or remove element constraints if set.",
            stats.nodes_expanded
        ));
    } else {
        for (i, route) in routes.iter().enumerate() {
            text.push_str(&format_route_tree(route, smiles, i + 1));
            text.push_str(&format!(
                "  Building blocks: {}\n\n",
                route.building_blocks.join(", ")
            ));
        }
    }
    json!({"content": [{"type": "text", "text": text}]})
}

fn handle_explain_route(smiles: &str, args: &Value) -> Value {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let max_routes = args["max_routes"].as_u64().unwrap_or(1) as usize;
    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        ..Default::default()
    };
    let (routes, _) = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("search error: {e}")),
    };
    if routes.is_empty() {
        return json!({"content": [{"type": "text", "text":
            format!("No routes found for {smiles}.")}]});
    }
    let text: String = routes
        .iter()
        .enumerate()
        .map(|(i, r)| explain_route(r, smiles, i + 1))
        .collect();
    json!({"content": [{"type": "text", "text": text}]})
}

fn handle_plan_with_constraints(smiles: &str, args: &Value) -> Value {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let max_routes = args["max_routes"].as_u64().unwrap_or(5) as usize;
    let avoid = args["avoid_elements"].as_str().unwrap_or("");
    let require = args["require_elements"].as_str().unwrap_or("");
    let avoid_bbs: Option<Vec<String>> = args["avoid_building_blocks"]
        .as_str()
        .map(|s| s.split(',').map(|bb| bb.trim().to_string()).collect());
    let require_bbs: Option<Vec<String>> = args["require_building_blocks"]
        .as_str()
        .map(|s| s.split(',').map(|bb| bb.trim().to_string()).collect());
    let max_steps = args["max_steps"].as_u64().map(|n| n as usize);
    let max_route_cost = args["max_route_cost"].as_f64();
    let min_confidence = args["min_confidence"].as_f64();
    let min_success_prob = args["min_success_probability"].as_f64();
    if let Err(message) = renkin::constraints::validate_route_thresholds(
        max_route_cost,
        min_confidence,
        min_success_prob,
    ) {
        return tool_error(&message);
    }
    let require_fams: Option<Vec<String>> = args["require_reaction_families"]
        .as_str()
        .map(|s| s.split(',').map(|f| f.trim().to_string()).collect());
    let avoid_fams: Option<Vec<String>> = args["avoid_reaction_families"]
        .as_str()
        .map(|s| s.split(',').map(|f| f.trim().to_string()).collect());
    let prefer_fams: Option<Vec<String>> = args["prefer_reaction_families"]
        .as_str()
        .map(|s| s.split(',').map(|f| f.trim().to_string()).collect());

    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        forbidden_elements: elem_symbols_to_mask(avoid),
        required_element_present: elem_symbols_to_mask(require),
        ..Default::default()
    };
    let (mut routes, _) = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("search error: {e}")),
    };

    // Apply post-filters
    if let Some(n) = max_steps {
        routes.retain(|r| r.steps.len() <= n);
    }
    if let Some(ref blocked) = avoid_bbs {
        routes.retain(|r| {
            !r.building_blocks
                .iter()
                .any(|bb| blocked.iter().any(|candidate| candidate == bb))
        });
    }
    if let Some(ref required) = require_bbs {
        routes.retain(|r| {
            r.building_blocks
                .iter()
                .any(|bb| required.iter().any(|candidate| candidate == bb))
        });
    }
    if let Some(max_cost) = max_route_cost {
        routes.retain(|r| r.route_cost <= max_cost);
    }
    if let Some(v) = min_confidence {
        routes.retain(|r| r.confidence >= v);
    }
    if let Some(v) = min_success_prob {
        routes.retain(|r| r.success_probability >= v);
    }
    if let Some(ref fams) = require_fams {
        routes.retain(|r| {
            r.steps.iter().any(|s| {
                s.reaction_family
                    .as_deref()
                    .is_some_and(|f| fams.iter().any(|required| required == f))
            })
        });
    }
    if let Some(ref fams) = avoid_fams {
        routes.retain(|r| {
            !r.steps.iter().any(|s| {
                s.reaction_family
                    .as_deref()
                    .is_some_and(|f| fams.iter().any(|avoided| avoided == f))
            })
        });
    }
    if let Some(ref fams) = prefer_fams {
        routes.sort_by_key(|r| {
            let has = r.steps.iter().any(|s| {
                s.reaction_family
                    .as_deref()
                    .is_some_and(|f| fams.iter().any(|p| p == f))
            });
            u8::from(!has)
        });
    }

    if routes.is_empty() {
        return json!({"content": [{"type": "text", "text":
            format!("No routes found for {smiles} matching the given constraints.")}]});
    }
    let mut text = format!(
        "Target: {smiles}\nRoutes after constraints: {}\n\n",
        routes.len()
    );
    for (i, route) in routes.iter().enumerate() {
        text.push_str(&format_route_tree(route, smiles, i + 1));
        text.push_str(&format!(
            "  confidence={:.2}  success_P={:.2}  cost={:.2}  BBs: {}\n\n",
            route.confidence,
            route.success_probability,
            route.route_cost,
            route.building_blocks.join(", ")
        ));
    }
    json!({"content": [{"type": "text", "text": text}]})
}

fn handle_find_pareto_routes(smiles: &str, args: &Value) -> Value {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let max_routes = args["max_routes"].as_u64().unwrap_or(10) as usize;
    let obj_spec = args["objectives"]
        .as_str()
        .unwrap_or("cost:min,success_probability:max,steps:min");

    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        ..Default::default()
    };
    let (routes, _) = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("search error: {e}")),
    };
    if routes.is_empty() {
        return json!({"content": [{"type": "text", "text":
            format!("No routes found for {smiles}.")}]});
    }

    // ponytail: duplicated from main.rs — lift to lib if a 3rd caller appears.
    let objs = mcp_parse_objectives(obj_spec);
    let front = mcp_pareto_front(&routes, &objs);

    let mut text = format!(
        "Target: {smiles}\nSearched: {} routes  Pareto front: {} routes\nObjectives: {}\n\n",
        routes.len(),
        front.len(),
        obj_spec
    );
    for (rank, &idx) in front.iter().enumerate() {
        let r = &routes[idx];
        let label = mcp_tradeoff_label(idx, &front, &routes, &objs);
        text.push_str(&format!(
            "Route {} (#{} overall){}\n  cost={:.2}  success_P={:.2}  steps={}  confidence={:.2}\n  BBs: {}\n\n",
            rank + 1, idx + 1,
            label.map(|l| format!("  [{l}]")).unwrap_or_default(),
            r.route_cost, r.success_probability, r.steps.len(), r.confidence,
            r.building_blocks.join(", ")
        ));
    }
    json!({"content": [{"type": "text", "text": text}]})
}

// Pareto helpers (duplicated from main.rs — see ponytail comment above)
fn mcp_parse_objectives(spec: &str) -> Vec<(u8, bool)> {
    // Encoding: field as u8 index, direction as bool (true=min)
    // 0=cost 1=success_prob 2=steps 3=depth 4=confidence 5=convergency 6=atom_economy
    spec.split(',')
        .filter_map(|part| {
            let (f, d) = part.trim().split_once(':')?;
            let field = match f.trim() {
                "cost" => 0u8,
                "success_probability" | "success" => 1,
                "steps" => 2,
                "depth" => 3,
                "confidence" => 4,
                "convergency" => 5,
                "atom_economy" => 6,
                _ => return None,
            };
            let minimize = d.trim() == "min";
            Some((field, minimize))
        })
        .collect()
}

/// Route-level atom-economy objective: `None` (not evaluable) as soon as any
/// step isn't `Normal`, rather than silently averaging over only the
/// evaluable steps and hiding the rest (Issue #79 review round 2; mirrors
/// main.rs's `atom_economy_objective`).
fn mcp_atom_economy_objective(r: &search::Route) -> Option<f64> {
    if r.steps.is_empty()
        || r.steps
            .iter()
            .any(|s| s.atom_economy_status != search::AtomEconomyStatus::Normal)
    {
        return None;
    }
    let sum: f64 = r
        .steps
        .iter()
        .map(|s| s.atom_economy.expect("Normal must carry a value"))
        .sum();
    Some(sum / r.steps.len() as f64)
}

/// `None` for every field means "not evaluable"; only atom_economy (field 6)
/// can currently produce one. Every other field is always `Some`.
fn mcp_obj_val(r: &search::Route, field: u8) -> Option<f64> {
    match field {
        0 => Some(r.route_cost),
        1 => Some(r.success_probability),
        2 => Some(r.steps.len() as f64),
        3 => Some(r.depth as f64),
        4 => Some(r.confidence),
        5 => Some(r.convergency),
        _ => mcp_atom_economy_objective(r),
    }
}

/// Compares `b`'s value against `a`'s, returning `(b_is_better, b_is_worse)`.
/// A `None` (not-evaluable) value is always worse than any `Some` value,
/// regardless of direction -- evaluable beats non-evaluable, never
/// converted to 0 or ±infinity. Two `None`s tie.
fn mcp_obj_compare(minimize: bool, a: Option<f64>, b: Option<f64>) -> (bool, bool) {
    match (a, b) {
        (None, None) => (false, false),
        (Some(_), None) => (false, true),
        (None, Some(_)) => (true, false),
        (Some(va), Some(vb)) => {
            if minimize {
                (vb < va, vb > va)
            } else {
                (vb > va, vb < va)
            }
        }
    }
}

fn mcp_pareto_front(routes: &[search::Route], objs: &[(u8, bool)]) -> Vec<usize> {
    (0..routes.len())
        .filter(|&i| {
            !(0..routes.len()).any(|j| {
                if j == i {
                    return false;
                }
                let mut all_no_worse = true;
                let mut any_better = false;
                for &(f, minimize) in objs {
                    let va = mcp_obj_val(&routes[i], f);
                    let vb = mcp_obj_val(&routes[j], f);
                    let (b_better, b_worse) = mcp_obj_compare(minimize, va, vb);
                    if b_worse {
                        all_no_worse = false;
                    }
                    if b_better {
                        any_better = true;
                    }
                }
                all_no_worse && any_better
            })
        })
        .collect()
}

fn mcp_tradeoff_label(
    idx: usize,
    front: &[usize],
    routes: &[search::Route],
    objs: &[(u8, bool)],
) -> Option<String> {
    let names = [
        "cheapest",
        "most_reliable",
        "shortest",
        "shallowest",
        "highest_confidence",
        "most_convergent",
        "best_atom_economy",
    ];
    let mut labels = Vec::new();
    for &(f, minimize) in objs {
        let my = mcp_obj_val(&routes[idx], f);
        // A route whose own value on this objective isn't evaluable is
        // never the unique best on it, even on a singleton front.
        if my.is_none() {
            continue;
        }
        if front.iter().filter(|&&j| j != idx).all(|&j| {
            let o = mcp_obj_val(&routes[j], f);
            mcp_obj_compare(minimize, o, my).0
        }) && let Some(name) = names.get(f as usize)
        {
            labels.push(*name);
        }
    }
    if labels.is_empty() {
        None
    } else {
        Some(labels.join("_and_"))
    }
}

fn handle_validate_route(smiles: &str, args: &Value) -> Value {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes: 1,
        ..Default::default()
    };

    let (routes, _) = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("search error: {e}")),
    };

    if routes.is_empty() {
        return json!({"content": [{"type": "text", "text":
            format!("No routes found for {smiles}.")}]});
    }
    let route = &routes[0];
    let mut text = format!(
        "Target: {smiles}\nValidating best route ({} step(s)):\n\n",
        route.steps.len()
    );
    let mut all_ok = true;
    for (i, step) in route.steps.iter().enumerate() {
        let ok = step_balanced(&step.target, &step.precursors);
        if !ok {
            all_ok = false;
        }
        text.push_str(&format!(
            "Step {}: {} → [{}]  atom_balance={}\n",
            i + 1,
            step.target,
            step.precursors.join(", "),
            if ok { "✓" } else { "✗ FAIL" },
        ));
    }
    text.push_str(&format!(
        "\nOverall: {}  confidence={:.2}  success_probability={:.2}",
        if all_ok {
            "PASS ✓"
        } else {
            "FAIL ✗ (atom imbalance detected)"
        },
        route.confidence,
        route.success_probability,
    ));
    json!({"content": [{"type": "text", "text": text}]})
}

fn route_diversity(routes: &[Route]) -> f64 {
    if routes.len() < 2 {
        return 0.0;
    }
    let mut total_sim = 0.0;
    let mut count = 0usize;
    for i in 0..routes.len() {
        for j in (i + 1)..routes.len() {
            let a: std::collections::HashSet<&str> = routes[i]
                .building_blocks
                .iter()
                .map(|s| s.as_str())
                .collect();
            let b: std::collections::HashSet<&str> = routes[j]
                .building_blocks
                .iter()
                .map(|s| s.as_str())
                .collect();
            let inter = a.intersection(&b).count();
            let union = a.len() + b.len() - inter;
            total_sim += if union == 0 {
                1.0
            } else {
                inter as f64 / union as f64
            };
            count += 1;
        }
    }
    1.0 - (total_sim / count as f64)
}

fn handle_estimate_diversity(smiles: &str, args: &Value) -> Value {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let max_routes = args["max_routes"].as_u64().unwrap_or(5) as usize;
    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        ..Default::default()
    };

    let (routes, _) = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("search error: {e}")),
    };

    if routes.is_empty() {
        return json!({"content": [{"type": "text", "text":
            format!("No routes found for {smiles}.")}]});
    }
    let diversity = route_diversity(&routes);
    let mut text = format!(
        "Target: {smiles}\nRoutes found: {}  Route diversity: {:.3}\n\n",
        routes.len(),
        diversity
    );
    text.push_str(if diversity > 0.5 {
        "High diversity — multiple distinct synthetic strategies available.\n"
    } else if diversity > 0.0 {
        "Moderate diversity — routes share some building blocks.\n"
    } else {
        "Low diversity — all routes use the same building blocks.\n"
    });
    text.push_str("\nBuilding block sets per route:\n");
    for (i, route) in routes.iter().enumerate() {
        text.push_str(&format!(
            "  Route {}: [{}]\n",
            i + 1,
            route.building_blocks.join(", ")
        ));
    }
    json!({"content": [{"type": "text", "text": text}]})
}

fn handle_diagnose_failure(smiles: &str, args: &Value) -> Value {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes: 1,
        ..Default::default()
    };
    let (routes, stats) = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("search error: {e}")),
    };

    if !routes.is_empty() {
        return json!({"content": [{"type": "text", "text":
            format!("Routes found for {smiles} — no failure to diagnose. Use find_routes to see them.")}]});
    }

    let mut causes: Vec<&str> = Vec::new();
    let mut suggestions: Vec<String> = Vec::new();

    if stats.stock_hits == 0 {
        causes.push("no building block in the default stock matched any search node");
        suggestions
            .push("provide a larger stock file via the building_blocks server config".to_string());
    }
    if stats.max_depth_reached {
        causes.push("search depth exhausted before reaching building blocks");
        suggestions.push(format!("retry with depth={}", depth + 2));
    }
    if stats.beam_limit_hit {
        causes.push("beam width was too narrow — promising nodes were pruned");
        suggestions.push("retry find_routes with a larger beam_width (e.g. 200)".to_string());
    }
    if stats.matched_templates < 5 {
        causes.push("very few templates matched the target structure");
        suggestions.push(
            "the target may contain unusual functional groups not covered by current templates"
                .to_string(),
        );
    }
    if causes.is_empty() {
        causes.push("unknown — search exhausted without finding a route");
        suggestions.push("try increasing depth, or check whether the SMILES is valid".to_string());
    }

    let text = format!(
        "Diagnosis for: {smiles}\nSearch stats: nodes_expanded={}, matched_templates={}, stock_hits={}\n\nLikely causes:\n{}\n\nSuggestions:\n{}",
        stats.nodes_expanded,
        stats.matched_templates,
        stats.stock_hits,
        causes
            .iter()
            .map(|c| format!("  • {c}"))
            .collect::<Vec<_>>()
            .join("\n"),
        suggestions
            .iter()
            .enumerate()
            .map(|(i, s)| format!("  {}. {s}", i + 1))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    json!({"content": [{"type": "text", "text": text}]})
}

fn tool_error(msg: &str) -> Value {
    json!({"content": [{"type": "text", "text": msg}], "isError": true})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_routes_schema_advertises_coverage_mode() {
        let tools = handle_tools_list();
        let find_routes = &tools["tools"].as_array().expect("tools must be an array")[0];
        let properties = &find_routes["inputSchema"]["properties"];
        assert_eq!(
            properties["search_mode"]["enum"],
            json!(["standard", "coverage"])
        );
        assert!(properties["coverage_templates"].is_object());
        assert!(properties["coverage_timeout_secs"].is_object());
    }

    #[test]
    fn coverage_mode_requires_template_path() {
        let response = handle_tools_call(&json!({
            "params": {
                "name": "find_routes",
                "arguments": {"smiles": "CCO", "search_mode": "coverage"}
            }
        }));
        assert_eq!(response["isError"], json!(true));
        assert!(
            response["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("requires coverage_templates")
        );
    }

    #[test]
    fn plan_with_constraints_schema_advertises_building_block_filters() {
        let tools = handle_tools_list();
        let plan = tools["tools"]
            .as_array()
            .expect("tools must be an array")
            .iter()
            .find(|tool| tool["name"] == "plan_with_constraints")
            .expect("plan_with_constraints must be advertised");
        let properties = &plan["inputSchema"]["properties"];
        assert!(properties["avoid_building_blocks"].is_object());
        assert!(properties["require_building_blocks"].is_object());
    }
}
