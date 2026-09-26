//! Self-contained HTML route report with 2D molecule depictions
//! (SynPlanner route-visualisation parity).
//!
//! Each route is drawn as a retrosynthetic tree: molecule cards (2D SVG
//! depiction from `chematic-depict`, canonical SMILES, and a terminal badge)
//! joined by reaction labels (rule name and template ID). The page has no
//! scripts and no external resources, so it can be archived or attached as
//! evidence as-is. Leaf badges keep RENKIN's identity policy visible: a leaf
//! is `stock` only when it is an exact stock member; an opt-in small-molecule
//! terminal is labelled `size terminal`, never `stock`.
//!
//! Only compiled with the `depict` feature (on by default for native builds;
//! WASM builds use `--no-default-features`).

use std::collections::HashMap;
use std::fmt::Write as _;

use chematic::depict::{RenderOptions, depict_svg_opts};

use crate::chem_env::{ChemEnv, mol_from_smiles};
use crate::search::{ReactionStep, Route};

/// Escape text for HTML element content and double-quoted attributes.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Leaf/intermediate classification shown on each molecule card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Target,
    Intermediate,
    Stock,
    SizeTerminal,
    UnresolvedLeaf,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Role::Target => "target",
            Role::Intermediate => "intermediate",
            Role::Stock => "stock",
            Role::SizeTerminal => "size terminal",
            Role::UnresolvedLeaf => "not in stock",
        }
    }
    fn class(self) -> &'static str {
        match self {
            Role::Target => "target",
            Role::Intermediate => "intermediate",
            Role::Stock => "stock",
            Role::SizeTerminal => "size",
            Role::UnresolvedLeaf => "unresolved",
        }
    }
}

struct Renderer<'a> {
    env: Option<&'a ChemEnv>,
    svg_cache: HashMap<String, String>,
    /// Without `env`: leaves of the current route already known to be
    /// non-stock size terminals (from a result's `small_molecule_terminal`).
    size_terminal_leaves: std::collections::HashSet<String>,
}

impl Renderer<'_> {
    fn svg(&mut self, smiles: &str) -> String {
        if let Some(svg) = self.svg_cache.get(smiles) {
            return svg.clone();
        }
        let svg = match mol_from_smiles(smiles) {
            Ok(mol) => {
                // Natural size from the layout bounding box: CSS caps large
                // molecules, and small ones are not blown up.
                let opts = RenderOptions {
                    padding: 12.0,
                    ..RenderOptions::default()
                };
                depict_svg_opts(&mol, &opts)
            }
            Err(_) => format!(
                "<div class=\"nodepict\">no depiction<br><code>{}</code></div>",
                escape(smiles)
            ),
        };
        self.svg_cache.insert(smiles.to_owned(), svg.clone());
        svg
    }

    fn leaf_role(&self, smiles: &str) -> Role {
        let Some(env) = self.env else {
            // Search output: every leaf of a completed route is a terminal;
            // the result itself says which ones are size terminals.
            return if self.size_terminal_leaves.contains(smiles) {
                Role::SizeTerminal
            } else {
                Role::Stock
            };
        };
        let in_stock = env.is_building_block_smiles(smiles)
            || mol_from_smiles(smiles).is_ok_and(|mol| env.is_building_block(&mol));
        if in_stock {
            Role::Stock
        } else if env.is_small_molecule_terminal_smiles(smiles) {
            Role::SizeTerminal
        } else {
            Role::UnresolvedLeaf
        }
    }

    fn card(&mut self, out: &mut String, smiles: &str, role: Role) {
        let svg = self.svg(smiles);
        let _ = write!(
            out,
            "<div class=\"mol {cls}\"><div class=\"pic\">{svg}</div>\
             <code class=\"smi\">{smi}</code><span class=\"badge\">{label}</span></div>",
            cls = role.class(),
            smi = escape(smiles),
            label = role.label(),
        );
    }

    fn node(
        &mut self,
        out: &mut String,
        smiles: &str,
        is_root: bool,
        by_target: &HashMap<&str, &ReactionStep>,
        on_path: &mut Vec<String>,
    ) {
        out.push_str("<div class=\"node\">");
        let step = by_target
            .get(smiles)
            .copied()
            .filter(|_| !on_path.iter().any(|s| s == smiles));
        match step {
            None => {
                let role = if is_root {
                    Role::Target
                } else {
                    self.leaf_role(smiles)
                };
                // A depth-0 route's target is itself the terminal.
                let role = if is_root && by_target.is_empty() {
                    self.leaf_role(smiles)
                } else {
                    role
                };
                self.card(out, smiles, role);
            }
            Some(step) => {
                let role = if is_root {
                    Role::Target
                } else {
                    Role::Intermediate
                };
                self.card(out, smiles, role);
                let _ = write!(
                    out,
                    "<div class=\"rxn\"><span class=\"arrow\">&#8658;</span> \
                     <b>{rule}</b> <code>{tid}</code></div><div class=\"children\">",
                    rule = escape(&step.rule),
                    tid = escape(&step.template_id),
                );
                on_path.push(smiles.to_owned());
                for precursor in &step.precursors {
                    self.node(out, precursor, false, by_target, on_path);
                }
                on_path.pop();
                out.push_str("</div>");
            }
        }
        out.push_str("</div>");
    }
}

fn root_of<'a>(route: &'a Route, fallback: &'a str) -> &'a str {
    let precursors: std::collections::HashSet<&str> = route
        .steps
        .iter()
        .flat_map(|s| s.precursors.iter().map(String::as_str))
        .collect();
    route
        .steps
        .iter()
        .map(|s| s.target.as_str())
        .find(|t| !precursors.contains(t))
        .or_else(|| route.steps.first().map(|s| s.target.as_str()))
        .unwrap_or(fallback)
}

const STYLE: &str = r#"
:root{--bg:#f7f7f5;--fg:#1d1d1b;--muted:#6b6b66;--card:#fff;--line:#c9c9c2;
--stock:#1f7a4d;--size:#9a6b00;--unres:#b3261e;--inter:#3a5a9b;--target:#5b3a9b}
@media (prefers-color-scheme:dark){:root{--bg:#161615;--fg:#ecece8;--muted:#a3a39c;
--card:#232321;--line:#4a4a45}}
*{box-sizing:border-box}body{margin:0;padding:24px 16px;background:var(--bg);color:var(--fg);
font:14px/1.45 system-ui,-apple-system,Segoe UI,sans-serif}
main{max-width:1200px;margin:0 auto}h1{font-size:20px;margin:0 0 4px}
.sub{color:var(--muted);margin:0 0 20px}.note{color:var(--muted);font-size:12px;margin-top:28px}
section.route{background:var(--card);border:1px solid var(--line);border-radius:10px;
padding:14px 16px;margin:0 0 18px;overflow-x:auto}
section.route h2{font-size:16px;margin:0 0 4px}.metrics{color:var(--muted);font-size:12px;margin:0 0 10px}
code{font:12px ui-monospace,SFMono-Regular,Menlo,monospace;word-break:break-all}
.tree{display:flex;align-items:flex-start}
.node{display:flex;flex-direction:column;align-items:flex-start;gap:4px}
.children{display:flex;flex-wrap:nowrap;align-items:flex-start;gap:14px;margin-left:14px;
padding:8px 0 4px 14px;border-left:2px solid var(--line)}
.mol{display:inline-flex;flex-direction:column;gap:4px;margin:6px 0;padding:6px;border-radius:8px;
border:1px solid var(--line);background:#fff;color:#1d1d1b;max-width:230px}
.mol .pic svg{display:block;width:auto;height:auto;max-width:200px;max-height:150px}
.mol.target{max-width:420px}.mol.target .pic svg{max-width:400px;max-height:280px}
.mol .smi{font-size:11px}
.badge{align-self:flex-start;font-size:11px;padding:1px 7px;border-radius:999px;color:#fff}
.stock .badge{background:var(--stock)}.size .badge{background:var(--size)}
.unresolved .badge{background:var(--unres)}.intermediate .badge{background:var(--inter)}
.target .badge{background:var(--target)}
.rxn{margin:0 0 0 6px;white-space:nowrap;color:var(--muted);font-size:12px}.rxn b{color:var(--fg)}
.arrow{font-size:14px}.nodepict{padding:20px;color:#6b6b66}
"#;

/// Render `routes` for `target` as a self-contained HTML page.
///
/// `env`, when given, classifies each leaf as exact `stock`, opt-in
/// `size terminal`, or `not in stock`. Without it every leaf of a completed
/// route is shown as a terminal (`stock`).
pub fn routes_html_report(target: &str, routes: &[Route], env: Option<&ChemEnv>) -> String {
    routes_html_report_with_size_terminals(target, routes, env, &[])
}

/// [`routes_html_report`] with, per route (same order), the leaves already
/// known to be non-stock size terminals. Used when rendering a saved result
/// without its stock (`env = None`).
pub fn routes_html_report_with_size_terminals(
    target: &str,
    routes: &[Route],
    env: Option<&ChemEnv>,
    size_terminals: &[Vec<String>],
) -> String {
    let mut renderer = Renderer {
        env,
        svg_cache: HashMap::new(),
        size_terminal_leaves: std::collections::HashSet::new(),
    };
    let mut body = String::new();
    if routes.is_empty() {
        body.push_str("<p>No route was found.</p>");
    }
    for (index, route) in routes.iter().enumerate() {
        let by_target: HashMap<&str, &ReactionStep> = route
            .steps
            .iter()
            .rev() // the first step for a target wins, matching display::build_tree
            .map(|s| (s.target.as_str(), s))
            .collect();
        let _ = write!(
            body,
            "<section class=\"route\"><h2>Route {n}</h2><p class=\"metrics\">{steps} step(s) · \
             depth {depth} · score {score:.3} · route cost {cost:.3} · success probability \
             {sp:.3} · building blocks {bbs}</p><div class=\"tree\">",
            n = index + 1,
            steps = route.steps.len(),
            depth = route.depth,
            score = route.score,
            cost = route.route_cost,
            sp = route.success_probability,
            bbs = route.building_blocks.len(),
        );
        renderer.size_terminal_leaves = size_terminals
            .get(index)
            .map(|leaves| leaves.iter().cloned().collect())
            .unwrap_or_default();
        let root = root_of(route, target);
        renderer.node(&mut body, root, true, &by_target, &mut Vec::new());
        body.push_str("</div></section>");
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>RENKIN routes</title><style>{STYLE}</style></head><body><main>\
         <h1>RENKIN routes</h1><p class=\"sub\">Target <code>{target}</code> · {count} route(s)</p>\
         {body}<p class=\"note\">Generated by RENKIN {version}. Depictions by chematic-depict. \
         A route is a structural retrosynthetic proposal, not a claim of laboratory \
         feasibility, yield, or safety. Badges: <b>stock</b> = exact standardized \
         stock member; <b>size terminal</b> = opt-in small-molecule terminal, not stock.</p>\
         </main></body></html>",
        target = escape(target),
        count = routes.len(),
        version = env!("CARGO_PKG_VERSION"),
    )
}

/// Render a saved RENKIN search result (the CLI's JSON output or Python
/// `find_routes()` string) without re-running the search. Only the fields
/// the report draws are read; `small_molecule_terminal.non_stock_leaves`,
/// when present, labels size terminals.
pub fn routes_html_from_result_json(
    result_json: &str,
    env: Option<&ChemEnv>,
) -> anyhow::Result<String> {
    #[derive(serde::Deserialize)]
    struct StepInput {
        rule: String,
        template_id: String,
        target: String,
        precursors: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    struct RouteInput {
        steps: Vec<StepInput>,
        #[serde(default)]
        depth: u32,
        #[serde(default)]
        score: f64,
        #[serde(default)]
        building_blocks: Vec<String>,
        #[serde(default)]
        route_cost: f64,
        #[serde(default)]
        success_probability: f64,
    }
    #[derive(serde::Deserialize, Default)]
    struct SizeInput {
        #[serde(default)]
        non_stock_leaves: Vec<Vec<String>>,
    }
    #[derive(serde::Deserialize)]
    struct ResultInput {
        target: String,
        #[serde(default)]
        routes: Vec<RouteInput>,
        #[serde(default)]
        small_molecule_terminal: Option<SizeInput>,
    }
    let parsed: ResultInput = serde_json::from_str(result_json)
        .map_err(|e| anyhow::anyhow!("not a RENKIN search result: {e}"))?;
    let routes: Vec<Route> = parsed
        .routes
        .into_iter()
        .map(|route| Route {
            steps: route
                .steps
                .into_iter()
                .map(|step| ReactionStep {
                    rule: step.rule,
                    template_id: step.template_id,
                    target: step.target,
                    precursors: step.precursors,
                    conditions: None,
                    atom_economy: None,
                    atom_economy_raw_percent: None,
                    atom_economy_status: crate::search::AtomEconomyStatus::NotEvaluable,
                    step_confidence: 0.0,
                    procedure_hint: None,
                    reaction_family: None,
                    metadata_source: None,
                    metadata_scope: None,
                    evidence: None,
                })
                .collect(),
            depth: route.depth,
            score: route.score,
            building_blocks: route.building_blocks,
            confidence: 0.0,
            convergency: 0.0,
            success_probability: route.success_probability,
            route_cost: route.route_cost,
        })
        .collect();
    let size_terminals = parsed
        .small_molecule_terminal
        .unwrap_or_default()
        .non_stock_leaves;
    Ok(routes_html_report_with_size_terminals(
        &parsed.target,
        &routes,
        env,
        &size_terminals,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::AtomEconomyStatus;

    fn step(target: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: "ester_cleavage".to_owned(),
            template_id: "rule:ester_cleavage".to_owned(),
            target: target.to_owned(),
            precursors: precursors.iter().map(|p| (*p).to_owned()).collect(),
            conditions: None,
            atom_economy: None,
            atom_economy_raw_percent: None,
            atom_economy_status: AtomEconomyStatus::NotEvaluable,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        }
    }

    fn route(steps: Vec<ReactionStep>, bbs: &[&str]) -> Route {
        Route {
            depth: steps.len() as u32,
            steps,
            score: 1.0,
            building_blocks: bbs.iter().map(|s| (*s).to_owned()).collect(),
            confidence: 1.0,
            convergency: 0.0,
            success_probability: 0.5,
            route_cost: 0.0,
        }
    }

    const ASPIRIN: &str = "CC(=O)Oc1ccccc1C(=O)O";

    #[test]
    fn report_is_self_contained_and_classifies_leaves() {
        let env = ChemEnv::in_memory(&["O=C(O)c1ccccc1O"]).with_small_molecule_terminal(4);
        let routes = vec![route(
            vec![step(ASPIRIN, &["CC(=O)O", "O=C(O)c1ccccc1O"])],
            &["CC(=O)O", "O=C(O)c1ccccc1O"],
        )];
        let html = routes_html_report(ASPIRIN, &routes, Some(&env));
        assert!(html.starts_with("<!doctype html>"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("http://") || html.contains("http://www.w3.org"));
        assert_eq!(html.matches("<svg").count(), 3, "target + two leaves");
        assert!(html.contains("class=\"mol target\""));
        assert!(html.contains("class=\"mol stock\""));
        assert!(
            html.contains("class=\"mol size\""),
            "acetic acid is a size terminal"
        );
        assert!(html.contains("rule:ester_cleavage"));
    }

    #[test]
    fn repeated_molecules_reuse_depictions_and_text_is_escaped() {
        let mut renderer = Renderer {
            env: None,
            svg_cache: HashMap::new(),
            size_terminal_leaves: std::collections::HashSet::new(),
        };
        let first = renderer.svg("CCO");
        let second = renderer.svg("CCO");
        assert_eq!(first, second);
        assert_eq!(renderer.svg_cache.len(), 1);
        assert!(renderer.svg("not((smiles").contains("no depiction"));
        assert_eq!(escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
    }

    #[test]
    fn saved_result_json_renders_with_size_terminal_labels() {
        let json = serde_json::json!({
            "target": ASPIRIN,
            "routes_found": 1,
            "routes": [{
                "steps": [{
                    "rule": "ester_cleavage",
                    "template_id": "rule:ester_cleavage",
                    "target": ASPIRIN,
                    "precursors": ["CC(=O)O", "O=C(O)c1ccccc1O"],
                    "step_confidence": 1.0
                }],
                "depth": 1,
                "score": 1.0,
                "building_blocks": ["CC(=O)O", "O=C(O)c1ccccc1O"],
                "route_cost": 0.0,
                "success_probability": 0.5
            }],
            "small_molecule_terminal": {"max_heavy_atoms": 4, "non_stock_leaves": [["CC(=O)O"]]}
        });
        let html = routes_html_from_result_json(&json.to_string(), None).unwrap();
        assert!(html.contains("class=\"mol size\""));
        assert!(html.contains("class=\"mol stock\""));
        assert!(routes_html_from_result_json("{}", None).is_err());
    }

    #[test]
    fn empty_and_depth_zero_routes_render() {
        let empty = routes_html_report(ASPIRIN, &[], None);
        assert!(empty.contains("No route was found."));
        let env = ChemEnv::in_memory(&["CCO"]);
        let buy = routes_html_report("CCO", &[route(Vec::new(), &["CCO"])], Some(&env));
        assert!(buy.contains("class=\"mol stock\""));
        assert_eq!(buy.matches("<svg").count(), 1);
    }
}
