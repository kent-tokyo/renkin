use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write as _;

use anyhow::Result;
use renkin::search::Route;

/// Bipartite reaction knowledge graph: molecule nodes ↔ reaction nodes.
/// Built from retrosynthesis routes; exportable to GraphML or Cypher.
pub struct ReactionGraph {
    mol_index: HashMap<String, usize>,
    pub mols: Vec<MolNode>,
    pub rxns: Vec<RxnNode>,
}

pub struct MolNode {
    pub smiles: String,
    /// Reaction indices that produce this molecule (as product).
    pub produced_by: Vec<usize>,
    /// Reaction indices that consume this molecule (as precursor).
    pub consumed_by: Vec<usize>,
}

pub struct RxnNode {
    pub rule: String,
    pub product: String,
    pub precursors: Vec<String>,
    pub family: Option<String>,
    pub step_confidence: f64,
    pub route_score: f64,
}

impl ReactionGraph {
    pub fn from_routes(routes: &[Route]) -> Self {
        let mut g = ReactionGraph {
            mol_index: HashMap::new(),
            mols: Vec::new(),
            rxns: Vec::new(),
        };
        let mut rxn_index: HashMap<(String, String), usize> = HashMap::new();

        for route in routes {
            for step in &route.steps {
                let prod_idx = g.get_or_insert_mol(&step.target);

                let rxn_key = (step.target.clone(), step.rule.clone());
                let rxn_idx = *rxn_index.entry(rxn_key).or_insert_with(|| {
                    let idx = g.rxns.len();
                    g.rxns.push(RxnNode {
                        rule: step.rule.clone(),
                        product: step.target.clone(),
                        precursors: step.precursors.clone(),
                        family: step.reaction_family.clone(),
                        step_confidence: step.step_confidence,
                        route_score: route.score,
                    });
                    idx
                });

                if !g.mols[prod_idx].produced_by.contains(&rxn_idx) {
                    g.mols[prod_idx].produced_by.push(rxn_idx);
                }
                for prec in &step.precursors {
                    let prec_idx = g.get_or_insert_mol(prec);
                    if !g.mols[prec_idx].consumed_by.contains(&rxn_idx) {
                        g.mols[prec_idx].consumed_by.push(rxn_idx);
                    }
                }
            }
        }
        g
    }

    fn get_or_insert_mol(&mut self, smiles: &str) -> usize {
        if let Some(&idx) = self.mol_index.get(smiles) {
            return idx;
        }
        let idx = self.mols.len();
        self.mol_index.insert(smiles.to_string(), idx);
        self.mols.push(MolNode {
            smiles: smiles.to_string(),
            produced_by: Vec::new(),
            consumed_by: Vec::new(),
        });
        idx
    }

    /// Find all synthesis trees in the graph that connect `target` to `starting_materials`.
    /// Returns one `Vec<usize>` (sorted reaction indices) per valid synthesis tree.
    pub fn find_paths(&self, target: &str, starting_materials: &[&str]) -> Vec<Vec<usize>> {
        let sm_set: HashSet<&str> = starting_materials.iter().copied().collect();
        self.dfs(target, &sm_set, 0)
    }

    fn dfs(&self, smiles: &str, sm_set: &HashSet<&str>, depth: usize) -> Vec<Vec<usize>> {
        if sm_set.contains(smiles) {
            return vec![vec![]]; // leaf: empty rxn list is valid
        }
        if depth > 16 {
            return vec![]; // cycle guard (retrosynthesis is a DAG, but limit anyway)
        }
        let Some(&mol_idx) = self.mol_index.get(smiles) else {
            return vec![];
        };

        let mut result = Vec::new();
        'rxn: for &rxn_idx in &self.mols[mol_idx].produced_by {
            let rxn = &self.rxns[rxn_idx];
            // sub-paths for each precursor; all must be reachable
            let sub_paths: Vec<Vec<Vec<usize>>> = rxn
                .precursors
                .iter()
                .map(|p| self.dfs(p, sm_set, depth + 1))
                .collect();
            if sub_paths.iter().any(|sp| sp.is_empty()) {
                continue 'rxn;
            }
            // cartesian product across precursors, prepend rxn_idx
            let mut combos: Vec<Vec<usize>> = vec![vec![rxn_idx]];
            for sub in &sub_paths {
                let mut next = Vec::new();
                for partial in &combos {
                    for path in sub {
                        let mut combined = partial.clone();
                        combined.extend_from_slice(path);
                        next.push(combined);
                    }
                }
                combos = next;
            }
            for mut path in combos {
                path.sort_unstable();
                path.dedup();
                result.push(path);
            }
        }
        result
    }

    /// Write bipartite graph to GraphML (readable by Gephi, yEd, Cytoscape, etc.).
    pub fn export_graphml(&self, path: &str) -> Result<()> {
        let mut f = File::create(path)?;
        writeln!(f, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
        writeln!(
            f,
            r#"<graphml xmlns="http://graphml.graphdrawing.org/graphml">"#
        )?;
        writeln!(
            f,
            r#"  <key id="label"      for="node" attr.name="label"      attr.type="string"/>"#
        )?;
        writeln!(
            f,
            r#"  <key id="type"       for="node" attr.name="type"       attr.type="string"/>"#
        )?;
        writeln!(
            f,
            r#"  <key id="confidence" for="node" attr.name="confidence" attr.type="double"/>"#
        )?;
        writeln!(f, r#"  <graph id="G" edgedefault="directed">"#)?;

        for (i, mol) in self.mols.iter().enumerate() {
            writeln!(
                f,
                r#"    <node id="mol:{i}"><data key="label">{}</data><data key="type">molecule</data></node>"#,
                xml_escape(&mol.smiles)
            )?;
        }
        for (i, rxn) in self.rxns.iter().enumerate() {
            writeln!(
                f,
                r#"    <node id="rxn:{i}"><data key="label">{}</data><data key="type">reaction</data><data key="confidence">{:.4}</data></node>"#,
                xml_escape(&rxn.rule),
                rxn.step_confidence
            )?;
            for prec in &rxn.precursors {
                if let Some(&pi) = self.mol_index.get(prec) {
                    writeln!(f, r#"    <edge source="mol:{pi}" target="rxn:{i}"/>"#)?;
                }
            }
            if let Some(&pi) = self.mol_index.get(&rxn.product) {
                writeln!(f, r#"    <edge source="rxn:{i}" target="mol:{pi}"/>"#)?;
            }
        }

        writeln!(f, "  </graph>")?;
        writeln!(f, "</graphml>")?;
        Ok(())
    }

    /// Write Cypher statements for Neo4j import (use `cypher-shell` or neo4j-admin).
    pub fn export_cypher(&self, path: &str) -> Result<()> {
        let mut f = File::create(path)?;
        writeln!(
            f,
            "// Reaction Knowledge Graph — import with neo4j-admin or cypher-shell"
        )?;
        for (i, mol) in self.mols.iter().enumerate() {
            writeln!(
                f,
                r#"MERGE (m{i}:Molecule {{smiles: "{}", id: {i}}});"#,
                cypher_escape(&mol.smiles)
            )?;
        }
        for (i, rxn) in self.rxns.iter().enumerate() {
            writeln!(
                f,
                r#"MERGE (r{i}:Reaction {{rule: "{}", id: {i}, confidence: {:.4}}});"#,
                cypher_escape(&rxn.rule),
                rxn.step_confidence
            )?;
            for prec in &rxn.precursors {
                if let Some(&pi) = self.mol_index.get(prec) {
                    writeln!(f, "MERGE (m{pi})-[:PRECURSOR_OF]->(r{i});")?;
                }
            }
            if let Some(&pi) = self.mol_index.get(&rxn.product) {
                writeln!(f, "MERGE (r{i})-[:PRODUCES]->(m{pi});")?;
            }
        }
        Ok(())
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn cypher_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use renkin::search::{ReactionStep, Route};

    fn mock_route() -> Route {
        Route {
            steps: vec![ReactionStep {
                rule: "amide_retro".to_string(),
                target: "CC(=O)N".to_string(),
                precursors: vec!["CC(=O)O".to_string(), "N".to_string()],
                conditions: None,
                atom_economy: None,
                step_confidence: 0.8,
                procedure_hint: None,
                reaction_family: Some("amide_coupling".to_string()),
            }],
            depth: 1,
            score: 1.5,
            building_blocks: vec!["CC(=O)O".to_string(), "N".to_string()],
            confidence: 0.8,
            convergency: 1.0,
            success_probability: 0.8,
            route_cost: 2.0,
        }
    }

    #[test]
    fn build_and_query() {
        let kg = ReactionGraph::from_routes(&[mock_route()]);

        assert_eq!(kg.mols.len(), 3); // target + 2 precursors
        assert_eq!(kg.rxns.len(), 1);

        let paths = kg.find_paths("CC(=O)N", &["CC(=O)O", "N"]);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], vec![0]);

        // exports produce non-empty files
        let tmp = std::env::temp_dir();
        let gml = tmp.join("renkin_kg_test.graphml");
        let cyp = tmp.join("renkin_kg_test.cypher");
        kg.export_graphml(gml.to_str().unwrap()).unwrap();
        kg.export_cypher(cyp.to_str().unwrap()).unwrap();
        assert!(std::fs::metadata(&gml).unwrap().len() > 0);
        assert!(std::fs::metadata(&cyp).unwrap().len() > 0);
    }
}
