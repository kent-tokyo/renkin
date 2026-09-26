//! Route-to-route distance and clustering (AiZynthFinder
//! `route_distances` / `RouteCollection.cluster` parity).
//!
//! A route is turned into an AiZynthFinder-style bipartite tree: molecule
//! nodes (canonical SMILES) alternate with reaction nodes (template ID).
//! Children are put in a canonical order (by a recursive subtree signature)
//! so two routes that differ only in precursor listing order are identical.
//! The distance is the Zhang–Shasha ordered tree edit distance over that
//! canonical ordering with unit costs: insert/delete = 1, relabel = 0 for an
//! identical label and 1 for a different label of the same node kind; a
//! molecule node is never relabelled into a reaction node.
//!
//! Differences from AiZynthFinder, stated so results are not over-read:
//! AiZynthFinder weights molecule relabels by fingerprint Tanimoto distance
//! and can search child permutations; this module uses exact-label unit
//! costs over one canonical ordering, which is deterministic, dependency
//! free, and an upper bound on the unordered unit-cost distance. Distances
//! are structural route differences, not chemical-similarity scores.
//!
//! Clustering is average-linkage agglomerative clustering on the distance
//! matrix. With no fixed cluster count, `k` is chosen in
//! `2..=min(max_clusters, n - 1)` by the highest mean silhouette (ties →
//! fewer clusters). Cluster labels are renumbered by first appearance in the
//! input route order, so the best-ranked route is always in cluster 0.

use crate::search::Route;
use serde::Serialize;
use std::collections::HashMap;

/// Output schema version for [`RouteClustering`].
pub const ROUTE_CLUSTERING_SCHEMA_VERSION: u32 = 1;

/// Distance method identifier recorded in the output.
pub const ROUTE_DISTANCE_METHOD: &str = "canonical_ordered_ted_unit_cost";

#[derive(Debug, Clone, PartialEq, Eq)]
enum NodeKind {
    Molecule,
    Reaction,
}

#[derive(Debug, Clone)]
struct Node {
    kind: NodeKind,
    label: String,
    children: Vec<Node>,
}

impl Node {
    /// Recursive canonical signature used only to order siblings.
    fn signature(&self) -> String {
        let tag = match self.kind {
            NodeKind::Molecule => 'M',
            NodeKind::Reaction => 'R',
        };
        if self.children.is_empty() {
            format!("{tag}:{}", self.label)
        } else {
            let inner: Vec<String> = self.children.iter().map(Node::signature).collect();
            format!("{tag}:{}({})", self.label, inner.join(","))
        }
    }

    fn canonicalize(&mut self) {
        for child in &mut self.children {
            child.canonicalize();
        }
        self.children.sort_by_cached_key(Node::signature);
    }
}

fn root_smiles(route: &Route) -> Option<&str> {
    let first = route.steps.first()?;
    let precursors: std::collections::HashSet<&str> = route
        .steps
        .iter()
        .flat_map(|s| s.precursors.iter().map(String::as_str))
        .collect();
    Some(
        route
            .steps
            .iter()
            .map(|s| s.target.as_str())
            .find(|t| !precursors.contains(t))
            .unwrap_or(first.target.as_str()),
    )
}

fn build_tree(route: &Route, fallback_target: &str) -> Node {
    let by_target: HashMap<&str, (&str, &[String])> = route
        .steps
        .iter()
        .rev() // first step for a target wins, matching display::build_tree
        .map(|s| {
            (
                s.target.as_str(),
                (s.template_id.as_str(), s.precursors.as_slice()),
            )
        })
        .collect();

    fn molecule(
        smiles: &str,
        by_target: &HashMap<&str, (&str, &[String])>,
        on_path: &mut Vec<String>,
    ) -> Node {
        let mut node = Node {
            kind: NodeKind::Molecule,
            label: smiles.to_owned(),
            children: Vec::new(),
        };
        // `on_path` guards against a malformed cyclic step list.
        if let Some(&(template_id, precursors)) = by_target.get(smiles)
            && !on_path.iter().any(|s| s == smiles)
        {
            on_path.push(smiles.to_owned());
            node.children.push(Node {
                kind: NodeKind::Reaction,
                label: template_id.to_owned(),
                children: precursors
                    .iter()
                    .map(|p| molecule(p, by_target, on_path))
                    .collect(),
            });
            on_path.pop();
        }
        node
    }

    let root = root_smiles(route).unwrap_or(fallback_target);
    let mut tree = molecule(root, &by_target, &mut Vec::new());
    tree.canonicalize();
    tree
}

/// Post-order flattened tree for Zhang–Shasha.
struct Flat<'a> {
    nodes: Vec<&'a Node>,
    /// Leftmost leaf descendant (post-order index) of each node.
    lld: Vec<usize>,
    keyroots: Vec<usize>,
}

impl<'a> Flat<'a> {
    fn new(root: &'a Node) -> Self {
        fn walk<'a>(node: &'a Node, nodes: &mut Vec<&'a Node>, lld: &mut Vec<usize>) -> usize {
            let mut leftmost = None;
            for child in &node.children {
                let child_lld = walk(child, nodes, lld);
                leftmost.get_or_insert(child_lld);
            }
            let index = nodes.len();
            nodes.push(node);
            let own = leftmost.unwrap_or(index);
            lld.push(own);
            own
        }
        let mut nodes = Vec::new();
        let mut lld = Vec::new();
        walk(root, &mut nodes, &mut lld);
        let mut keyroots = Vec::new();
        for i in 0..nodes.len() {
            // i is a keyroot iff no later node shares its leftmost leaf.
            if !(i + 1..nodes.len()).any(|j| lld[j] == lld[i]) {
                keyroots.push(i);
            }
        }
        Self {
            nodes,
            lld,
            keyroots,
        }
    }
}

fn relabel_cost(a: &Node, b: &Node) -> f64 {
    if a.kind != b.kind {
        2.0 // never cheaper than delete + insert
    } else if a.label == b.label {
        0.0
    } else {
        1.0
    }
}

/// Zhang–Shasha ordered tree edit distance with unit insert/delete costs.
fn tree_edit_distance(a: &Node, b: &Node) -> f64 {
    let fa = Flat::new(a);
    let fb = Flat::new(b);
    let (n, m) = (fa.nodes.len(), fb.nodes.len());
    let mut td = vec![vec![0.0_f64; m]; n];
    for &i in &fa.keyroots {
        for &j in &fb.keyroots {
            let (li, lj) = (fa.lld[i], fb.lld[j]);
            let rows = i - li + 2;
            let cols = j - lj + 2;
            let mut fd = vec![vec![0.0_f64; cols]; rows];
            for x in 1..rows {
                fd[x][0] = fd[x - 1][0] + 1.0;
            }
            for y in 1..cols {
                fd[0][y] = fd[0][y - 1] + 1.0;
            }
            for x in 1..rows {
                for y in 1..cols {
                    let (ni, nj) = (li + x - 1, lj + y - 1);
                    let delete = fd[x - 1][y] + 1.0;
                    let insert = fd[x][y - 1] + 1.0;
                    if fa.lld[ni] == li && fb.lld[nj] == lj {
                        let relabel = fd[x - 1][y - 1] + relabel_cost(fa.nodes[ni], fb.nodes[nj]);
                        fd[x][y] = delete.min(insert).min(relabel);
                        td[ni][nj] = fd[x][y];
                    } else {
                        let px = fa.lld[ni] - li;
                        let py = fb.lld[nj] - lj;
                        fd[x][y] = delete.min(insert).min(fd[px][py] + td[ni][nj]);
                    }
                }
            }
        }
    }
    td[n - 1][m - 1]
}

/// Pairwise route distance matrix (symmetric, zero diagonal).
///
/// `target` is used only for depth-0 routes, which have no steps to derive
/// the root from.
pub fn route_distance_matrix(routes: &[Route], target: &str) -> Vec<Vec<f64>> {
    let trees: Vec<Node> = routes.iter().map(|r| build_tree(r, target)).collect();
    let n = trees.len();
    let mut matrix = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let d = tree_edit_distance(&trees[i], &trees[j]);
            matrix[i][j] = d;
            matrix[j][i] = d;
        }
    }
    matrix
}

/// Average-linkage agglomerative clustering into exactly `k` clusters.
/// Labels are renumbered by first appearance in input order.
fn average_linkage(distances: &[Vec<f64>], k: usize) -> Vec<usize> {
    let n = distances.len();
    let mut clusters: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let k = k.clamp(1, n.max(1));
    while clusters.len() > k {
        let mut best: Option<(f64, usize, usize)> = None;
        for a in 0..clusters.len() {
            for b in a + 1..clusters.len() {
                let mut sum = 0.0;
                for &i in &clusters[a] {
                    for &j in &clusters[b] {
                        sum += distances[i][j];
                    }
                }
                let mean = sum / (clusters[a].len() * clusters[b].len()) as f64;
                // Strict `<` keeps the first (lowest-index) pair on ties.
                if best.is_none_or(|(d, _, _)| mean < d) {
                    best = Some((mean, a, b));
                }
            }
        }
        let (_, a, b) = best.expect("more than k >= 1 clusters implies a pair");
        let merged = clusters.remove(b);
        clusters[a].extend(merged);
    }
    let mut raw = vec![0usize; n];
    for (label, members) in clusters.iter().enumerate() {
        for &i in members {
            raw[i] = label;
        }
    }
    renumber_by_first_appearance(&raw)
}

fn renumber_by_first_appearance(raw: &[usize]) -> Vec<usize> {
    let mut mapping: HashMap<usize, usize> = HashMap::new();
    raw.iter()
        .map(|label| {
            let next = mapping.len();
            *mapping.entry(*label).or_insert(next)
        })
        .collect()
}

/// Mean silhouette coefficient; singletons contribute 0.
fn mean_silhouette(distances: &[Vec<f64>], labels: &[usize]) -> f64 {
    let n = labels.len();
    if n == 0 {
        return 0.0;
    }
    let k = labels.iter().copied().max().map_or(0, |m| m + 1);
    let mut total = 0.0;
    for i in 0..n {
        let mut sums = vec![0.0; k];
        let mut counts = vec![0usize; k];
        for j in 0..n {
            if i != j {
                sums[labels[j]] += distances[i][j];
                counts[labels[j]] += 1;
            }
        }
        let own = labels[i];
        if counts[own] == 0 {
            continue;
        }
        let a = sums[own] / counts[own] as f64;
        let b = (0..k)
            .filter(|&c| c != own && counts[c] > 0)
            .map(|c| sums[c] / counts[c] as f64)
            .fold(f64::INFINITY, f64::min);
        if b.is_finite() {
            let denom = a.max(b);
            if denom > 0.0 {
                total += (b - a) / denom;
            }
        }
    }
    total / n as f64
}

/// Clustering result, serialized into search output.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RouteClustering {
    pub schema_version: u32,
    pub distance_method: &'static str,
    pub linkage: &'static str,
    /// `"fixed"`, `"silhouette"`, or `"too_few_routes"`.
    pub selection: &'static str,
    pub n_clusters: usize,
    /// Mean silhouette of the chosen labelling; `None` for fewer than two
    /// clusters, where it is undefined.
    pub silhouette: Option<f64>,
    /// Cluster label per route, in the same order as the routes.
    pub labels: Vec<usize>,
    pub distance_matrix: Vec<Vec<f64>>,
}

/// Cluster `routes` by structural distance.
///
/// `n_clusters = Some(k)` fixes the count (clamped to the route count);
/// `None` selects `k` by silhouette up to `max_clusters`.
pub fn cluster_routes(
    routes: &[Route],
    target: &str,
    n_clusters: Option<usize>,
    max_clusters: usize,
) -> RouteClustering {
    let distance_matrix = route_distance_matrix(routes, target);
    let n = routes.len();
    let (selection, labels) = match n_clusters {
        Some(k) => ("fixed", average_linkage(&distance_matrix, k)),
        None if n < 3 => ("too_few_routes", vec![0; n]),
        None => {
            let upper = max_clusters.min(n - 1).max(2);
            let mut best: Option<(f64, Vec<usize>)> = None;
            for k in 2..=upper {
                let labels = average_linkage(&distance_matrix, k);
                let score = mean_silhouette(&distance_matrix, &labels);
                if best.as_ref().is_none_or(|(s, _)| score > *s + 1e-12) {
                    best = Some((score, labels));
                }
            }
            ("silhouette", best.map(|(_, l)| l).unwrap_or_default())
        }
    };
    let n_clusters = labels.iter().copied().max().map_or(0, |m| m + 1);
    let silhouette = (n_clusters >= 2).then(|| mean_silhouette(&distance_matrix, &labels));
    RouteClustering {
        schema_version: ROUTE_CLUSTERING_SCHEMA_VERSION,
        distance_method: ROUTE_DISTANCE_METHOD,
        linkage: "average",
        selection,
        n_clusters,
        silhouette,
        labels,
        distance_matrix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{AtomEconomyStatus, ReactionStep};

    fn step(target: &str, template: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: template.to_owned(),
            template_id: format!("rule:{template}"),
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

    fn route(steps: Vec<ReactionStep>) -> Route {
        Route {
            depth: steps.len() as u32,
            steps,
            score: 0.0,
            building_blocks: Vec::new(),
            confidence: 0.0,
            convergency: 0.0,
            success_probability: 0.0,
            route_cost: 0.0,
        }
    }

    #[test]
    fn identical_routes_and_reordered_precursors_have_zero_distance() {
        let a = route(vec![step("T", "ester", &["A", "B"])]);
        let b = route(vec![step("T", "ester", &["B", "A"])]);
        let m = route_distance_matrix(&[a.clone(), a, b], "T");
        assert_eq!(m[0][1], 0.0);
        assert_eq!(m[0][2], 0.0);
    }

    #[test]
    fn distance_counts_unit_edits() {
        let base = route(vec![step("T", "ester", &["A", "B"])]);
        // Same disconnection, one different precursor: one molecule relabel.
        let one_precursor = route(vec![step("T", "ester", &["A", "C"])]);
        // Different template, same precursors: one reaction relabel.
        let other_rule = route(vec![step("T", "amide", &["A", "B"])]);
        // Extra step below B: +1 reaction node +1 molecule node.
        let deeper = route(vec![
            step("T", "ester", &["A", "B"]),
            step("B", "suzuki", &["D"]),
        ]);
        let m = route_distance_matrix(&[base, one_precursor, other_rule, deeper], "T");
        assert_eq!(m[0][1], 1.0);
        assert_eq!(m[0][2], 1.0);
        assert_eq!(m[0][3], 2.0);
        for (i, row) in m.iter().enumerate() {
            assert_eq!(row[i], 0.0);
            for (j, value) in row.iter().enumerate() {
                assert_eq!(*value, m[j][i], "matrix must be symmetric");
            }
        }
    }

    #[test]
    fn depth_zero_route_is_a_single_molecule() {
        let buy = route(Vec::new());
        let one_step = route(vec![step("T", "ester", &["A", "B"])]);
        let m = route_distance_matrix(&[buy, one_step], "T");
        // Insert one reaction node and two molecule nodes.
        assert_eq!(m[0][1], 3.0);
    }

    #[test]
    fn silhouette_clustering_separates_two_families() {
        let routes = vec![
            route(vec![step("T", "ester", &["A", "B"])]),
            route(vec![step("T", "amide", &["X", "Y"])]),
            route(vec![step("T", "ester", &["A", "C"])]),
            route(vec![step("T", "amide", &["X", "Z"])]),
        ];
        let clustering = cluster_routes(&routes, "T", None, 5);
        assert_eq!(clustering.selection, "silhouette");
        assert_eq!(clustering.n_clusters, 2);
        assert_eq!(clustering.labels, vec![0, 1, 0, 1]);
        assert!(clustering.silhouette.unwrap() > 0.0);
    }

    #[test]
    fn fixed_and_degenerate_counts() {
        let routes = vec![
            route(vec![step("T", "ester", &["A", "B"])]),
            route(vec![step("T", "amide", &["X", "Y"])]),
        ];
        let few = cluster_routes(&routes, "T", None, 5);
        assert_eq!(few.selection, "too_few_routes");
        assert_eq!(few.labels, vec![0, 0]);
        assert_eq!(few.silhouette, None);

        let fixed = cluster_routes(&routes, "T", Some(2), 5);
        assert_eq!(fixed.selection, "fixed");
        assert_eq!(fixed.labels, vec![0, 1]);

        let clamped = cluster_routes(&routes, "T", Some(10), 5);
        assert_eq!(clamped.n_clusters, 2);

        let empty = cluster_routes(&[], "T", None, 5);
        assert_eq!(empty.n_clusters, 0);
        assert!(empty.labels.is_empty());
    }
}
