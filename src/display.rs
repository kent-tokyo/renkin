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
