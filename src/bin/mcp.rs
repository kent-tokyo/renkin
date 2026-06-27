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

use chematic::chem::molecular_weight;
use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::chem_env::{self, elem_symbols_to_mask, mol_from_smiles};
use renkin::display::{explain_route, format_route_tree};
use renkin::search::{self, Route, SearchConfig};
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
                "description": "Find retrosynthetic routes for a target molecule back to commercially available building blocks. Uses A* / AND-OR tree search with SMIRKS templates and 509 curated building blocks.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "smiles": {"type": "string", "description": "Target molecule SMILES"},
                        "depth": {"type": "integer", "description": "Max retrosynthesis depth (default: 5)"},
                        "max_routes": {"type": "integer", "description": "Routes to return (default: 5)"},
                        "avoid_elements": {"type": "string", "description": "Comma-separated elements to exclude from BBs (e.g. \"Br,I\")"},
                        "require_elements": {"type": "string", "description": "Elements that must appear in ≥1 building block (e.g. \"B\")"}
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
                "description": "Find retrosynthetic routes applying explicit constraints: avoid elements, require elements, max steps, min confidence, min success probability, preferred reaction families. Designed for LLM-driven synthesis planning (Project Ariadne style).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "smiles": {"type": "string", "description": "Target molecule SMILES"},
                        "depth": {"type": "integer", "description": "Max search depth (default: 5)"},
                        "max_routes": {"type": "integer", "description": "Max routes to return (default: 5)"},
                        "avoid_elements": {"type": "string", "description": "Comma-separated elements to ban from BBs (e.g. \"Br,I\")"},
                        "require_elements": {"type": "string", "description": "Elements that must appear in ≥1 BB (e.g. \"B\")"},
                        "max_steps": {"type": "integer", "description": "Maximum number of synthesis steps per route"},
                        "min_confidence": {"type": "number", "description": "Minimum template confidence [0,1]"},
                        "min_success_probability": {"type": "number", "description": "Minimum route success probability [0,1]"},
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
        _ => handle_find_routes(smiles, args),
    }
}

fn handle_find_routes(smiles: &str, args: &Value) -> Value {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let max_routes = args["max_routes"].as_u64().unwrap_or(5) as usize;
    let avoid = args["avoid_elements"].as_str().unwrap_or("");
    let require = args["require_elements"].as_str().unwrap_or("");

    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        forbidden_elements: elem_symbols_to_mask(avoid),
        required_element_present: elem_symbols_to_mask(require),
        ..Default::default()
    };

    let (routes, stats) = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("search error: {e}")),
    };

    let mut text = format!("Target: {smiles}\nRoutes found: {}\n\n", routes.len());
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
    let max_steps = args["max_steps"].as_u64().map(|n| n as usize);
    let min_confidence = args["min_confidence"].as_f64();
    let min_success_prob = args["min_success_probability"].as_f64();
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
    if let Some(v) = min_confidence {
        routes.retain(|r| r.confidence >= v);
    }
    if let Some(v) = min_success_prob {
        routes.retain(|r| r.success_probability >= v);
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

fn mcp_obj_val(r: &search::Route, field: u8) -> f64 {
    match field {
        0 => r.route_cost,
        1 => r.success_probability,
        2 => r.steps.len() as f64,
        3 => r.depth as f64,
        4 => r.confidence,
        5 => r.convergency,
        _ => {
            let v: Vec<f64> = r.steps.iter().filter_map(|s| s.atom_economy).collect();
            if v.is_empty() {
                0.0
            } else {
                v.iter().sum::<f64>() / v.len() as f64
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
                    let (b_better, b_worse) = if minimize {
                        (vb < va, vb > va)
                    } else {
                        (vb > va, vb < va)
                    };
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
        if front.iter().filter(|&&j| j != idx).all(|&j| {
            let o = mcp_obj_val(&routes[j], f);
            if minimize { my < o } else { my > o }
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

fn step_balanced(target: &str, precursors: &[String]) -> bool {
    let target_mw = mol_from_smiles(target)
        .ok()
        .map(|m| molecular_weight(&m))
        .unwrap_or(0.0);
    if target_mw == 0.0 {
        return true;
    }
    let precursor_mw: f64 = precursors
        .iter()
        .filter_map(|s| mol_from_smiles(s).ok())
        .map(|m| molecular_weight(&m))
        .sum();
    target_mw <= precursor_mw * 1.01
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

fn tool_error(msg: &str) -> Value {
    json!({"content": [{"type": "text", "text": msg}], "isError": true})
}
