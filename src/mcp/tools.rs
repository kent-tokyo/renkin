//! RENKIN tool definitions and business logic for the MCP server.
//!
//! Nothing in this module knows about JSON-RPC framing or protocol eras
//! (legacy 2024-11-05 vs. modern 2026-07-28) — handlers take `(smiles, args)`
//! and return a [`ToolOutcome`]; [`crate::mcp::protocol`] renders that into
//! the era-appropriate wire shape.

use crate::DEFAULT_BUILDING_BLOCKS;
use crate::chem_env::{self, elem_symbols_to_mask};
use crate::display::{explain_route, format_route_tree};
use crate::search::{self, Route, SearchConfig};
use crate::validation::step_balanced;
use serde_json::{Value, json};

/// The result of running a RENKIN tool, independent of wire era.
pub struct ToolOutcome {
    pub text: String,
    pub structured: Option<Value>,
    pub is_error: bool,
}

impl ToolOutcome {
    fn ok(text: String) -> Self {
        ToolOutcome {
            text,
            structured: None,
            is_error: false,
        }
    }

    fn ok_structured(text: String, structured: Value) -> Self {
        ToolOutcome {
            text,
            structured: Some(structured),
            is_error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        ToolOutcome {
            text: text.into(),
            structured: None,
            is_error: true,
        }
    }

    /// Legacy 2024-11-05 `tools/call` result shape — unchanged from the
    /// pre-refactor implementation (see `tests/fixtures/mcp/2024-11-05/`).
    pub fn to_legacy_value(&self) -> Value {
        let mut v = json!({"content": [{"type": "text", "text": self.text}]});
        if self.is_error {
            v["isError"] = json!(true);
        }
        v
    }

    /// Modern 2026-07-28 `tools/call` result shape (pre-envelope: the
    /// `resultType` / `_meta.serverInfo` wrapping happens in `protocol.rs`).
    pub fn to_modern_value(&self) -> Value {
        let mut v = json!({"content": [{"type": "text", "text": self.text}]});
        if let Some(s) = &self.structured {
            v["structuredContent"] = s.clone();
        }
        if self.is_error {
            v["isError"] = json!(true);
        }
        v
    }
}

type ToolHandler = fn(smiles: &str, args: &Value) -> ToolOutcome;

pub struct ToolDefinition {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Schema exposed to 2024-11-05 clients — frozen; do not add constraints
    /// here without a paired legacy-transcript regression check.
    pub legacy_input_schema: fn() -> Value,
    /// Schema exposed to 2026-07-28 clients: JSON Schema 2020-12,
    /// `additionalProperties: false`, and numeric bounds. Every bound here
    /// must have a matching rejection path in `validate_modern_args`.
    pub modern_input_schema: fn() -> Value,
    pub modern_output_schema: Option<fn() -> Value>,
    pub handler: ToolHandler,
}

/// Declaration order is the wire order for `tools/list` in both eras (tested
/// by `tools_list_order_is_deterministic`) — this matches the pre-refactor
/// binary's order exactly, which the legacy golden transcript pins down.
pub static TOOLS: &[ToolDefinition] = &[
    ToolDefinition {
        name: "find_routes",
        title: "Find retrosynthetic routes",
        description: "Find retrosynthetic routes for a target molecule back to commercially available building blocks. Uses A* / AND-OR tree search with SMIRKS templates and commercially available building blocks.",
        legacy_input_schema: legacy_schema_find_routes,
        modern_input_schema: modern_schema_find_routes,
        modern_output_schema: None,
        handler: handle_find_routes,
    },
    ToolDefinition {
        name: "validate_route",
        title: "Validate a retrosynthetic route",
        description: "Find the best retrosynthetic route for a target molecule and validate it: check atom balance of each step (target_MW ≤ Σ precursor_MW) and report confidence/probability scores.",
        legacy_input_schema: legacy_schema_validate_route,
        modern_input_schema: modern_schema_validate_route,
        modern_output_schema: Some(output_schema_validate_route),
        handler: handle_validate_route,
    },
    ToolDefinition {
        name: "explain_route",
        title: "Explain a retrosynthetic route",
        description: "Find retrosynthetic routes for a target and return a human-readable explanation of the top route(s): strengths, weaknesses, and per-step details derived from confidence, success_probability, atom_economy, and reaction_family.",
        legacy_input_schema: legacy_schema_explain_route,
        modern_input_schema: modern_schema_explain_route,
        modern_output_schema: None,
        handler: handle_explain_route,
    },
    ToolDefinition {
        name: "find_pareto_routes",
        title: "Find Pareto-optimal routes",
        description: "Find retrosynthetic routes for a target and return the Pareto-optimal subset across multiple objectives (route_cost, success_probability, steps, etc.). Each Pareto route is non-dominated — no other route is better on all objectives simultaneously.",
        legacy_input_schema: legacy_schema_find_pareto_routes,
        modern_input_schema: modern_schema_find_pareto_routes,
        modern_output_schema: None,
        handler: handle_find_pareto_routes,
    },
    ToolDefinition {
        name: "plan_with_constraints",
        title: "Plan a route under constraints",
        description: "Find retrosynthetic routes applying explicit constraints: avoid elements, require elements, max steps, min confidence, min success probability, preferred reaction families. Designed for LLM-driven synthesis planning (Project Ariadne style).",
        legacy_input_schema: legacy_schema_plan_with_constraints,
        modern_input_schema: modern_schema_plan_with_constraints,
        modern_output_schema: None,
        handler: handle_plan_with_constraints,
    },
    ToolDefinition {
        name: "estimate_diversity",
        title: "Estimate route diversity",
        description: "Find multiple retrosynthetic routes for a target molecule and report the route diversity score (1 - avg pairwise Jaccard similarity of building-block sets). Higher = more diverse options available.",
        legacy_input_schema: legacy_schema_estimate_diversity,
        modern_input_schema: modern_schema_estimate_diversity,
        modern_output_schema: Some(output_schema_estimate_diversity),
        handler: handle_estimate_diversity,
    },
    ToolDefinition {
        name: "diagnose_failure",
        title: "Diagnose a search failure",
        description: "Diagnose why no retrosynthetic route was found for a target molecule. Runs the search and analyses SearchStats to identify likely causes (depth exhausted, no matching templates, beam too narrow, no building block matches) and returns actionable suggestions.",
        legacy_input_schema: legacy_schema_diagnose_failure,
        modern_input_schema: modern_schema_diagnose_failure,
        modern_output_schema: Some(output_schema_diagnose_failure),
        handler: handle_diagnose_failure,
    },
];

pub fn find(name: &str) -> Option<&'static ToolDefinition> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Legacy 2024-11-05 `tools/call` dispatch — byte-for-byte the pre-refactor
/// behavior, including the pre-existing "unknown tool name falls back to
/// find_routes" quirk. Do not "fix" this here: it is a legacy-compat
/// guarantee, not a design choice. The modern era's own dispatch (in
/// `protocol.rs`) does not use this function.
pub fn dispatch_legacy(msg_params: &Value) -> ToolOutcome {
    let tool_name = msg_params["name"].as_str().unwrap_or("find_routes");
    let args = &msg_params["arguments"];
    let smiles = match args["smiles"].as_str() {
        Some(s) => s,
        None => return ToolOutcome::error("missing required argument: smiles"),
    };
    let handler = find(tool_name)
        .map(|t| t.handler)
        .unwrap_or(handle_find_routes);
    handler(smiles, args)
}

/// Minimal JSON Schema 2020-12 subset check covering exactly the vocabulary
/// RENKIN's own tool schemas use (`type`, `required`, `minimum`, `maximum`,
/// `additionalProperties`). Not a general-purpose validator.
pub fn validate_modern_args(schema: &Value, args: &Value) -> Result<(), String> {
    let Some(args_obj) = args.as_object() else {
        return Err("arguments must be an object".to_string());
    };
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for name in required.iter().filter_map(Value::as_str) {
            if !args_obj.contains_key(name) {
                return Err(format!("missing required argument: {name}"));
            }
        }
    }
    let props = schema.get("properties").and_then(Value::as_object);
    let additional_properties_allowed =
        schema.get("additionalProperties") != Some(&Value::Bool(false));
    for (key, value) in args_obj {
        let Some(prop_schema) = props.and_then(|p| p.get(key)) else {
            if !additional_properties_allowed {
                return Err(format!("unexpected argument: {key}"));
            }
            continue;
        };
        let ty = prop_schema.get("type").and_then(Value::as_str);
        let type_ok = match ty {
            Some("string") => value.is_string(),
            Some("integer") => value.is_i64() || value.is_u64(),
            Some("number") => value.is_number(),
            Some("boolean") => value.is_boolean(),
            _ => true,
        };
        if !type_ok {
            return Err(format!(
                "argument {key} has the wrong type (expected {})",
                ty.unwrap_or("unknown")
            ));
        }
        if let Some(n) = value.as_f64() {
            if let Some(min) = prop_schema.get("minimum").and_then(Value::as_f64)
                && n < min
            {
                return Err(format!("argument {key}={n} is below minimum {min}"));
            }
            if let Some(max) = prop_schema.get("maximum").and_then(Value::as_f64)
                && n > max
            {
                return Err(format!("argument {key}={n} is above maximum {max}"));
            }
        }
    }
    Ok(())
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

fn handle_find_routes(smiles: &str, args: &Value) -> ToolOutcome {
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
        Err(e) => return ToolOutcome::error(format!("search error: {e}")),
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
    ToolOutcome::ok(text)
}

fn handle_explain_route(smiles: &str, args: &Value) -> ToolOutcome {
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
        Err(e) => return ToolOutcome::error(format!("search error: {e}")),
    };
    if routes.is_empty() {
        return ToolOutcome::ok(format!("No routes found for {smiles}."));
    }
    let text: String = routes
        .iter()
        .enumerate()
        .map(|(i, r)| explain_route(r, smiles, i + 1))
        .collect();
    ToolOutcome::ok(text)
}

fn handle_plan_with_constraints(smiles: &str, args: &Value) -> ToolOutcome {
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
        Err(e) => return ToolOutcome::error(format!("search error: {e}")),
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
        return ToolOutcome::ok(format!(
            "No routes found for {smiles} matching the given constraints."
        ));
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
    ToolOutcome::ok(text)
}

fn handle_find_pareto_routes(smiles: &str, args: &Value) -> ToolOutcome {
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
        Err(e) => return ToolOutcome::error(format!("search error: {e}")),
    };
    if routes.is_empty() {
        return ToolOutcome::ok(format!("No routes found for {smiles}."));
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
    ToolOutcome::ok(text)
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
/// main.rs's `atom_economy_objective`, ported from `src/bin/mcp.rs`'s
/// pre-refactor fix in `de7e6d7` -- this module didn't exist yet when that
/// fix landed on `master`).
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

fn handle_validate_route(smiles: &str, args: &Value) -> ToolOutcome {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes: 1,
        ..Default::default()
    };

    let (routes, _) = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return ToolOutcome::error(format!("search error: {e}")),
    };

    if routes.is_empty() {
        return ToolOutcome::ok_structured(
            format!("No routes found for {smiles}."),
            json!({"routes_found": 0}),
        );
    }
    let route = &routes[0];
    let mut text = format!(
        "Target: {smiles}\nValidating best route ({} step(s)):\n\n",
        route.steps.len()
    );
    let mut all_ok = true;
    let mut steps_json = Vec::with_capacity(route.steps.len());
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
        steps_json.push(json!({
            "step": i + 1,
            "target": step.target,
            "precursors": step.precursors,
            "atom_balanced": ok,
        }));
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
    ToolOutcome::ok_structured(
        text,
        json!({
            "routes_found": routes.len(),
            "all_atom_balanced": all_ok,
            "confidence": route.confidence,
            "success_probability": route.success_probability,
            "steps": steps_json,
        }),
    )
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

fn handle_estimate_diversity(smiles: &str, args: &Value) -> ToolOutcome {
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
        Err(e) => return ToolOutcome::error(format!("search error: {e}")),
    };

    if routes.is_empty() {
        return ToolOutcome::ok_structured(
            format!("No routes found for {smiles}."),
            json!({"routes_found": 0}),
        );
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
    let mut bb_sets = Vec::with_capacity(routes.len());
    for (i, route) in routes.iter().enumerate() {
        text.push_str(&format!(
            "  Route {}: [{}]\n",
            i + 1,
            route.building_blocks.join(", ")
        ));
        bb_sets.push(route.building_blocks.clone());
    }
    ToolOutcome::ok_structured(
        text,
        json!({
            "routes_found": routes.len(),
            "diversity": diversity,
            "building_block_sets": bb_sets,
        }),
    )
}

fn handle_diagnose_failure(smiles: &str, args: &Value) -> ToolOutcome {
    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let (env, rules) = load_env_and_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes: 1,
        ..Default::default()
    };
    let (routes, stats) = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return ToolOutcome::error(format!("search error: {e}")),
    };

    if !routes.is_empty() {
        return ToolOutcome::ok_structured(
            format!(
                "Routes found for {smiles} — no failure to diagnose. Use find_routes to see them."
            ),
            json!({"routes_found": routes.len()}),
        );
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
    ToolOutcome::ok_structured(
        text,
        json!({
            "routes_found": 0,
            "nodes_expanded": stats.nodes_expanded,
            "matched_templates": stats.matched_templates,
            "stock_hits": stats.stock_hits,
            "causes": causes,
            "suggestions": suggestions,
        }),
    )
}

// ---------------------------------------------------------------------
// Legacy (2024-11-05) input schemas — frozen. Any edit here must be
// re-verified against tests/fixtures/mcp/2024-11-05/legacy_transcript_output.jsonl.
// The only permitted change is the removal of the stale "509 curated
// building blocks" claim from find_routes' description (done above).
// ---------------------------------------------------------------------

fn legacy_schema_find_routes() -> Value {
    json!({
        "type": "object",
        "properties": {
            "smiles": {"type": "string", "description": "Target molecule SMILES"},
            "depth": {"type": "integer", "description": "Max retrosynthesis depth (default: 5)"},
            "max_routes": {"type": "integer", "description": "Routes to return (default: 5)"},
            "avoid_elements": {"type": "string", "description": "Comma-separated elements to exclude from BBs (e.g. \"Br,I\")"},
            "require_elements": {"type": "string", "description": "Elements that must appear in ≥1 building block (e.g. \"B\")"}
        },
        "required": ["smiles"]
    })
}

fn legacy_schema_validate_route() -> Value {
    json!({
        "type": "object",
        "properties": {
            "smiles": {"type": "string", "description": "Target molecule SMILES"},
            "depth": {"type": "integer", "description": "Max search depth (default: 5)"}
        },
        "required": ["smiles"]
    })
}

fn legacy_schema_explain_route() -> Value {
    json!({
        "type": "object",
        "properties": {
            "smiles": {"type": "string", "description": "Target molecule SMILES"},
            "depth": {"type": "integer", "description": "Max search depth (default: 5)"},
            "max_routes": {"type": "integer", "description": "Routes to explain (default: 1)"}
        },
        "required": ["smiles"]
    })
}

fn legacy_schema_find_pareto_routes() -> Value {
    json!({
        "type": "object",
        "properties": {
            "smiles": {"type": "string", "description": "Target molecule SMILES"},
            "depth": {"type": "integer", "description": "Max search depth (default: 5)"},
            "max_routes": {"type": "integer", "description": "Routes to search before computing Pareto front (default: 10)"},
            "objectives": {"type": "string", "description": "Comma-separated objectives, e.g. \"cost:min,success_probability:max,steps:min\" (default)"}
        },
        "required": ["smiles"]
    })
}

fn legacy_schema_plan_with_constraints() -> Value {
    json!({
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
    })
}

fn legacy_schema_estimate_diversity() -> Value {
    json!({
        "type": "object",
        "properties": {
            "smiles": {"type": "string", "description": "Target molecule SMILES"},
            "max_routes": {"type": "integer", "description": "Number of routes to compare (default: 5)"},
            "depth": {"type": "integer", "description": "Max search depth (default: 5)"}
        },
        "required": ["smiles"]
    })
}

fn legacy_schema_diagnose_failure() -> Value {
    json!({
        "type": "object",
        "properties": {
            "smiles": {"type": "string", "description": "Target molecule SMILES"},
            "depth": {"type": "integer", "description": "Max search depth (default: 5)"}
        },
        "required": ["smiles"]
    })
}

// ---------------------------------------------------------------------
// Modern (2026-07-28) input/output schemas — JSON Schema 2020-12,
// additionalProperties: false, numeric bounds matched by
// `validate_modern_args` (called before every modern tools/call).
// Bounds are only added where the codebase already documents them
// (depth, max_routes per this PR's spec; min_confidence /
// min_success_probability from their pre-existing "[0,1]" docstrings).
// ---------------------------------------------------------------------

const SCHEMA_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

fn depth_prop(default: u64) -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": 20, "default": default, "description": format!("Max search depth (default: {default})")})
}

fn max_routes_prop(default: u64, description: &str) -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": 100, "default": default, "description": description})
}

fn smiles_prop() -> Value {
    json!({"type": "string", "description": "Target molecule SMILES"})
}

fn modern_schema_find_routes() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "smiles": smiles_prop(),
            "depth": depth_prop(5),
            "max_routes": max_routes_prop(5, "Routes to return (default: 5)"),
            "avoid_elements": {"type": "string", "description": "Comma-separated elements to exclude from BBs (e.g. \"Br,I\")"},
            "require_elements": {"type": "string", "description": "Elements that must appear in ≥1 building block (e.g. \"B\")"}
        },
        "required": ["smiles"],
        "additionalProperties": false
    })
}

fn modern_schema_validate_route() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "smiles": smiles_prop(),
            "depth": depth_prop(5)
        },
        "required": ["smiles"],
        "additionalProperties": false
    })
}

fn output_schema_validate_route() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "routes_found": {"type": "integer"},
            "all_atom_balanced": {"type": "boolean"},
            "confidence": {"type": "number"},
            "success_probability": {"type": "number"},
            "steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "step": {"type": "integer"},
                        "target": {"type": "string"},
                        "precursors": {"type": "array", "items": {"type": "string"}},
                        "atom_balanced": {"type": "boolean"}
                    },
                    "required": ["step", "target", "precursors", "atom_balanced"]
                }
            }
        },
        "required": ["routes_found"]
    })
}

fn modern_schema_explain_route() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "smiles": smiles_prop(),
            "depth": depth_prop(5),
            "max_routes": max_routes_prop(1, "Routes to explain (default: 1)")
        },
        "required": ["smiles"],
        "additionalProperties": false
    })
}

fn modern_schema_find_pareto_routes() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "smiles": smiles_prop(),
            "depth": depth_prop(5),
            "max_routes": max_routes_prop(10, "Routes to search before computing Pareto front (default: 10)"),
            "objectives": {"type": "string", "description": "Comma-separated objectives, e.g. \"cost:min,success_probability:max,steps:min\" (default)"}
        },
        "required": ["smiles"],
        "additionalProperties": false
    })
}

fn modern_schema_plan_with_constraints() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "smiles": smiles_prop(),
            "depth": depth_prop(5),
            "max_routes": max_routes_prop(5, "Max routes to return (default: 5)"),
            "avoid_elements": {"type": "string", "description": "Comma-separated elements to ban from BBs (e.g. \"Br,I\")"},
            "require_elements": {"type": "string", "description": "Elements that must appear in ≥1 BB (e.g. \"B\")"},
            "max_steps": {"type": "integer", "minimum": 1, "description": "Maximum number of synthesis steps per route"},
            "min_confidence": {"type": "number", "minimum": 0, "maximum": 1, "description": "Minimum template confidence [0,1]"},
            "min_success_probability": {"type": "number", "minimum": 0, "maximum": 1, "description": "Minimum route success probability [0,1]"},
            "prefer_reaction_families": {"type": "string", "description": "Comma-separated reaction families to rank first (e.g. \"amide_coupling,suzuki_retro\")"}
        },
        "required": ["smiles"],
        "additionalProperties": false
    })
}

fn modern_schema_estimate_diversity() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "smiles": smiles_prop(),
            "max_routes": max_routes_prop(5, "Number of routes to compare (default: 5)"),
            "depth": depth_prop(5)
        },
        "required": ["smiles"],
        "additionalProperties": false
    })
}

fn output_schema_estimate_diversity() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "routes_found": {"type": "integer"},
            "diversity": {"type": "number"},
            "building_block_sets": {"type": "array", "items": {"type": "array", "items": {"type": "string"}}}
        },
        "required": ["routes_found"]
    })
}

fn modern_schema_diagnose_failure() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "smiles": smiles_prop(),
            "depth": depth_prop(5)
        },
        "required": ["smiles"],
        "additionalProperties": false
    })
}

fn output_schema_diagnose_failure() -> Value {
    json!({
        "$schema": SCHEMA_2020_12,
        "type": "object",
        "properties": {
            "routes_found": {"type": "integer"},
            "nodes_expanded": {"type": "integer"},
            "matched_templates": {"type": "integer"},
            "stock_hits": {"type": "integer"},
            "causes": {"type": "array", "items": {"type": "string"}},
            "suggestions": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["routes_found"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_order_is_deterministic_and_matches_legacy_transcript() {
        let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "find_routes",
                "validate_route",
                "explain_route",
                "find_pareto_routes",
                "plan_with_constraints",
                "estimate_diversity",
                "diagnose_failure",
            ]
        );
    }

    #[test]
    fn every_modern_schema_is_object_rooted_and_2020_12() {
        for t in TOOLS {
            let s = (t.modern_input_schema)();
            assert_eq!(s["type"], "object");
            assert_eq!(s["$schema"], SCHEMA_2020_12);
            assert_eq!(s["additionalProperties"], false);
        }
    }

    #[test]
    fn legacy_schemas_have_no_modern_only_fields() {
        for t in TOOLS {
            let s = (t.legacy_input_schema)();
            assert!(s.get("$schema").is_none());
            assert!(s.get("additionalProperties").is_none());
        }
    }

    #[test]
    fn find_routes_description_has_no_stale_fixed_count() {
        let def = find("find_routes").unwrap();
        assert!(!def.description.contains("509"));
    }

    #[test]
    fn validate_modern_args_enforces_depth_bounds() {
        let schema = modern_schema_find_routes();
        assert!(validate_modern_args(&schema, &json!({"smiles": "CCO", "depth": 5})).is_ok());
        assert!(validate_modern_args(&schema, &json!({"smiles": "CCO", "depth": 21})).is_err());
        assert!(validate_modern_args(&schema, &json!({"smiles": "CCO", "depth": 0})).is_err());
    }

    #[test]
    fn validate_modern_args_rejects_missing_required() {
        let schema = modern_schema_find_routes();
        assert!(validate_modern_args(&schema, &json!({})).is_err());
    }

    #[test]
    fn validate_modern_args_rejects_additional_properties() {
        let schema = modern_schema_find_routes();
        assert!(validate_modern_args(&schema, &json!({"smiles": "CCO", "bogus": 1})).is_err());
    }

    #[test]
    fn no_route_branches_still_conform_to_declared_output_schema() {
        // routes_found is the only required field in each of the three
        // output schemas below; the "no routes found" branch must still
        // emit it (see handle_validate_route / handle_estimate_diversity).
        for schema_fn in [
            output_schema_validate_route,
            output_schema_estimate_diversity,
        ] {
            let schema = schema_fn();
            let required: Vec<&str> = schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(required, vec!["routes_found"]);
        }
    }
}
