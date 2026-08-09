use std::collections::HashMap;

use crate::evidence::{
    ConditionCandidate, EvidenceReference, ExampleMatch, FloatRange, ReferenceKind, ReportedYield,
    WarningSeverity, YieldBasis, YieldPercentage,
};
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

// ── Evidence formatting helpers ────────────────────────────────────────────

fn trim_float(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

/// Formats an inclusive range as `"75–85°C"`, or a bare value when `min == max`.
fn format_range(r: &FloatRange, unit: &str) -> String {
    if (r.min - r.max).abs() < f64::EPSILON {
        format!("{}{unit}", trim_float(r.min))
    } else {
        format!("{}–{}{unit}", trim_float(r.min), trim_float(r.max))
    }
}

/// Single-line, chemist-readable summary of a curated condition set.
fn format_condition_candidate(c: &ConditionCandidate) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(c.catalysts.iter().cloned());
    parts.extend(c.reagents.iter().cloned());
    parts.extend(c.bases.iter().cloned());
    if !c.solvents.is_empty() {
        parts.push(c.solvents.join("/"));
    }
    if let Some(r) = &c.temperature_c {
        parts.push(format_range(r, "°C"));
    }
    if let Some(r) = &c.time_hours {
        parts.push(format_range(r, " h"));
    }
    if let Some(a) = &c.atmosphere {
        parts.push(format!("{a} atmosphere"));
    }
    if let Some(n) = &c.notes {
        parts.push(n.clone());
    }
    if parts.is_empty() {
        "no details recorded".to_string()
    } else {
        parts.join(", ")
    }
}

fn format_yield_basis(b: YieldBasis) -> &'static str {
    match b {
        YieldBasis::Isolated => "isolated",
        YieldBasis::Assay => "assay",
        YieldBasis::Conversion => "conversion",
        YieldBasis::Unknown => "unknown basis",
    }
}

/// Formats a reported yield as `"78% isolated"` or `"72–81% isolated"`.
fn format_reported_yield(y: &ReportedYield) -> String {
    let pct = match &y.percentage {
        YieldPercentage::Single(p) => format!("{}%", trim_float(*p)),
        YieldPercentage::Range(r) => format!("{}–{}%", trim_float(r.min), trim_float(r.max)),
    };
    format!("{pct} {}", format_yield_basis(y.basis))
}

fn format_reference_kind(k: ReferenceKind) -> &'static str {
    match k {
        ReferenceKind::Doi => "DOI",
        ReferenceKind::Patent => "Patent",
        ReferenceKind::Url => "URL",
        ReferenceKind::DatasetRecord => "Dataset record",
    }
}

fn format_reference(r: &EvidenceReference) -> String {
    format!("{} {}", format_reference_kind(r.kind), r.identifier)
}

fn format_warning_severity(s: WarningSeverity) -> &'static str {
    match s {
        WarningSeverity::Info => "info",
        WarningSeverity::Low => "low",
        WarningSeverity::Medium => "medium",
        WarningSeverity::High => "high",
    }
}

/// Resolves `ids` against `all_refs` (a step's full `evidence.references`
/// list), silently skipping any id with no matching reference -- should not
/// occur after sidecar validation, but this renderer must never panic on it.
fn resolve_references<'a>(
    ids: &[String],
    all_refs: &'a [EvidenceReference],
) -> Vec<&'a EvidenceReference> {
    ids.iter()
        .filter_map(|id| all_refs.iter().find(|r| &r.id == id))
        .collect()
}

/// Renders one step's curated evidence: rule-author default conditions (if
/// any), curated examples (already resolved and capped by
/// `TemplateMetadataEntry::to_step_evidence` -- every exact-substrate match
/// plus up to 3 same-template-different-substrate precedents), and any
/// template-level warnings. Appends directly to `out`.
fn render_step_evidence(out: &mut String, step: &ReactionStep) {
    if let Some(cond) = &step.conditions {
        out.push_str("    Rule-author default conditions (not literature-derived):\n");
        if let Some(c) = &cond.catalyst {
            out.push_str(&format!("      Catalyst/reagent: {c}\n"));
        }
        if let Some(s) = &cond.solvent {
            out.push_str(&format!("      Solvent: {s}\n"));
        }
        if let Some(t) = &cond.temperature {
            out.push_str(&format!("      Temperature: {t}\n"));
        }
    }

    let Some(evidence) = &step.evidence else {
        return;
    };

    if !evidence.examples.is_empty() {
        out.push_str("    Evidence:\n");
        // Already resolved (exact matches first, template-only capped at 3) by
        // TemplateMetadataEntry::to_step_evidence -- no re-canonicalizing here.
        for resolved in &evidence.examples {
            let ex = &resolved.example;
            let header = match resolved.match_kind {
                ExampleMatch::ExactSubstrate => "Exact substrate example:",
                ExampleMatch::TemplateOnly => {
                    "Template-level literature example (different substrate; not a prediction):"
                }
            };
            out.push_str(&format!("      {header}\n"));
            // Dedups references shown more than once among conditions/yield/the
            // example's own reference_ids -- all rendered under "Reference:".
            // Warnings get their own dedup set: a reference explaining *why
            // there's a warning* is a distinct citation from one backing the
            // conditions/yield data, even if it's the same id, so it still
            // renders its own "Source:" line.
            let mut shown_content_refs: std::collections::HashSet<&str> =
                std::collections::HashSet::new();
            if let Some(c) = &ex.conditions {
                out.push_str(&format!(
                    "        Conditions: {}\n",
                    format_condition_candidate(c)
                ));
                for r in resolve_references(&c.reference_ids, &evidence.references) {
                    if shown_content_refs.insert(r.id.as_str()) {
                        out.push_str(&format!("          Reference: {}\n", format_reference(r)));
                    }
                }
            }
            if let Some(y) = &ex.reported_yield {
                out.push_str(&format!(
                    "        Reported yield: {}\n",
                    format_reported_yield(y)
                ));
                for r in resolve_references(&y.reference_ids, &evidence.references) {
                    if shown_content_refs.insert(r.id.as_str()) {
                        out.push_str(&format!("          Reference: {}\n", format_reference(r)));
                    }
                }
            }
            for r in resolve_references(&ex.reference_ids, &evidence.references) {
                if shown_content_refs.insert(r.id.as_str()) {
                    out.push_str(&format!("        Reference: {}\n", format_reference(r)));
                }
            }
            if let Some(drid) = &ex.dataset_record_id {
                out.push_str(&format!("        Dataset record: {drid}\n"));
            }
            let mut shown_warning_refs: std::collections::HashSet<&str> =
                std::collections::HashSet::new();
            for w in &ex.warnings {
                out.push_str(&format!(
                    "        Warning: [{}] {}\n",
                    format_warning_severity(w.severity),
                    w.message
                ));
                for r in resolve_references(&w.reference_ids, &evidence.references) {
                    if shown_warning_refs.insert(r.id.as_str()) {
                        out.push_str(&format!("          Source: {}\n", format_reference(r)));
                    }
                }
            }
        }
        let remaining = evidence
            .template_examples_total
            .saturating_sub(evidence.examples.len());
        if remaining > 0 {
            out.push_str(&format!(
                "      ... and {remaining} more template examples\n"
            ));
        }
    }

    if !evidence.warnings.is_empty() {
        out.push_str("    Warnings:\n");
        for w in &evidence.warnings {
            out.push_str(&format!(
                "      [{}] {}\n",
                format_warning_severity(w.severity),
                w.message
            ));
            for r in resolve_references(&w.reference_ids, &evidence.references) {
                out.push_str(&format!("        Source: {}\n", format_reference(r)));
            }
        }
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
            "high template-frequency route score ({:.2}) — not a calibrated experimental success probability",
            route.success_probability
        ));
    } else if route.success_probability < 0.5 {
        weaknesses.push(format!(
            "low frequency-derived route ranking score ({:.2}) — cascaded template rarity, not a calibrated experimental success probability",
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
    // Issue #79: surface an above-expected-range ratio explicitly rather
    // than letting a step silently drop out of both the "good economy" and
    // "low economy" categories above (which is all a clamped-to-100 value
    // used to do). This ratio alone is not proof of target-atom loss -- the
    // denominator is only the precursors a template names, not every
    // reactant/reagent a real reaction would use, so an omitted reagent can
    // push it over 100% for a perfectly valid route. The message stays
    // neutral about the cause and points at the independent
    // element-accounting diagnostic rather than asserting loss.
    for (i, s) in route.steps.iter().enumerate() {
        if s.atom_economy_status == crate::search::AtomEconomyStatus::AboveExpectedRange
            && let Some(raw) = s.atom_economy_raw_percent
        {
            weaknesses.push(format!(
                "step {} atom economy exceeds the represented precursor mass ({raw:.0}%); possible causes include omitted reactants/reagents, an incomplete template outcome, or target-atom loss -- check the element-accounting diagnostic",
                i + 1
            ));
        }
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
        render_step_evidence(&mut out, step);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{
        EvidenceScope, MetadataSource, ReactionExample, ReactionWarning, ResolvedReactionExample,
        StepEvidence, WarningSeverity,
    };
    use crate::search::ReactionConditions;

    fn base_step() -> ReactionStep {
        ReactionStep {
            rule: "ester_cleavage".to_string(),
            template_id: "rule:ester_cleavage".to_string(),
            target: "CC(=O)Oc1ccccc1C(=O)O".to_string(),
            precursors: vec!["CC(=O)O".to_string(), "Oc1ccccc1C(=O)O".to_string()],
            conditions: None,
            atom_economy: Some(90.0),
            atom_economy_raw_percent: Some(90.0),
            atom_economy_status: crate::search::AtomEconomyStatus::Normal,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: Some("esterification".to_string()),
            metadata_source: Some(MetadataSource::HandcraftedDefault),
            metadata_scope: Some(EvidenceScope::ReactionFamily),
            evidence: None,
        }
    }

    fn base_route(steps: Vec<ReactionStep>, success_probability: f64) -> Route {
        Route {
            steps,
            depth: 1,
            score: 1.0,
            building_blocks: vec![],
            confidence: 0.9,
            convergency: 0.0,
            success_probability,
            route_cost: 1.0,
        }
    }

    fn sample_reference() -> EvidenceReference {
        EvidenceReference {
            id: "ref-1".to_string(),
            kind: ReferenceKind::Doi,
            identifier: "10.xxxx/example".to_string(),
            title: None,
        }
    }

    fn sample_example(target: &str) -> ReactionExample {
        ReactionExample {
            id: "ex-1".to_string(),
            target_smiles: target.to_string(),
            precursor_smiles: vec!["CC(=O)O".to_string(), "Oc1ccccc1C(=O)O".to_string()],
            conditions: Some(ConditionCandidate {
                catalysts: vec![],
                reagents: vec!["H2SO4".to_string()],
                bases: vec![],
                solvents: vec!["toluene".to_string()],
                temperature_c: None,
                time_hours: None,
                atmosphere: None,
                notes: None,
                source: MetadataSource::Literature,
                scope: EvidenceScope::SubstrateSpecific,
                reference_ids: vec!["ref-1".to_string()],
            }),
            reported_yield: Some(ReportedYield {
                percentage: YieldPercentage::Single(78.0),
                basis: YieldBasis::Isolated,
                source: MetadataSource::Literature,
                scope: EvidenceScope::SubstrateSpecific,
                reference_ids: vec!["ref-1".to_string()],
            }),
            warnings: vec![],
            reference_ids: vec!["ref-1".to_string()],
            dataset_record_id: Some("ds-1".to_string()),
            notes: None,
        }
    }

    fn resolved(match_kind: ExampleMatch, example: ReactionExample) -> ResolvedReactionExample {
        ResolvedReactionExample {
            match_kind,
            example,
        }
    }

    // Mirrors what `TemplateMetadataEntry::to_step_evidence` actually hands a
    // step: already-resolved/capped `examples` plus `template_examples_total`.
    // Display-layer tests build this directly rather than a raw sidecar, since
    // resolution/capping/ordering are evidence.rs's responsibility now.
    fn step_evidence(
        references: Vec<EvidenceReference>,
        warnings: Vec<ReactionWarning>,
        examples: Vec<ResolvedReactionExample>,
        template_examples_total: usize,
    ) -> StepEvidence {
        StepEvidence {
            condition_candidates: vec![],
            reported_yields: vec![],
            references,
            warnings,
            examples,
            template_examples_total,
        }
    }

    #[test]
    fn generic_conditions_labeled_not_literature_derived() {
        let mut step = base_step();
        step.conditions = Some(ReactionConditions {
            catalyst: Some("H2SO4".to_string()),
            solvent: Some("toluene".to_string()),
            temperature: Some("reflux".to_string()),
            notes: None,
        });
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(out.contains("Rule-author default conditions (not literature-derived):"));
        assert!(out.contains("Catalyst/reagent: H2SO4"));
        assert!(out.contains("Solvent: toluene"));
        assert!(out.contains("Temperature: reflux"));
    }

    #[test]
    fn exact_substrate_example_renders_conditions_yield_reference_and_dataset_record() {
        let mut step = base_step();
        step.evidence = Some(step_evidence(
            vec![sample_reference()],
            vec![],
            vec![resolved(
                ExampleMatch::ExactSubstrate,
                sample_example("CC(=O)Oc1ccccc1C(=O)O"),
            )],
            1,
        ));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(out.contains("Evidence:"));
        assert!(out.contains("Exact substrate example:"));
        assert!(out.contains("Reported yield: 78% isolated"));
        assert!(out.contains("Reference: DOI 10.xxxx/example"));
        assert!(out.contains("Dataset record: ds-1"));
    }

    #[test]
    fn condition_specific_reference_is_shown_even_when_example_level_ids_empty() {
        let mut step = base_step();
        let example = ReactionExample {
            id: "ex-1".to_string(),
            target_smiles: "CC(=O)Oc1ccccc1C(=O)O".to_string(),
            precursor_smiles: vec!["CC(=O)O".to_string(), "Oc1ccccc1C(=O)O".to_string()],
            conditions: Some(ConditionCandidate {
                catalysts: vec![],
                reagents: vec!["H2SO4".to_string()],
                bases: vec![],
                solvents: vec![],
                temperature_c: None,
                time_hours: None,
                atmosphere: None,
                notes: None,
                source: MetadataSource::Literature,
                scope: EvidenceScope::SubstrateSpecific,
                reference_ids: vec!["cond-ref".to_string()],
            }),
            reported_yield: None,
            warnings: vec![],
            reference_ids: vec![], // deliberately empty -- must not suppress the condition's own reference
            dataset_record_id: None,
            notes: None,
        };
        let references = vec![EvidenceReference {
            id: "cond-ref".to_string(),
            kind: ReferenceKind::Doi,
            identifier: "10.aaa/cond".to_string(),
            title: None,
        }];
        step.evidence = Some(step_evidence(
            references,
            vec![],
            vec![resolved(ExampleMatch::ExactSubstrate, example)],
            1,
        ));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(out.contains("Reference: DOI 10.aaa/cond"));
    }

    #[test]
    fn yield_specific_reference_is_shown_even_when_example_level_ids_empty() {
        let mut step = base_step();
        let example = ReactionExample {
            id: "ex-1".to_string(),
            target_smiles: "CC(=O)Oc1ccccc1C(=O)O".to_string(),
            precursor_smiles: vec!["CC(=O)O".to_string(), "Oc1ccccc1C(=O)O".to_string()],
            conditions: None,
            reported_yield: Some(ReportedYield {
                percentage: YieldPercentage::Single(78.0),
                basis: YieldBasis::Isolated,
                source: MetadataSource::Literature,
                scope: EvidenceScope::SubstrateSpecific,
                reference_ids: vec!["yield-ref".to_string()],
            }),
            warnings: vec![],
            reference_ids: vec![], // deliberately empty -- must not suppress the yield's own reference
            dataset_record_id: None,
            notes: None,
        };
        let references = vec![EvidenceReference {
            id: "yield-ref".to_string(),
            kind: ReferenceKind::Doi,
            identifier: "10.bbb/yield".to_string(),
            title: None,
        }];
        step.evidence = Some(step_evidence(
            references,
            vec![],
            vec![resolved(ExampleMatch::ExactSubstrate, example)],
            1,
        ));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(out.contains("Reference: DOI 10.bbb/yield"));
    }

    #[test]
    fn reference_shared_by_conditions_and_yield_is_shown_once() {
        let mut step = base_step();
        let shared_ref_ids = vec!["shared-ref".to_string()];
        let example = ReactionExample {
            id: "ex-1".to_string(),
            target_smiles: "CC(=O)Oc1ccccc1C(=O)O".to_string(),
            precursor_smiles: vec!["CC(=O)O".to_string(), "Oc1ccccc1C(=O)O".to_string()],
            conditions: Some(ConditionCandidate {
                catalysts: vec![],
                reagents: vec!["H2SO4".to_string()],
                bases: vec![],
                solvents: vec![],
                temperature_c: None,
                time_hours: None,
                atmosphere: None,
                notes: None,
                source: MetadataSource::Literature,
                scope: EvidenceScope::SubstrateSpecific,
                reference_ids: shared_ref_ids.clone(),
            }),
            reported_yield: Some(ReportedYield {
                percentage: YieldPercentage::Single(78.0),
                basis: YieldBasis::Isolated,
                source: MetadataSource::Literature,
                scope: EvidenceScope::SubstrateSpecific,
                reference_ids: shared_ref_ids.clone(),
            }),
            warnings: vec![],
            reference_ids: shared_ref_ids,
            dataset_record_id: None,
            notes: None,
        };
        let references = vec![EvidenceReference {
            id: "shared-ref".to_string(),
            kind: ReferenceKind::Doi,
            identifier: "10.ccc/shared".to_string(),
            title: None,
        }];
        step.evidence = Some(step_evidence(
            references,
            vec![],
            vec![resolved(ExampleMatch::ExactSubstrate, example)],
            1,
        ));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert_eq!(
            out.matches("Reference: DOI 10.ccc/shared").count(),
            1,
            "a reference cited by conditions, reported_yield, and the example's own \
             reference_ids must render only once, got: {out}"
        );
    }

    #[test]
    fn different_substrate_example_labeled_not_a_prediction() {
        let mut step = base_step();
        step.evidence = Some(step_evidence(
            vec![sample_reference()],
            vec![],
            vec![resolved(ExampleMatch::TemplateOnly, sample_example("CCO"))],
            1,
        ));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(out.contains(
            "Template-level literature example (different substrate; not a prediction):"
        ));
        assert!(!out.contains("Exact substrate example:"));
    }

    #[test]
    fn remainder_count_is_total_minus_shown() {
        // Resolution/capping is evidence.rs's job (see
        // to_step_evidence_keeps_all_exact_and_caps_template_only_at_three);
        // this only checks display.rs's "... and N more" arithmetic against
        // whatever `template_examples_total` it's handed.
        let mut step = base_step();
        let examples: Vec<ResolvedReactionExample> = (0..3)
            .map(|i| {
                let mut ex = sample_example("CCO");
                ex.id = format!("ex-{i}");
                resolved(ExampleMatch::TemplateOnly, ex)
            })
            .collect();
        step.evidence = Some(step_evidence(vec![], vec![], examples, 5));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(out.contains("... and 2 more template examples"));
    }

    #[test]
    fn exact_substrate_examples_render_before_template_only() {
        let mut step = base_step();
        let examples = vec![
            resolved(
                ExampleMatch::ExactSubstrate,
                sample_example("CC(=O)Oc1ccccc1C(=O)O"),
            ),
            resolved(ExampleMatch::TemplateOnly, sample_example("CCO")),
        ];
        step.evidence = Some(step_evidence(vec![], vec![], examples, 2));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        let exact_pos = out.find("Exact substrate example:").unwrap();
        let template_only_pos = out.find("Template-level literature example").unwrap();
        assert!(
            exact_pos < template_only_pos,
            "exact-substrate match must render before template-only matches, got: {out}"
        );
    }

    #[test]
    fn example_level_warning_is_rendered() {
        let mut step = base_step();
        let mut example = sample_example("CC(=O)Oc1ccccc1C(=O)O");
        example.warnings = vec![ReactionWarning {
            code: "substrate_specific_side_reaction".to_string(),
            severity: WarningSeverity::High,
            message: "Decomposition observed for this exact substrate above 100C.".to_string(),
            source: MetadataSource::Literature,
            scope: EvidenceScope::SubstrateSpecific,
            reference_ids: vec!["ref-1".to_string()],
        }];
        step.evidence = Some(step_evidence(
            vec![sample_reference()],
            vec![],
            vec![resolved(ExampleMatch::ExactSubstrate, example)],
            1,
        ));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(out.contains(
            "Warning: [high] Decomposition observed for this exact substrate above 100C."
        ));
        assert!(out.contains("Source: DOI 10.xxxx/example"));
    }

    #[test]
    fn warnings_rendered_with_severity_and_source() {
        let mut step = base_step();
        step.evidence = Some(step_evidence(
            vec![sample_reference()],
            vec![ReactionWarning {
                code: "possible_protodeboronation".to_string(),
                severity: WarningSeverity::Medium,
                message: "Possible protodeboronation under prolonged heating.".to_string(),
                source: MetadataSource::Literature,
                scope: EvidenceScope::Template,
                reference_ids: vec!["ref-1".to_string()],
            }],
            vec![],
            0,
        ));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(out.contains("Warnings:"));
        assert!(out.contains("[medium] Possible protodeboronation under prolonged heating."));
        assert!(out.contains("Source: DOI 10.xxxx/example"));
    }

    #[test]
    fn absent_warnings_are_not_rendered_as_no_side_reactions() {
        let mut step = base_step();
        step.evidence = Some(step_evidence(vec![], vec![], vec![], 0));
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(!out.contains("Warnings:"));
        assert!(!out.to_lowercase().contains("no side reaction"));
    }

    #[test]
    fn high_success_probability_wording_avoids_forbidden_phrase() {
        let route = base_route(vec![base_step()], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(!out.contains("step-success"));
        assert!(out.contains("template-frequency route score"));
        assert!(out.contains("not a calibrated experimental success probability"));
    }

    #[test]
    fn low_success_probability_wording_avoids_forbidden_phrase() {
        let route = base_route(vec![base_step()], 0.3);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(!out.contains("step-success"));
        assert!(!out.contains("probability that every step"));
        assert!(out.contains("frequency-derived route ranking score"));
    }

    // Issue #79 review round 2: the atom-economy ratio alone can't tell an
    // intentionally-omitted reagent apart from genuine target-atom loss, so
    // the weakness message must stay neutral and never assert loss on its
    // own -- it may only ever list it as one of several possible causes.
    #[test]
    fn above_expected_range_step_gets_a_neutral_weakness_message() {
        let mut step = base_step();
        step.atom_economy = None;
        step.atom_economy_raw_percent = Some(183.4);
        step.atom_economy_status = crate::search::AtomEconomyStatus::AboveExpectedRange;
        let route = base_route(vec![step], 0.9);
        let out = explain_route(&route, "CC(=O)Oc1ccccc1C(=O)O", 1);
        assert!(
            out.contains("check the element-accounting diagnostic"),
            "got: {out}"
        );
        assert!(
            out.contains("possible causes include omitted reactants/reagents"),
            "got: {out}"
        );
        assert!(
            !out.contains("target-atom loss suspected"),
            "must not assert loss as a standalone claim, got: {out}"
        );
        assert!(!out.contains("physical maximum"), "got: {out}");
    }
}
