//! Rule-safety census (v0.36.0 Phase 1): a mechanical, static screen of
//! every hand-crafted `default_rules()` SMIRKS against the risk shape that
//! broke `aryl_amine_retro`/`buchwald_hartwig_retro` (issue #77) -- a
//! minimally-constrained LHS plus a bare single-atom RHS product fragment
//! lets chematic's substituent-carry-through BFS wander unchecked into a
//! ring elsewhere in the real target, producing a "solved" route that's
//! either missing a fragment or has a corrupted one.
//!
//! Screening only: produces a cited candidate list (Markdown + JSON), never
//! removes or fixes a rule itself. A rule only gets touched once a specific
//! target fixture reproduces the defect via direct `apply_retro` calls --
//! matching a risk shape here is a reason to build that fixture, not a
//! verdict.
//!
//! Four signals, each computed from the rule's own SMIRKS string, not
//! guessed or hand-classified:
//! - `lhs_mapped_atom_count`: fewer mapped atoms means less declared
//!   context walling off chematic's BFS carry-through.
//! - `lhs_has_ring_closure_digit`: whether the LHS pattern itself declares
//!   any ring closure (a rule whose own pattern already commits to ring
//!   context is a different, presumably lower-risk, shape).
//! - `rhs_product_fragment_count`: `.`-separated RHS fragments; 1 means no
//!   "second precursor" risk at all -- most graph-based rules and several
//!   SMIRKS-based ones (`alcohol_oxidation_retro`, `acyl_chloride_from_acid`)
//!   are single-product functional-group interconversions, out of scope by
//!   construction.
//! - `rhs_bare_single_atom_fragments`: which RHS fragments (if any) parse to
//!   exactly one atom with no further declared structure -- the exact shape
//!   `aryl_amine_retro`/`buchwald_hartwig_retro`'s broken side had.
//!
//! Graph-based rules (empty `smirks`) are reported but never flagged: every
//! one of the 8 already calls `is_bridge_bond` and requires the cut bond to
//! be outside any ring before applying (confirmed by reading each function
//! body this session) -- structurally immune to this defect class by
//! construction, not just untested.

use renkin::chem_env::{RetroRule, default_rules, mol_from_smiles};

struct RuleReport {
    name: String,
    smirks: String,
    graph_based: bool,
    lhs_mapped_atom_count: Option<usize>,
    lhs_has_ring_closure_digit: bool,
    rhs_product_fragment_count: Option<usize>,
    rhs_bare_single_atom_fragments: Vec<String>,
    flagged: bool,
    flag_reasons: Vec<String>,
}

/// True iff `pattern` contains a SMILES ring-closure digit -- i.e. a digit
/// not part of an atom-map annotation (`:N`). Atom maps are always written
/// `:` immediately followed by the digits; stripping every such run first
/// leaves only genuine ring-closure digits (which appear directly after an
/// atom token, no `:` involved) to detect.
fn has_ring_closure_digit(pattern: &str) -> bool {
    let mut stripped = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ':' {
            while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                chars.next();
            }
            continue;
        }
        stripped.push(c);
    }
    stripped.chars().any(|c| c.is_ascii_digit())
}

fn analyze(rule: &RetroRule) -> RuleReport {
    if rule.smirks.is_empty() {
        return RuleReport {
            name: rule.name.clone(),
            smirks: String::new(),
            graph_based: true,
            lhs_mapped_atom_count: None,
            lhs_has_ring_closure_digit: false,
            rhs_product_fragment_count: None,
            rhs_bare_single_atom_fragments: Vec::new(),
            flagged: false,
            flag_reasons: vec![
                "graph-based: requires is_bridge_bond (non-ring) by construction".to_string(),
            ],
        };
    }

    let Some((lhs, rhs)) = rule.smirks.split_once(">>") else {
        return RuleReport {
            name: rule.name.clone(),
            smirks: rule.smirks.clone(),
            graph_based: false,
            lhs_mapped_atom_count: None,
            lhs_has_ring_closure_digit: false,
            rhs_product_fragment_count: None,
            rhs_bare_single_atom_fragments: Vec::new(),
            flagged: false,
            flag_reasons: vec!["unparseable SMIRKS (no '>>')".to_string()],
        };
    };

    let lhs_mapped_atom_count = mol_from_smiles(lhs.trim())
        .ok()
        .map(|m| m.atoms().filter(|(_, a)| a.atom_map.is_some()).count());
    let lhs_has_ring = has_ring_closure_digit(lhs);

    let rhs_fragments: Vec<&str> = rhs.split('.').map(str::trim).collect();
    let rhs_product_fragment_count = Some(rhs_fragments.len());

    let bare_fragments: Vec<String> = rhs_fragments
        .iter()
        .filter(|frag| {
            mol_from_smiles(frag)
                .map(|m| m.atom_count() == 1)
                .unwrap_or(false)
        })
        .map(|frag| frag.to_string())
        .collect();

    let mut flag_reasons = Vec::new();
    if let Some(n) = lhs_mapped_atom_count
        && n <= 2
    {
        flag_reasons.push(format!(
            "minimal LHS: only {n} mapped atom(s) declared -- little context walling off BFS carry-through"
        ));
    }
    if rhs_fragments.len() >= 2 {
        flag_reasons.push(format!(
            "multi-product RHS: {} fragments",
            rhs_fragments.len()
        ));
    }
    if !bare_fragments.is_empty() {
        flag_reasons.push(format!(
            "bare single-atom RHS fragment(s): {} -- exact shape of the confirmed \
             aryl_amine_retro/buchwald_hartwig_retro defect",
            bare_fragments.join(", ")
        ));
    }
    if !lhs_has_ring {
        flag_reasons.push(
            "LHS declares no ring closure of its own (background fact for this whole \
             corpus, not independently discriminating)"
                .to_string(),
        );
    }

    // Flagged (candidate for a fixture) iff it has the two structurally
    // necessary ingredients together: a multi-product RHS AND at least one
    // bare single-atom RHS fragment. Minimal-LHS and no-ring-closure are
    // reported as supporting context on every candidate, not independent
    // triggers -- they're true for nearly this whole rule corpus and don't
    // discriminate on their own (see module doc).
    let flagged = rhs_fragments.len() >= 2 && !bare_fragments.is_empty();

    RuleReport {
        name: rule.name.clone(),
        smirks: rule.smirks.clone(),
        graph_based: false,
        lhs_mapped_atom_count,
        lhs_has_ring_closure_digit: lhs_has_ring,
        rhs_product_fragment_count,
        rhs_bare_single_atom_fragments: bare_fragments,
        flagged,
        flag_reasons,
    }
}

fn main() {
    let rules = default_rules();
    let reports: Vec<RuleReport> = rules.iter().map(analyze).collect();

    let flagged: Vec<&RuleReport> = reports.iter().filter(|r| r.flagged).collect();
    let graph_based: Vec<&RuleReport> = reports.iter().filter(|r| r.graph_based).collect();
    let single_product: Vec<&RuleReport> = reports
        .iter()
        .filter(|r| !r.graph_based && r.rhs_product_fragment_count == Some(1))
        .collect();

    eprintln!(
        "rule_safety_census: {} hand-crafted rules total ({} graph-based, {} single-product \
         SMIRKS, {} flagged)",
        reports.len(),
        graph_based.len(),
        single_product.len(),
        flagged.len()
    );

    // ---- Markdown report ----
    let mut md = String::new();
    md.push_str("# Rule-safety census (v0.36.0 Phase 1)\n\n");
    md.push_str(&format!(
        "Static SMIRKS screen of all {} `default_rules()` entries against the risk shape \
         that broke `aryl_amine_retro`/`buchwald_hartwig_retro` (issue #77). Screening only \
         -- a flag here is a reason to build a fixture, not a verdict. See \
         `docs/design/` for the full v0.36.0 plan.\n\n",
        reports.len()
    ));

    md.push_str("## Flagged: multi-product RHS with a bare single-atom fragment\n\n");
    md.push_str("| Rule | SMIRKS | LHS mapped atoms | Bare RHS fragment(s) |\n");
    md.push_str("|---|---|---:|---|\n");
    for r in &flagged {
        md.push_str(&format!(
            "| `{}` | `{}` | {} | {} |\n",
            r.name,
            r.smirks,
            r.lhs_mapped_atom_count
                .map(|n| n.to_string())
                .unwrap_or_default(),
            r.rhs_bare_single_atom_fragments.join(", ")
        ));
    }

    md.push_str("\n## Not flagged: single-product SMIRKS (no second-fragment risk)\n\n");
    md.push_str("| Rule | SMIRKS |\n|---|---|\n");
    for r in &single_product {
        md.push_str(&format!("| `{}` | `{}` |\n", r.name, r.smirks));
    }

    md.push_str("\n## Not flagged: graph-based (ring-guarded by construction)\n\n");
    md.push_str("| Rule |\n|---|\n");
    for r in &graph_based {
        md.push_str(&format!("| `{}` |\n", r.name));
    }

    md.push_str("\n## Full per-rule detail\n\n");
    for r in &reports {
        md.push_str(&format!("### `{}`\n\n", r.name));
        if r.graph_based {
            md.push_str("Graph-based (empty SMIRKS).\n\n");
        } else {
            md.push_str(&format!("SMIRKS: `{}`\n\n", r.smirks));
        }
        for reason in &r.flag_reasons {
            md.push_str(&format!("- {reason}\n"));
        }
        md.push('\n');
    }

    std::fs::write("rule_safety_census.md", &md).expect("write rule_safety_census.md");

    // ---- JSON report ----
    let json_rows: Vec<serde_json::Value> = reports
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "smirks": r.smirks,
                "graph_based": r.graph_based,
                "lhs_mapped_atom_count": r.lhs_mapped_atom_count,
                "lhs_has_ring_closure_digit": r.lhs_has_ring_closure_digit,
                "rhs_product_fragment_count": r.rhs_product_fragment_count,
                "rhs_bare_single_atom_fragments": r.rhs_bare_single_atom_fragments,
                "flagged": r.flagged,
                "flag_reasons": r.flag_reasons,
            })
        })
        .collect();
    let json_out = serde_json::json!({
        "total_rules": reports.len(),
        "flagged_count": flagged.len(),
        "graph_based_count": graph_based.len(),
        "single_product_count": single_product.len(),
        "rules": json_rows,
    });
    std::fs::write(
        "rule_safety_census.json",
        serde_json::to_string_pretty(&json_out).unwrap(),
    )
    .expect("write rule_safety_census.json");

    println!("wrote rule_safety_census.md and rule_safety_census.json");
}
