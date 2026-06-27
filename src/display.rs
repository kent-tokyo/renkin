use std::collections::HashMap;

use crate::search::{ReactionStep, Route};

// ── Tree node ──────────────────────────────────────────────────────────────

struct TreeNode {
    smiles: String,
    rule: Option<String>,
    children: Vec<TreeNode>,
    is_bb: bool,
}

/// Recursively build a tree from a flat step list.
/// Each step maps target → precursors; steps without a matching parent step
/// are building blocks (leaves).
fn build_tree(steps: &[ReactionStep], root: &str) -> TreeNode {
    // Map target SMILES → (rule, precursors)
    let step_map: HashMap<&str, (&str, &[String])> = steps
        .iter()
        .map(|s| {
            (
                s.target.as_str(),
                (s.rule.as_str(), s.precursors.as_slice()),
            )
        })
        .collect();

    build_node(&step_map, root, None)
}

fn build_node<'a>(
    step_map: &HashMap<&'a str, (&'a str, &'a [String])>,
    smiles: &'a str,
    rule: Option<&'a str>,
) -> TreeNode {
    if let Some(&(r, precs)) = step_map.get(smiles) {
        TreeNode {
            smiles: smiles.to_string(),
            rule: Some(r.to_string()),
            children: precs
                .iter()
                .map(|p| build_node(step_map, p.as_str(), None))
                .collect(),
            is_bb: false,
        }
    } else {
        TreeNode {
            smiles: smiles.to_string(),
            rule: rule.map(str::to_string),
            children: vec![],
            is_bb: true,
        }
    }
}

// ── ASCII tree renderer ────────────────────────────────────────────────────

/// Find the canonical root SMILES from a route's steps.
/// The root is the unique step.target that is not a precursor of any other step.
fn find_root<'a>(steps: &'a [ReactionStep], fallback: &'a str) -> &'a str {
    if steps.is_empty() {
        return fallback;
    }
    let all_precursors: std::collections::HashSet<&str> = steps
        .iter()
        .flat_map(|s| s.precursors.iter().map(String::as_str))
        .collect();
    for step in steps {
        if !all_precursors.contains(step.target.as_str()) {
            return step.target.as_str();
        }
    }
    steps[0].target.as_str()
}

/// Format a route as a human-readable ASCII tree.
///
/// Example:
/// ```text
/// Route 1  [score=1.24, depth=2]
/// CC(=O)Oc1ccccc1C(=O)O
/// └── [ester_cleavage]
///     ├── OC(=O)c1ccccc1O  ✓ BB
///     └── CC(=O)O  ✓ BB
/// ```
pub fn format_route_tree(route: &Route, target: &str, route_num: usize) -> String {
    let root = find_root(&route.steps, target);
    let tree = build_tree(&route.steps, root);
    let mut out = String::new();
    out.push_str(&format!(
        "Route {}  [score={:.2}, depth={}]\n",
        route_num, route.score, route.depth
    ));
    render_node(&tree, &mut out, "", true);
    out
}

fn render_node(node: &TreeNode, out: &mut String, prefix: &str, is_last: bool) {
    let connector = if prefix.is_empty() {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };

    let bb_tag = if node.is_bb { "  ✓ BB" } else { "" };

    if node.is_bb {
        out.push_str(&format!("{prefix}{connector}{}{bb_tag}\n", node.smiles));
    } else {
        // Show the molecule, then the reaction rule on the next line
        out.push_str(&format!("{prefix}{connector}{}\n", node.smiles));
        if let Some(rule) = &node.rule {
            let rule_prefix = if prefix.is_empty() {
                String::new()
            } else if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };
            out.push_str(&format!("{}└── [{}]\n", rule_prefix, rule));
        }

        let rule_child_prefix = if prefix.is_empty() {
            "    ".to_string()
        } else if is_last {
            format!("{prefix}        ")
        } else {
            format!("{prefix}│       ")
        };

        for (i, child) in node.children.iter().enumerate() {
            let last = i == node.children.len() - 1;
            render_node(child, out, &rule_child_prefix, last);
        }
    }
}

// ── Mermaid renderer ───────────────────────────────────────────────────────

struct MermaidEdge {
    from: usize,
    to: usize,
    label: String,
}

struct MermaidNode {
    id: usize,
    label: String,
}

fn collect_mermaid(
    node: &TreeNode,
    nodes: &mut Vec<MermaidNode>,
    edges: &mut Vec<MermaidEdge>,
    counter: &mut usize,
    parent_id: Option<(usize, String)>,
) {
    let my_id = *counter;
    *counter += 1;

    let label = if node.is_bb {
        format!("{} ✓", node.smiles)
    } else {
        node.smiles.clone()
    };
    nodes.push(MermaidNode {
        id: my_id,
        label: label.replace('"', "'"),
    });

    if let Some((pid, rule)) = parent_id {
        edges.push(MermaidEdge {
            from: pid,
            to: my_id,
            label: rule,
        });
    }

    let rule = node.rule.clone().unwrap_or_default();
    for child in &node.children {
        collect_mermaid(child, nodes, edges, counter, Some((my_id, rule.clone())));
    }
}

// ── Explain renderer ──────────────────────────────────────────────────────

/// Format a human-readable explanation of why a route was ranked as it is.
/// Derives strengths and weaknesses purely from existing Route fields — no
/// new computation.
pub fn explain_route(route: &Route, target: &str, num: usize) -> String {
    let mut out = format!(
        "Route {}  [score={:.2}, depth={}]\nTarget: {target}\n\n",
        num, route.score, route.depth
    );

    let mut strengths: Vec<String> = Vec::new();
    let mut weaknesses: Vec<String> = Vec::new();

    if route.depth == 1 {
        strengths.push("single-step synthesis".into());
    } else if route.depth >= 4 {
        weaknesses.push(format!(
            "long route ({} steps) — more steps increase failure risk",
            route.depth
        ));
    }
    if route.confidence >= 0.8 {
        strengths.push(format!(
            "high template frequency (confidence {:.2})",
            route.confidence
        ));
    } else if route.confidence < 0.4 {
        weaknesses.push(format!(
            "rare template used (confidence {:.2})",
            route.confidence
        ));
    }
    if route.success_probability >= 0.7 {
        strengths.push(format!(
            "high step-success probability ({:.2})",
            route.success_probability
        ));
    } else if route.success_probability < 0.5 {
        weaknesses.push(format!(
            "success_probability {:.2} — cascaded template uncertainty",
            route.success_probability
        ));
    }
    if route.convergency >= 0.8 && route.depth > 1 {
        strengths.push("parallel synthesis possible".into());
    }
    if route.steps.iter().any(|s| s.procedure_hint.is_some()) {
        strengths.push("procedure hints available".into());
    }
    let bad_ae: Vec<(usize, f64)> = route
        .steps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.atom_economy.filter(|&ae| ae < 50.0).map(|ae| (i + 1, ae)))
        .collect();
    if bad_ae.is_empty()
        && route
            .steps
            .iter()
            .all(|s| s.atom_economy.is_some_and(|ae| ae >= 70.0))
    {
        strengths.push("good atom economy across all steps".into());
    }
    for (i, ae) in &bad_ae {
        weaknesses.push(format!("low atom economy in step {i} ({ae:.0}%)"));
    }
    let mut families: Vec<&str> = Vec::new();
    for step in &route.steps {
        if let Some(f) = step.reaction_family.as_deref()
            && !families.contains(&f)
        {
            families.push(f);
        }
    }
    if !families.is_empty() {
        strengths.push(format!("named reaction: {}", families.join(", ")));
    }

    if !strengths.is_empty() {
        out.push_str("Strengths:\n");
        for s in &strengths {
            out.push_str(&format!("  - {s}\n"));
        }
    }
    if !weaknesses.is_empty() {
        out.push_str("Weaknesses:\n");
        for w in &weaknesses {
            out.push_str(&format!("  - {w}\n"));
        }
    }

    out.push_str("\nSteps:\n");
    for (i, step) in route.steps.iter().enumerate() {
        let label = step.reaction_family.as_deref().unwrap_or(&step.rule);
        let ae = step
            .atom_economy
            .map(|a| format!(", atom_economy {a:.0}%"))
            .unwrap_or_default();
        out.push_str(&format!(
            "  Step {}: {} (confidence {:.2}{})\n",
            i + 1,
            label,
            step.step_confidence,
            ae
        ));
        if let Some(hint) = &step.procedure_hint {
            out.push_str(&format!("    Procedure: {hint}\n"));
        }
    }
    out.push('\n');
    out
}

// ── Comparison table renderer ──────────────────────────────────────────────

/// Format a comparison table of multiple routes (one row per route).
pub fn format_route_table(routes: &[Route]) -> String {
    let mut out = format!(
        "{:<6} {:<6} {:<6} {:<7} {:<7} {:<7} {:<6} {}\n",
        "Route", "Steps", "Depth", "Conf", "SuccP", "Cost", "Conv", "Family"
    );
    out.push_str(&"-".repeat(62));
    out.push('\n');
    for (i, route) in routes.iter().enumerate() {
        let mut families: Vec<&str> = Vec::new();
        for step in &route.steps {
            if let Some(f) = step.reaction_family.as_deref()
                && !families.contains(&f)
            {
                families.push(f);
            }
        }
        let family = if families.is_empty() {
            "—".to_string()
        } else {
            families.join(", ")
        };
        out.push_str(&format!(
            "{:<6} {:<6} {:<6} {:<7.2} {:<7.2} {:<7.2} {:<6.2} {}\n",
            i + 1,
            route.steps.len(),
            route.depth,
            route.confidence,
            route.success_probability,
            route.route_cost,
            route.convergency,
            family,
        ));
    }
    out
}

/// Format a route as a Mermaid flowchart (LR direction).
///
/// Example:
/// ```text
/// graph LR
///   n0["c1ccc(-c2ccccc2)cc1"] -->|suzuki_retro| n1["c1ccccc1Br ✓"]
///   n0 -->|suzuki_retro| n2["c1ccccc1 ✓"]
/// ```
pub fn format_route_mermaid(route: &Route, target: &str, route_num: usize) -> String {
    let root = find_root(&route.steps, target);
    let tree = build_tree(&route.steps, root);

    let mut nodes: Vec<MermaidNode> = Vec::new();
    let mut edges: Vec<MermaidEdge> = Vec::new();
    let mut counter = 0usize;
    collect_mermaid(&tree, &mut nodes, &mut edges, &mut counter, None);

    let mut out = format!(
        "graph LR\n  %% Route {}  score={:.2}  depth={}\n",
        route_num, route.score, route.depth
    );
    for n in &nodes {
        out.push_str(&format!("  n{}[\"{}\"]\n", n.id, n.label));
    }
    for e in &edges {
        out.push_str(&format!("  n{} -->|{}| n{}\n", e.from, e.label, e.to));
    }
    out
}
