//! SpectatorBondLoss: detects a real target bond that a retro-rule's own
//! SMIRKS never declares as broken, yet chematic's reaction engine silently
//! drops when assembling precursors -- turning a step that looks
//! atom-balanced into one whose precursors can never forward-reconstruct
//! the target, for a purely mechanical (not chemical) reason.
//!
//! Grew out of Finding #4's pilot investigation
//! (`docs/validation/finding4-validator-pilot-2026-08-23.md`): four of six
//! Invalid+balanced steps (`extracted_112`/`extracted_824`/`extracted_109`/
//! `extracted_4255`) turned out to share this exact mechanical cause, not
//! four unrelated chemistry mistakes. Confirmed by reading chematic-rxn
//! 0.16.0's own `transform.rs` (the pinned version, not just a local
//! checkout) and empirically verifying with a synthetic aziridine fixture:
//! `run_reactants` NEVER carries `atom_map` through to product atoms (every
//! output atom's map is unconditionally cleared -- confirmed directly, not
//! assumed), so detection has to work from the *target side*, using the
//! rule's own declared structure and a real substructure match -- never by
//! inspecting precursor output.
//!
//! Two distinct mechanisms, not one:
//!
//! - **Case A** ([`detect_case_a`]): a bond exists in the real target
//!   between two atoms the rule's SMIRKS matches (both carry a `:N` map
//!   number), but neither the LHS nor any RHS fragment declares an edge
//!   between those two map numbers. chematic's carry-through step
//!   unconditionally skips any bond where *both* endpoints are matched
//!   atoms, regardless of what the template intends
//!   (`all_template_atoms.contains(a) && contains(b)` in `transform.rs`) --
//!   a static, per-rule question once a match is found, no BFS needed.
//!   Positive controls: `extracted_824`, `extracted_4255`.
//! - **Case B** ([`detect_case_b`]): a real target path connects a matched
//!   atom of one `.`-separated RHS product fragment to a matched atom of a
//!   *different* fragment, running only through genuinely unmatched atoms
//!   (i.e. no direct bond between the two matched endpoints -- that's Case
//!   A's territory). chematic assembles each product fragment as a wholly
//!   separate molecule, so this chain can never survive intact in either
//!   output. Deliberately not implemented as a line-by-line port of
//!   chematic's own BFS-with-globally-seeded-`visited` internals (those are
//!   private, and copying implementation details this closely would
//!   silently drift out of sync with any future chematic release) --
//!   implemented instead as RENKIN's own independent territory model, a
//!   plain BFS over the target's real bond graph blocked from routing
//!   through any matched atom other than the two endpoints. Positive
//!   controls: `extracted_109`, `extracted_112`.
//!
//! All four of Finding #4's `genuine_template_error` instances were
//! originally assumed, before writing and running these detectors, to
//! split 3-Case-A/1-Case-B (`extracted_824`/`extracted_109`/`extracted_112`
//! direct, `extracted_4255` alone needing the BFS model). Actually running
//! the code against each real fixture corrected that twice over: tracing
//! `extracted_109`/`extracted_112`'s real connectivity showed their lost
//! bonds each run through unmatched intermediate atoms, not a direct edge
//! (Case B, not Case A) -- and `extracted_4255`'s "aromatic ring neighbor
//! the LHS pattern never examines" turned out to itself be a *matched* atom
//! (map number 6, from the third RHS fragment), directly bonded to the
//! carbonyl carbon (map number 2, from the second fragment) -- a plain Case
//! A instance, not Case B. The real split is 2-and-2, not 3-and-1.
//!
//! Both cases report through the same [`SpectatorBondLossFinding`] shape,
//! deliberately separate from any accept/reject decision: detection always
//! runs (once enabled) and the full finding set is always preserved,
//! independent of whether a caller's policy later excludes the candidate --
//! the same "policy changes only the verdict, never the findings"
//! separation `bridge::audit::AuditPolicy` already established for
//! post-hoc route auditing (v0.29.0).

use std::collections::{HashMap, HashSet, VecDeque};

use chematic::core::{AtomIdx, BondOrder};
use chematic::rxn::{ReactionMatch, apply_reaction_match, find_reaction_matches};
use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::chem_env::{Molecule, RetroRule, mol_from_smiles, split_fragments};

/// Which of the two mechanisms a [`SpectatorBondLossFinding`] was detected
/// under -- kept on the finding itself (not just implied by which detector
/// ran) since both may eventually run together and a caller inspecting a
/// finding shouldn't have to know which function produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpectatorBondLossCase {
    /// Two SMIRKS-matched atoms are really bonded in the target; neither
    /// the LHS nor any RHS fragment declares that edge.
    MatchedPairUndeclared,
    /// A matched atom is really bonded to an atom that belongs, by
    /// connectivity, to a *different* RHS product template's territory.
    CrossProductTerritory,
}

/// Readable label for a [`BondOrder`], exhaustive over chematic-core
/// 0.16.0's variant set on purpose (a `_` wildcard would silently swallow
/// any bond order added in a future chematic release rather than failing
/// to compile as a visible reminder to classify it).
fn bond_order_label(order: BondOrder) -> &'static str {
    match order {
        BondOrder::Single => "single",
        BondOrder::Double => "double",
        BondOrder::Triple => "triple",
        BondOrder::Quadruple => "quadruple",
        BondOrder::Aromatic => "aromatic",
        BondOrder::Up => "up",
        BondOrder::Down => "down",
        BondOrder::Zero => "zero",
        BondOrder::Dative => "dative",
        BondOrder::QueryAny => "query_any",
        BondOrder::QuerySingleOrDouble => "query_single_or_double",
        BondOrder::QuerySingleOrAromatic => "query_single_or_aromatic",
        BondOrder::QueryDoubleOrAromatic => "query_double_or_aromatic",
    }
}

/// One target bond a rule's own structure never accounts for.
/// `source_atom_a`/`source_atom_b` are the target molecule's own
/// [`AtomIdx`] values (0-based, in the target's own atom order) -- stable
/// identifiers into the exact target this detection ran against, not the
/// rule's own (much smaller, match-specific) map-number space, since Case B
/// atoms don't necessarily carry a SMIRKS map number at all.
#[derive(Debug, Clone, Serialize)]
pub struct LostBond {
    pub source_atom_a: u32,
    pub source_atom_b: u32,
    pub bond_order: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpectatorBondLossFinding {
    pub template_id: String,
    pub rule_name: String,
    pub case: SpectatorBondLossCase,
    pub lost_bonds: Vec<LostBond>,
    pub evidence: String,
}

/// How much SpectatorBondLoss work `raw_propose` does for one search or
/// candidate-generation call. Replaces the earlier plain
/// `spectator_bond_diagnostics: bool` field -- pre-release only, no
/// published crate version ever shipped that field, so this is a free
/// rename, not a breaking change. Mirrors `bridge::audit::AuditPolicy`'s
/// "policy changes only the verdict, never the finding set" separation:
/// `Gated` runs exactly the same detection `DiagnosticsOnly` does, it just
/// additionally acts on the findings [`gate_candidates`] resolves with
/// confidence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpectatorBondPolicy {
    /// No detection at all -- zero cost, matches the pre-v0.35 absent
    /// behavior.
    #[default]
    Off,
    /// Detect and record findings; never excludes a candidate.
    DiagnosticsOnly,
    /// Detect, record, and additionally reject candidates [`gate_candidates`]
    /// resolves with confidence (v1 scope: `#`-free rules only -- see
    /// `docs/design/spectator-bond-fail-closed-gating-v0.md` §2).
    Gated,
}

/// One candidate `raw_propose` excluded under [`SpectatorBondPolicy::Gated`]
/// -- the diagnostic trail an exclusion leaves behind even though the
/// candidate itself no longer appears in the returned proposal list.
#[derive(Debug, Clone, Serialize)]
pub struct GatedCandidateRecord {
    pub rule_name: String,
    pub template_id: String,
    pub precursor_smiles: Vec<String>,
    pub findings: Vec<SpectatorBondLossFinding>,
}

/// Map-number pairs (unordered, smaller-first) with a declared bond
/// somewhere in `pattern`'s own structure. `pattern` is either a retro
/// rule's LHS or one `.`-split RHS fragment. Both parse as plain
/// SMILES-with-atom-maps -- SMIRKS in this codebase never uses true SMARTS
/// operators (see feedback memory: chematic-rxn's SMIRKS engine treats them
/// as inert literal syntax, not query operators), so `mol_from_smiles`
/// (not `parse_smarts`) is the right parser for reading out a pattern's own
/// literal bond graph. An atom with no map number can never contribute a
/// pair here by construction (only mapped atoms are ever looked up against
/// this set in [`detect_case_a`]).
///
/// `None` means `pattern` failed to parse -- distinct from "parsed fine
/// and declares nothing." Diagnostics-only callers ([`detect_case_a`])
/// treat the two the same (`unwrap_or_default`, matching this function's
/// pre-existing behavior). [`gate_candidates`] does not: an unparseable
/// fragment can only make the declared set too *small*, which risks
/// misclassifying a real declared bond as spectator loss -- a false
/// reject, not a false negative -- so gating must know the difference.
fn declared_map_pairs(pattern: &str) -> Option<HashSet<(u16, u16)>> {
    let mol = mol_from_smiles(pattern.trim()).ok()?;
    let mut pairs = HashSet::new();
    for (_, bond) in mol.bonds() {
        let a = mol.atom(bond.atom1).atom_map;
        let b = mol.atom(bond.atom2).atom_map;
        if let (Some(a), Some(b)) = (a, b) {
            pairs.insert(if a < b { (a, b) } else { (b, a) });
        }
    }
    Some(pairs)
}

/// Case A lost bonds for one already-resolved match, with no cross-match
/// dedup -- [`detect_case_a`] (aggregating over every match) does its own
/// dedup on top of this; [`gate_candidates`] (evaluating one match at a
/// time) uses it as-is. Shared so the two can never define "lost bond"
/// differently.
fn case_a_lost_bonds_for_match(
    target: &Molecule,
    declared: &HashSet<(u16, u16)>,
    positions: &FxHashMap<u16, (usize, AtomIdx)>,
) -> Vec<LostBond> {
    let map_numbers: Vec<u16> = positions.keys().copied().collect();
    let mut lost_bonds = Vec::new();
    for i in 0..map_numbers.len() {
        for j in (i + 1)..map_numbers.len() {
            let (mi, mj) = (map_numbers[i], map_numbers[j]);
            let key = if mi < mj { (mi, mj) } else { (mj, mi) };
            if declared.contains(&key) {
                continue;
            }
            let (_, ai) = positions[&mi];
            let (_, aj) = positions[&mj];
            let Some((_, bond)) = target.bond_between(ai, aj) else {
                continue;
            };
            lost_bonds.push(LostBond {
                source_atom_a: ai.0,
                source_atom_b: aj.0,
                bond_order: bond_order_label(bond.order),
            });
        }
    }
    lost_bonds
}

/// Case A detector -- see this module's own doc comment for the mechanism.
/// Graph-based rules (empty `smirks`) trivially have nothing to analyze.
/// One finding per real substructure match that has at least one lost
/// bond; a single physical target bond is never reported twice even if
/// more than one match happens to cover it (matches can overlap on
/// symmetric targets). Positive controls: `extracted_824`, `extracted_4255`.
pub fn detect_case_a(target: &Molecule, rule: &RetroRule) -> Vec<SpectatorBondLossFinding> {
    if rule.smirks.is_empty() {
        return Vec::new();
    }
    let Some((lhs, rhs)) = rule.smirks.split_once(">>") else {
        return Vec::new();
    };
    let mut declared = declared_map_pairs(lhs).unwrap_or_default();
    for fragment in rhs.split('.') {
        declared.extend(declared_map_pairs(fragment).unwrap_or_default());
    }

    let Ok(matches) = find_reaction_matches(&rule.smirks, &[target]) else {
        return Vec::new();
    };

    let mut findings = Vec::new();
    let mut seen_bonds: HashSet<(u32, u32)> = HashSet::new();
    for m in &matches {
        let Ok(positions) = m.atom_map_positions(&rule.smirks) else {
            continue;
        };
        let mut lost_bonds = Vec::new();
        for bond in case_a_lost_bonds_for_match(target, &declared, &positions) {
            let bond_key = (
                bond.source_atom_a.min(bond.source_atom_b),
                bond.source_atom_a.max(bond.source_atom_b),
            );
            if seen_bonds.insert(bond_key) {
                lost_bonds.push(bond);
            }
        }
        if !lost_bonds.is_empty() {
            findings.push(SpectatorBondLossFinding {
                template_id: rule.template_id.clone(),
                rule_name: rule.name.clone(),
                case: SpectatorBondLossCase::MatchedPairUndeclared,
                evidence: format!(
                    "{} bond(s) between matched atoms exist in the target but are declared \
                     broken by neither {}'s LHS nor any RHS fragment",
                    lost_bonds.len(),
                    rule.name
                ),
                lost_bonds,
            });
        }
    }
    findings
}

/// Case B detector: RENKIN's own independent territory model, built
/// directly from the match and the target's real bond graph -- not a port
/// of chematic's private per-product `visited`/`src_to_new` assembly
/// internals (those are undocumented implementation details a clean-room
/// model deliberately avoids depending on).
///
/// For a multi-product RHS (2+ `.`-separated fragments), chematic assembles
/// each fragment as an entirely separate molecule. A real target path that
/// connects a matched atom of one product fragment to a matched atom of a
/// *different* product fragment, running only through genuinely unmatched
/// atoms, can never survive intact in either product's own output: traced
/// atom-by-atom against chematic-rxn 0.16.0's real assembly behavior on
/// `extracted_112` (see this module's own doc comment) -- the connecting
/// unmatched atom(s) end up duplicated as terminal substituents in *both*
/// outputs, each one missing one of the two real bonds that would keep the
/// chain whole. A *direct* bond between two matched atoms of different
/// products is [`detect_case_a`]'s territory, not this function's -- Case A
/// doesn't care which product a matched atom's map number belongs to, only
/// whether the direct edge is declared anywhere. Positive controls:
/// `extracted_109`, `extracted_112` (see this module's own doc comment for
/// why `extracted_4255` -- originally assumed to need this detector -- is
/// actually a Case A instance instead).
pub fn detect_case_b(target: &Molecule, rule: &RetroRule) -> Vec<SpectatorBondLossFinding> {
    if rule.smirks.is_empty() {
        return Vec::new();
    }
    let Some((_, rhs)) = rule.smirks.split_once(">>") else {
        return Vec::new();
    };
    let fragments: Vec<&str> = rhs.split('.').map(str::trim).collect();
    if fragments.len() < 2 {
        return Vec::new(); // one product -- no "different product" to cross into
    }

    let owner = rhs_fragment_owner_map(&fragments);

    let Ok(matches) = find_reaction_matches(&rule.smirks, &[target]) else {
        return Vec::new();
    };

    let mut findings = Vec::new();
    for m in &matches {
        let Ok(positions) = m.atom_map_positions(&rule.smirks) else {
            continue;
        };
        for (mi, mj, lost_bonds) in case_b_lost_bond_chains_for_match(target, &owner, &positions) {
            findings.push(SpectatorBondLossFinding {
                template_id: rule.template_id.clone(),
                rule_name: rule.name.clone(),
                case: SpectatorBondLossCase::CrossProductTerritory,
                evidence: format!(
                    "matched atoms {mi} and {mj} (declared by different RHS product \
                     fragments in {}) are connected in the target only through unmatched \
                     atoms -- chematic assembles each product fragment as a separate \
                     molecule, so this chain can never survive intact in either output",
                    rule.name
                ),
                lost_bonds,
            });
        }
    }
    findings
}

/// `atom_map` number -> which `.`-split RHS fragment (by index) declares
/// it. A fragment that fails to parse simply contributes no entries (same
/// "fail open" behavior this had inline in [`detect_case_b`] before this
/// was extracted) -- unlike [`declared_map_pairs`]'s failure mode, an
/// under-populated `owner` map can only make [`detect_case_b`] miss a real
/// cross-fragment pair, never flag a false one, so it carries no gating
/// risk worth a `not_evaluable` guard of its own.
fn rhs_fragment_owner_map(fragments: &[&str]) -> HashMap<u16, usize> {
    let mut owner = HashMap::new();
    for (k, frag) in fragments.iter().enumerate() {
        let Ok(mol) = mol_from_smiles(frag) else {
            continue;
        };
        for (_, atom) in mol.atoms() {
            if let Some(m) = atom.atom_map {
                owner.insert(m, k);
            }
        }
    }
    owner
}

/// Case B lost-bond chains for one already-resolved match: one
/// `(map_a, map_b, chain)` entry per cross-product-fragment matched-atom
/// pair connected only through unmatched atoms (mirrors [`detect_case_b`]'s
/// existing one-finding-per-pair granularity). Shared so `detect_case_b`
/// and [`gate_candidates`] can never define "lost chain" differently.
fn case_b_lost_bond_chains_for_match(
    target: &Molecule,
    owner: &HashMap<u16, usize>,
    positions: &FxHashMap<u16, (usize, AtomIdx)>,
) -> Vec<(u16, u16, Vec<LostBond>)> {
    let all_matched: HashSet<AtomIdx> = positions.values().map(|&(_, a)| a).collect();
    let map_numbers: Vec<u16> = positions.keys().copied().collect();
    let mut checked_pairs: HashSet<(u16, u16)> = HashSet::new();
    let mut chains = Vec::new();

    for i in 0..map_numbers.len() {
        for j in (i + 1)..map_numbers.len() {
            let (mi, mj) = (map_numbers[i], map_numbers[j]);
            let (Some(&owner_i), Some(&owner_j)) = (owner.get(&mi), owner.get(&mj)) else {
                continue; // map number never appears on the RHS at all
            };
            if owner_i == owner_j {
                continue; // same product fragment -- not this detector's scope
            }
            let key = if mi < mj { (mi, mj) } else { (mj, mi) };
            if !checked_pairs.insert(key) {
                continue;
            }
            let (_, a) = positions[&mi];
            let (_, b) = positions[&mj];
            if target.bond_between(a, b).is_some() {
                continue; // direct bond -- detect_case_a's territory
            }
            let Some(path) = unmatched_only_path(target, a, b, &all_matched) else {
                continue;
            };
            let lost_bonds: Vec<LostBond> = path
                .windows(2)
                .filter_map(|w| {
                    target.bond_between(w[0], w[1]).map(|(_, bond)| LostBond {
                        source_atom_a: w[0].0,
                        source_atom_b: w[1].0,
                        bond_order: bond_order_label(bond.order),
                    })
                })
                .collect();
            if !lost_bonds.is_empty() {
                chains.push((mi, mj, lost_bonds));
            }
        }
    }
    chains
}

/// Shortest real-target path from `start` to `end`, refusing to route
/// through any atom in `blocked` other than the two endpoints themselves --
/// plain BFS over [`Molecule::neighbors`], independent of chematic's own
/// substructure-matching/assembly code. `None` if no such path exists.
fn unmatched_only_path(
    target: &Molecule,
    start: AtomIdx,
    end: AtomIdx,
    blocked: &HashSet<AtomIdx>,
) -> Option<Vec<AtomIdx>> {
    let mut queue: VecDeque<AtomIdx> = VecDeque::from([start]);
    let mut came_from: HashMap<AtomIdx, AtomIdx> = HashMap::new();
    let mut visited: HashSet<AtomIdx> = HashSet::from([start]);
    while let Some(cur) = queue.pop_front() {
        if cur == end {
            let mut path = vec![end];
            let mut node = end;
            while let Some(&prev) = came_from.get(&node) {
                path.push(prev);
                node = prev;
            }
            path.reverse();
            return Some(path);
        }
        for (nb, _) in target.neighbors(cur) {
            if visited.contains(&nb) || (nb != end && blocked.contains(&nb)) {
                continue;
            }
            visited.insert(nb);
            came_from.insert(nb, cur);
            queue.push_back(nb);
        }
    }
    None
}

/// Fail-closed gating verdict for one already-generated candidate.
/// Deliberately identified only by the candidate's own canonical
/// precursor-SMILES signature (see [`gate_candidates`]) rather than by a
/// `crate::candidate::RawCandidate` reference -- keeps this module
/// decoupled from candidate-generation's own types and testable in
/// isolation. See `docs/design/spectator-bond-fail-closed-gating-v0.md`
/// for the full design.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SpectatorBondGateVerdict {
    Accepted,
    Rejected {
        findings: Vec<SpectatorBondLossFinding>,
    },
    NotEvaluable {
        reason: &'static str,
    },
}

/// Fail-closed gating for one rule's already-generated candidates.
/// `candidate_signatures[i]` is candidate `i`'s own sorted, canonical
/// precursor-SMILES multiset (exactly what `RawCandidate.precursors`
/// already carries) -- returns one verdict per entry, same order.
///
/// v1 scope (`docs/design/spectator-bond-fail-closed-gating-v0.md` §2/§3):
/// only rules with no `#` in `rule.smirks` are ever evaluated with
/// confidence -- a `[#N]`-bearing rule applies concrete-element *variant*
/// strings in `apply_retro`, not this literal `rule.smirks`, so the replay
/// this function relies on wouldn't correspond to what was actually
/// applied. A rule whose LHS or any RHS fragment fails to parse is also
/// never evaluated with confidence, since `detect_case_a`'s "declared"
/// set would be too small, not too large -- that risks a false reject,
/// never a false accept. Both cases return `NotEvaluable` for every one
/// of the rule's candidates, never a silent `Accepted`.
pub fn gate_candidates(
    target: &Molecule,
    rule: &RetroRule,
    candidate_signatures: &[Vec<String>],
) -> Vec<SpectatorBondGateVerdict> {
    let accept_all = || vec![SpectatorBondGateVerdict::Accepted; candidate_signatures.len()];
    let not_evaluable_all = |reason: &'static str| {
        vec![SpectatorBondGateVerdict::NotEvaluable { reason }; candidate_signatures.len()]
    };

    if rule.smirks.is_empty() {
        return accept_all(); // graph-based rule -- nothing to analyze
    }
    if rule.smirks.contains('#') {
        return not_evaluable_all("hash_atom_wildcard_rule");
    }
    let Some((lhs, rhs)) = rule.smirks.split_once(">>") else {
        return accept_all();
    };
    let Some(mut declared) = declared_map_pairs(lhs) else {
        return not_evaluable_all("unparseable_declared_bonds");
    };
    for fragment in rhs.split('.') {
        let Some(frag_pairs) = declared_map_pairs(fragment) else {
            return not_evaluable_all("unparseable_declared_bonds");
        };
        declared.extend(frag_pairs);
    }

    let fragments: Vec<&str> = rhs.split('.').map(str::trim).collect();
    let owner = rhs_fragment_owner_map(&fragments);

    let Ok(matches) = find_reaction_matches(&rule.smirks, &[target]) else {
        return accept_all();
    };

    // Candidate signature -> one findings-list per match that replayed to
    // that exact signature (usually exactly one match; a symmetric target
    // can have several matches collapse onto the same product set).
    let mut by_signature: HashMap<Vec<String>, Vec<Vec<SpectatorBondLossFinding>>> = HashMap::new();
    for m in &matches {
        let Some(signature) = replay_match_signature(&rule.smirks, target, m) else {
            continue; // valence-rejected or replay failed -- matches run_reactants' own filter
        };
        let Ok(positions) = m.atom_map_positions(&rule.smirks) else {
            continue;
        };
        let mut findings = Vec::new();
        let case_a = case_a_lost_bonds_for_match(target, &declared, &positions);
        if !case_a.is_empty() {
            findings.push(build_finding(
                rule,
                SpectatorBondLossCase::MatchedPairUndeclared,
                case_a,
            ));
        }
        for (_, _, lost_bonds) in case_b_lost_bond_chains_for_match(target, &owner, &positions) {
            findings.push(build_finding(
                rule,
                SpectatorBondLossCase::CrossProductTerritory,
                lost_bonds,
            ));
        }
        by_signature.entry(signature).or_default().push(findings);
    }

    candidate_signatures
        .iter()
        .map(|sig| match by_signature.get(sig) {
            // No replayed match produced this exact candidate -- e.g. a
            // ring-context-guarded outcome this replay (deliberately
            // independent of that pipeline) can't reproduce. Never treated
            // as a clean pass: it's a correlation gap, not a clean bill of
            // health.
            None => SpectatorBondGateVerdict::NotEvaluable {
                reason: "match_correlation_failed",
            },
            Some(per_match) => {
                let defective: Vec<&Vec<SpectatorBondLossFinding>> =
                    per_match.iter().filter(|f| !f.is_empty()).collect();
                let clean_count = per_match.len() - defective.len();
                if !defective.is_empty() && clean_count > 0 {
                    // Same signature reached by both a defective and a
                    // clean match -- can't tell which one this candidate
                    // really is, so don't guess either way.
                    SpectatorBondGateVerdict::NotEvaluable {
                        reason: "ambiguous_signature",
                    }
                } else if let Some(first_defective) = defective.first() {
                    SpectatorBondGateVerdict::Rejected {
                        findings: (*first_defective).clone(),
                    }
                } else {
                    SpectatorBondGateVerdict::Accepted
                }
            }
        })
        .collect()
}

/// One match's own product set, replayed independently of whatever
/// `apply_retro`/`apply_retro_with_policy` actually produced. For `#`-free
/// rules this reproduces `run_reactants`'s own output for this exact
/// match, byte-for-byte: chematic-rxn 0.16.0's own `run_reactants_impl` is
/// literally `find_matches_impl` (what `find_reaction_matches` calls)
/// followed by `apply_match_impl` per match (what `apply_reaction_match`
/// calls) -- not a parallel reimplementation that could drift, confirmed
/// by reading `transform.rs` directly. `None` on a valence-rejected match
/// (mirrors `run_reactants`'s own `filter_map`) or a genuine replay error.
fn replay_match_signature(
    smirks: &str,
    target: &Molecule,
    m: &ReactionMatch,
) -> Option<Vec<String>> {
    let products = apply_reaction_match(smirks, &[target], m, true).ok()??;
    let mut signature: Vec<String> = products
        .iter()
        .flat_map(split_fragments)
        .map(|p| p.smiles)
        .collect();
    signature.sort_unstable();
    Some(signature)
}

/// Wraps a raw lost-bond list into a typed finding with the same evidence
/// phrasing `detect_case_a`/`detect_case_b` use, so a `Rejected` verdict's
/// findings read identically whether they came from those aggregate
/// detectors or from this per-match gating path.
fn build_finding(
    rule: &RetroRule,
    case: SpectatorBondLossCase,
    lost_bonds: Vec<LostBond>,
) -> SpectatorBondLossFinding {
    let evidence = match case {
        SpectatorBondLossCase::MatchedPairUndeclared => format!(
            "{} bond(s) between matched atoms exist in the target but are declared broken by \
             neither {}'s LHS nor any RHS fragment",
            lost_bonds.len(),
            rule.name
        ),
        SpectatorBondLossCase::CrossProductTerritory => format!(
            "matched atoms (declared by different RHS product fragments in {}) are connected \
             in the target only through unmatched atoms -- chematic assembles each product \
             fragment as a separate molecule, so this chain can never survive intact in either \
             output",
            rule.name
        ),
    };
    SpectatorBondLossFinding {
        template_id: rule.template_id.clone(),
        rule_name: rule.name.clone(),
        case,
        evidence,
        lost_bonds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rr(name: &str, smirks: &str) -> RetroRule {
        RetroRule {
            name: name.into(),
            template_id: format!("rule:{name}"),
            smirks: smirks.into(),
            ..Default::default()
        }
    }

    // ── declared_map_pairs ──────────────────────────────────────────────

    #[test]
    fn declared_map_pairs_reads_simple_chain() {
        let pairs = declared_map_pairs("[C:1]-[O:2]-[C:3]").unwrap();
        assert_eq!(pairs, HashSet::from([(1, 2), (2, 3)]));
    }

    #[test]
    fn declared_map_pairs_ignores_unmapped_atoms() {
        // The leaving-group O has no map number -- must not appear in any pair.
        let pairs = declared_map_pairs("[C:1]OC").unwrap();
        assert!(pairs.iter().all(|&(a, b)| a == 1 || b == 1));
    }

    #[test]
    fn declared_map_pairs_none_for_unparseable_pattern() {
        // `None`, not merely empty -- a parse failure must stay
        // distinguishable from "parsed fine and declares nothing" so
        // gate_candidates can refuse to trust an empty declared-set that
        // might just be a parse failure in disguise.
        assert_eq!(declared_map_pairs("not smiles at all !!"), None);
    }

    // ── detect_case_a: positive controls (Finding #4) ───────────────────

    #[test]
    fn detects_extracted_4255_difluorophthalide_ring_bond() {
        // Originally assumed to need Case B (the doc comment merged with
        // this fixture in PR #181 described the lost bond as running to
        // "an aromatic atom the LHS pattern never even examines"). Actually
        // running find_reaction_matches shows that atom (map number 6, the
        // third RHS fragment's own aromatic core) IS matched -- directly
        // bonded to the carbonyl carbon (map number 2, the second
        // fragment's core) -- a plain Case A instance.
        let target = mol_from_smiles("C2c1cc(c(F)cc1C(O2)=O)F").unwrap();
        let rule = rr(
            "extracted_4255",
            "[C:2]-[O:3]-[CH2:1]-[c:5](:[c:4]):[c:6]>>O=[CH2:1].[C:2]-[OH:3].[c:4]:[cH:5]:[c:6]",
        );
        let findings = detect_case_a(&target, &rule);
        assert!(
            !findings.is_empty(),
            "must detect the undeclared bond between the carbonyl carbon (map 2) and its \
             aromatic ring neighbor (map 6)"
        );
    }

    #[test]
    fn detects_extracted_824_oxazolidinone_ring_bond() {
        let target = mol_from_smiles("O=C2NCC(O2)Cc1ccccc1").unwrap();
        let rule = rr(
            "extracted_824",
            "[C:5]-[O:6]-[C:3](=[O:4])-[NH:2]-[C:1]>>[C:1]-[N:2]=[C:3]=[O:4].[C:5]-[OH:6]",
        );
        let findings = detect_case_a(&target, &rule);
        assert!(
            !findings.is_empty(),
            "must detect the undeclared C1-C5 ring-closing bond"
        );
        assert_eq!(
            findings[0].case,
            SpectatorBondLossCase::MatchedPairUndeclared
        );
        assert_eq!(
            findings.iter().map(|f| f.lost_bonds.len()).sum::<usize>(),
            1,
            "exactly one real bond (C1-C5) is undeclared here"
        );
    }

    // extracted_109 and extracted_112 turned out to be Case B (cross-product
    // territory), not Case A -- see this module's own doc comment for the
    // full trace. detect_case_a correctly finds nothing for either; that's
    // the right answer, tested as a negative control below, not a positive
    // one. Real Case A/B positive-control coverage for both lands together
    // once detect_case_b exists.

    #[test]
    fn extracted_109_azepane_is_not_a_case_a_instance() {
        let target = mol_from_smiles("C1CCCC[C@H](N1)C").unwrap();
        let rule = rr(
            "extracted_109",
            "[C:2]-[CH:1](-[NH:5]-[C:4])-[C:3]>>O=[C:1](-[C:2])-[C:3].[C:4]-[NH2:5]",
        );
        assert!(
            detect_case_a(&target, &rule).is_empty(),
            "extracted_109's defect is Case B (matched atom reachable from a different \
             product's core only through unmatched substituents) plus a separate atom-count \
             inflation issue, not a direct matched-matched undeclared bond -- detect_case_a \
             must not misfire here"
        );
    }

    #[test]
    fn extracted_112_indanone_is_not_a_case_a_instance() {
        let target = mol_from_smiles("c2ccc1CCC(c1c2)=O").unwrap();
        let rule = rr(
            "extracted_112",
            "[C:2]-[C:1](=[O:3])-[c:5](:[c:4]):[c:6]>>Cl-[C:1](-[C:2])=[O:3].[c:4]:[cH:5]:[c:6]",
        );
        assert!(
            detect_case_a(&target, &rule).is_empty(),
            "extracted_112's defect is Case B (the fused-ring bond runs through an unmatched \
             CH2 to a matched aromatic atom belonging to the OTHER product's core), not a \
             direct matched-matched undeclared bond -- detect_case_a must not misfire here"
        );
    }

    // ── detect_case_a: negative controls ────────────────────────────────
    // Must NOT fire on rules/targets this detector isn't meant to flag --
    // co_aliphatic_cleavage and cc_single_cleavage are separately classified
    // (source_step_underspecified / stereo_underspecified +
    // chemically_implausible_precursor, see
    // docs/validation/finding4-validator-pilot-2026-08-23.md), not
    // spectator_bond_loss, and ordinary two-fragment disconnections on
    // genuinely separate starting materials must never trigger this at all.

    #[test]
    fn co_aliphatic_cleavage_piperidinyl_carbamate_is_not_flagged() {
        // Same target/rule as the merged co_aliphatic_cleavage fixture
        // (src/validation/forward.rs) -- its defect is a missing stereo
        // assignment on an otherwise-correct, fully declared disconnection,
        // not an undeclared bond.
        let target = mol_from_smiles("O=C(OC1CCCNC1)N").unwrap();
        let rule = rr("co_aliphatic_cleavage", "[C:1][O:2]>>[C:1].[O:2]");
        assert!(
            detect_case_a(&target, &rule).is_empty(),
            "a real, fully-declared aliphatic C-O cleavage must never be flagged"
        );
    }

    #[test]
    fn cc_single_cleavage_azepane_methane_is_not_flagged() {
        // Same target/rule as the merged cc_single_cleavage fixture -- its
        // defect is an implausible precursor (bare methane), not a
        // connectivity/undeclared-bond problem; the declared C-C bond IS
        // the one real bond between the two matched atoms.
        let target = mol_from_smiles("C1CC[C@H](C)NC[C@@H]1C").unwrap();
        let rule = rr("cc_single_cleavage", "[C:1][C:2]>>[C:1].[C:2]");
        assert!(
            detect_case_a(&target, &rule).is_empty(),
            "a fully-declared single-bond cleavage must never be flagged"
        );
    }

    #[test]
    fn ordinary_intermolecular_amide_formation_is_not_flagged() {
        // A completely standard, real retro-amide-formation step: acid +
        // amine, two genuinely separate molecules with no other tether.
        let target = mol_from_smiles("CC(=O)NC").unwrap(); // N-methylacetamide
        let rule = rr(
            "amide_formation_retro",
            "[C:1](=[O:2])-[N:3]>>[C:1](=[O:2])[OH].[N:3]",
        );
        assert!(
            detect_case_a(&target, &rule).is_empty(),
            "ordinary intermolecular amide retro must never be flagged"
        );
    }

    #[test]
    fn ordinary_reductive_amination_is_not_flagged() {
        let target = mol_from_smiles("CC(C)NC").unwrap(); // N-methylisopropylamine
        let rule = rr(
            "reductive_amination_retro",
            "[C:1](-[C:4])(-[C:5])-[N:2]-[C:3]>>O=[C:1](-[C:4])-[C:5].[N:2]-[C:3]",
        );
        assert!(
            detect_case_a(&target, &rule).is_empty(),
            "ordinary reductive amination retro must never be flagged"
        );
    }

    #[test]
    fn legitimate_ring_opening_with_fully_declared_bonds_is_not_flagged() {
        // Cyclohexanol -> ring-opened via a fully-declared single-bond
        // cleavage (both endpoints matched, edge declared on the LHS) --
        // must not be confused with the ring-closing-bond-loss failure
        // class, since here the declared bond IS the one being cut.
        let target = mol_from_smiles("OC1CCCCC1").unwrap();
        let rule = rr("ring_open_retro", "[C:1][C:2]>>[C:1].[C:2]");
        assert!(
            detect_case_a(&target, &rule).is_empty(),
            "a fully-declared ring-bond cleavage must never be flagged, even though it acts on a ring"
        );
    }

    #[test]
    fn unrelated_distant_ring_on_target_is_not_flagged() {
        // The rule's match sits entirely outside a ring elsewhere in the
        // same molecule -- that unrelated ring's own bonds must never be
        // treated as a spectator loss (they were never matched atoms).
        let target = mol_from_smiles("c1ccccc1CC(=O)NC").unwrap(); // phenylacetyl-N-methylamide
        let rule = rr(
            "amide_formation_retro",
            "[C:1](=[O:2])-[N:3]>>[C:1](=[O:2])[OH].[N:3]",
        );
        assert!(
            detect_case_a(&target, &rule).is_empty(),
            "an unrelated ring elsewhere in the molecule must never trigger a false positive"
        );
    }

    #[test]
    fn suzuki_style_two_fragment_disconnection_is_not_flagged() {
        // Biphenyl -> two genuinely separate aryl fragments, no tether.
        let target = mol_from_smiles("c1ccc(cc1)-c1ccccc1").unwrap();
        let rule = rr("suzuki_retro_smirks", "[c:1]-[c:2]>>[c:1].[c:2]");
        assert!(
            detect_case_a(&target, &rule).is_empty(),
            "a real Suzuki-style two-fragment disconnection on separate rings must never be flagged"
        );
    }

    #[test]
    fn graph_based_rule_with_empty_smirks_returns_no_findings() {
        let target = mol_from_smiles("CCO").unwrap();
        let rule = rr("ester_cleavage", "");
        assert!(detect_case_a(&target, &rule).is_empty());
        assert!(detect_case_b(&target, &rule).is_empty());
    }

    // ── detect_case_b: positive controls (Finding #4) ───────────────────

    #[test]
    fn detects_extracted_112_indanone_cross_product_chain() {
        let target = mol_from_smiles("c2ccc1CCC(c1c2)=O").unwrap();
        let rule = rr(
            "extracted_112",
            "[C:2]-[C:1](=[O:3])-[c:5](:[c:4]):[c:6]>>Cl-[C:1](-[C:2])=[O:3].[c:4]:[cH:5]:[c:6]",
        );
        let findings = detect_case_b(&target, &rule);
        assert!(
            !findings.is_empty(),
            "must detect the C2..c6 chain running through the unmatched ring-closing CH2"
        );
        assert_eq!(
            findings[0].case,
            SpectatorBondLossCase::CrossProductTerritory
        );
    }

    #[test]
    fn detects_extracted_109_azepane_cross_product_chain() {
        let target = mol_from_smiles("C1CCCC[C@H](N1)C").unwrap();
        let rule = rr(
            "extracted_109",
            "[C:2]-[CH:1](-[NH:5]-[C:4])-[C:3]>>O=[C:1](-[C:2])-[C:3].[C:4]-[NH2:5]",
        );
        let findings = detect_case_b(&target, &rule);
        assert!(
            !findings.is_empty(),
            "must detect the C3..C4 chain running through the ring's unmatched carbons"
        );
    }

    #[test]
    fn extracted_4255_difluorophthalide_is_not_a_case_b_instance() {
        // The lost bond (map 2 <-> map 6) is a direct edge between two
        // matched atoms -- detect_case_a's territory (see
        // detects_extracted_4255_difluorophthalide_ring_bond above), not
        // this detector's cross-product-via-unmatched-atoms mechanism.
        let target = mol_from_smiles("C2c1cc(c(F)cc1C(O2)=O)F").unwrap();
        let rule = rr(
            "extracted_4255",
            "[C:2]-[O:3]-[CH2:1]-[c:5](:[c:4]):[c:6]>>O=[CH2:1].[C:2]-[OH:3].[c:4]:[cH:5]:[c:6]",
        );
        assert!(
            detect_case_b(&target, &rule).is_empty(),
            "the map2-map6 bond is direct, not routed through an unmatched atom -- detect_case_b \
             must defer to detect_case_a here, not double-report the same defect"
        );
    }

    // ── detect_case_b: negative controls ────────────────────────────────

    #[test]
    fn co_aliphatic_cleavage_is_not_flagged_by_case_b() {
        let target = mol_from_smiles("O=C(OC1CCCNC1)N").unwrap();
        let rule = rr("co_aliphatic_cleavage", "[C:1][O:2]>>[C:1].[O:2]");
        assert!(detect_case_b(&target, &rule).is_empty());
    }

    #[test]
    fn cc_single_cleavage_is_not_flagged_by_case_b() {
        let target = mol_from_smiles("C1CC[C@H](C)NC[C@@H]1C").unwrap();
        let rule = rr("cc_single_cleavage", "[C:1][C:2]>>[C:1].[C:2]");
        assert!(detect_case_b(&target, &rule).is_empty());
    }

    #[test]
    fn ordinary_intermolecular_amide_formation_is_not_flagged_by_case_b() {
        let target = mol_from_smiles("CC(=O)NC").unwrap();
        let rule = rr(
            "amide_formation_retro",
            "[C:1](=[O:2])-[N:3]>>[C:1](=[O:2])[OH].[N:3]",
        );
        assert!(detect_case_b(&target, &rule).is_empty());
    }

    #[test]
    fn ordinary_reductive_amination_is_not_flagged_by_case_b() {
        let target = mol_from_smiles("CC(C)NC").unwrap();
        let rule = rr(
            "reductive_amination_retro",
            "[C:1](-[C:4])(-[C:5])-[N:2]-[C:3]>>O=[C:1](-[C:4])-[C:5].[N:2]-[C:3]",
        );
        assert!(detect_case_b(&target, &rule).is_empty());
    }

    #[test]
    fn legitimate_ring_opening_is_not_flagged_by_case_b() {
        let target = mol_from_smiles("OC1CCCCC1").unwrap();
        let rule = rr("ring_open_retro", "[C:1][C:2]>>[C:1].[C:2]");
        assert!(detect_case_b(&target, &rule).is_empty());
    }

    #[test]
    fn unrelated_distant_ring_is_not_flagged_by_case_b() {
        let target = mol_from_smiles("c1ccccc1CC(=O)NC").unwrap();
        let rule = rr(
            "amide_formation_retro",
            "[C:1](=[O:2])-[N:3]>>[C:1](=[O:2])[OH].[N:3]",
        );
        assert!(detect_case_b(&target, &rule).is_empty());
    }

    #[test]
    fn suzuki_style_two_fragment_disconnection_is_not_flagged_by_case_b() {
        // Biphenyl's two rings are directly bonded (Case A's territory, and
        // even that isn't flagged since the direct bond IS declared) -- no
        // unmatched-only chain exists between them at all here.
        let target = mol_from_smiles("c1ccc(cc1)-c1ccccc1").unwrap();
        let rule = rr("suzuki_retro_smirks", "[c:1]-[c:2]>>[c:1].[c:2]");
        assert!(detect_case_b(&target, &rule).is_empty());
    }

    #[test]
    fn single_product_rhs_is_never_flagged_by_case_b() {
        // No second product fragment to cross into -- must short-circuit,
        // not just happen to find nothing.
        let target = mol_from_smiles("OC1CCCCC1").unwrap();
        let rule = rr("dehydration_retro", "[C:1][O:2]>>[C:1][O:2]");
        assert!(detect_case_b(&target, &rule).is_empty());
    }

    // ── gate_candidates ──────────────────────────────────────────────────

    fn first_match_signature(rule: &RetroRule, target: &Molecule) -> Vec<String> {
        let matches = find_reaction_matches(&rule.smirks, &[target]).unwrap();
        replay_match_signature(&rule.smirks, target, &matches[0]).unwrap()
    }

    #[test]
    fn gate_candidates_rejects_extracted_824() {
        let target = mol_from_smiles("O=C2NCC(O2)Cc1ccccc1").unwrap();
        let rule = rr(
            "extracted_824",
            "[C:5]-[O:6]-[C:3](=[O:4])-[NH:2]-[C:1]>>[C:1]-[N:2]=[C:3]=[O:4].[C:5]-[OH:6]",
        );
        let sig = first_match_signature(&rule, &target);
        let verdicts = gate_candidates(&target, &rule, &[sig]);
        assert!(
            matches!(verdicts[0], SpectatorBondGateVerdict::Rejected { .. }),
            "{:?}",
            verdicts[0]
        );
    }

    #[test]
    fn gate_candidates_rejects_extracted_109() {
        let target = mol_from_smiles("C1CCCC[C@H](N1)C").unwrap();
        let rule = rr(
            "extracted_109",
            "[C:2]-[CH:1](-[NH:5]-[C:4])-[C:3]>>O=[C:1](-[C:2])-[C:3].[C:4]-[NH2:5]",
        );
        let sig = first_match_signature(&rule, &target);
        let verdicts = gate_candidates(&target, &rule, &[sig]);
        assert!(
            matches!(verdicts[0], SpectatorBondGateVerdict::Rejected { .. }),
            "{:?}",
            verdicts[0]
        );
    }

    #[test]
    fn gate_candidates_rejects_extracted_112() {
        let target = mol_from_smiles("c2ccc1CCC(c1c2)=O").unwrap();
        let rule = rr(
            "extracted_112",
            "[C:2]-[C:1](=[O:3])-[c:5](:[c:4]):[c:6]>>Cl-[C:1](-[C:2])=[O:3].[c:4]:[cH:5]:[c:6]",
        );
        let sig = first_match_signature(&rule, &target);
        let verdicts = gate_candidates(&target, &rule, &[sig]);
        assert!(
            matches!(verdicts[0], SpectatorBondGateVerdict::Rejected { .. }),
            "{:?}",
            verdicts[0]
        );
    }

    #[test]
    fn gate_candidates_rejects_extracted_4255() {
        let target = mol_from_smiles("C2c1cc(c(F)cc1C(O2)=O)F").unwrap();
        let rule = rr(
            "extracted_4255",
            "[C:2]-[O:3]-[CH2:1]-[c:5](:[c:4]):[c:6]>>O=[CH2:1].[C:2]-[OH:3].[c:4]:[cH:5]:[c:6]",
        );
        let sig = first_match_signature(&rule, &target);
        let verdicts = gate_candidates(&target, &rule, &[sig]);
        assert!(
            matches!(verdicts[0], SpectatorBondGateVerdict::Rejected { .. }),
            "{:?}",
            verdicts[0]
        );
    }

    #[test]
    fn gate_candidates_accepts_negative_controls() {
        let cases = [
            (
                "O=C(OC1CCCNC1)N",
                rr("co_aliphatic_cleavage", "[C:1][O:2]>>[C:1].[O:2]"),
            ),
            (
                "C1CC[C@H](C)NC[C@@H]1C",
                rr("cc_single_cleavage", "[C:1][C:2]>>[C:1].[C:2]"),
            ),
            (
                "CC(=O)NC",
                rr(
                    "amide_formation_retro",
                    "[C:1](=[O:2])-[N:3]>>[C:1](=[O:2])[OH].[N:3]",
                ),
            ),
            (
                "OC1CCCCC1",
                rr("ring_open_retro", "[C:1][C:2]>>[C:1].[C:2]"),
            ),
        ];
        for (smiles, rule) in cases {
            let target = mol_from_smiles(smiles).unwrap();
            let sig = first_match_signature(&rule, &target);
            let verdicts = gate_candidates(&target, &rule, &[sig]);
            assert!(
                matches!(verdicts[0], SpectatorBondGateVerdict::Accepted),
                "{} / {}: {:?}",
                rule.name,
                smiles,
                verdicts[0]
            );
        }
    }

    #[test]
    fn gate_candidates_not_evaluable_for_hash_atom_wildcard_rule() {
        let target = mol_from_smiles("CCO").unwrap();
        let rule = rr("hash_rule", "[#6:1]-[#8:2]>>[#6:1].[#8:2]");
        let verdicts = gate_candidates(
            &target,
            &rule,
            &[vec!["CC".to_string()], vec!["O".to_string()]],
        );
        assert_eq!(verdicts.len(), 2);
        for v in &verdicts {
            assert!(
                matches!(
                    v,
                    SpectatorBondGateVerdict::NotEvaluable {
                        reason: "hash_atom_wildcard_rule"
                    }
                ),
                "{v:?}"
            );
        }
    }

    #[test]
    fn gate_candidates_not_evaluable_when_lhs_unparseable() {
        // A rule whose LHS can't be read as plain SMILES must never let
        // Case A's "declared" set silently default to empty for gating --
        // an empty declared set makes every real bond look undeclared.
        let target = mol_from_smiles("CCO").unwrap();
        let rule = rr("garbage_lhs", "not smiles at all !!>>[C:1].[C:2]");
        let verdicts = gate_candidates(&target, &rule, &[vec!["C".to_string()]]);
        assert!(
            matches!(
                verdicts[0],
                SpectatorBondGateVerdict::NotEvaluable {
                    reason: "unparseable_declared_bonds"
                }
            ),
            "{:?}",
            verdicts[0]
        );
    }

    #[test]
    fn gate_candidates_not_evaluable_when_signature_does_not_correlate_to_any_match() {
        let target = mol_from_smiles("CCO").unwrap();
        let rule = rr("co_aliphatic_cleavage", "[C:1][O:2]>>[C:1].[O:2]");
        let verdicts = gate_candidates(
            &target,
            &rule,
            &[vec!["totally-unrelated-signature".to_string()]],
        );
        assert!(
            matches!(
                verdicts[0],
                SpectatorBondGateVerdict::NotEvaluable {
                    reason: "match_correlation_failed"
                }
            ),
            "{:?}",
            verdicts[0]
        );
    }

    #[test]
    fn gate_candidates_mixed_outcome_rejects_only_the_defective_candidate() {
        // Same rule (extracted_824's oxazolidinone LHS/RHS), one target
        // with two disconnected components: a real oxazolidinone ring (the
        // known positive control -- C1/C5 really bonded, undeclared) and a
        // plain open-chain carbamate matching the identical 5-atom pattern
        // with no such bond. One rule, one target, matches in both
        // components, opposite verdicts -- proves gating rejects the
        // specific defective candidate and leaves its clean sibling from
        // the same rule alone, per
        // docs/design/spectator-bond-fail-closed-gating-v0.md §7.
        let target = mol_from_smiles("O=C2NCC(O2)Cc1ccccc1.COC(=O)NC").unwrap();
        let rule = rr(
            "extracted_824",
            "[C:5]-[O:6]-[C:3](=[O:4])-[NH:2]-[C:1]>>[C:1]-[N:2]=[C:3]=[O:4].[C:5]-[OH:6]",
        );
        let matches = find_reaction_matches(&rule.smirks, &[&target]).unwrap();
        assert!(
            matches.len() >= 2,
            "expected at least one match in each disconnected component"
        );
        let sigs: Vec<Vec<String>> = matches
            .iter()
            .map(|m| replay_match_signature(&rule.smirks, &target, m).unwrap())
            .collect();

        let verdicts = gate_candidates(&target, &rule, &sigs);
        assert!(
            verdicts
                .iter()
                .any(|v| matches!(v, SpectatorBondGateVerdict::Rejected { .. })),
            "the ring instance must reject: {verdicts:?}"
        );
        assert!(
            verdicts
                .iter()
                .any(|v| matches!(v, SpectatorBondGateVerdict::Accepted)),
            "the open-chain instance must stay accepted: {verdicts:?}"
        );
        assert!(
            !verdicts.iter().any(|v| matches!(
                v,
                SpectatorBondGateVerdict::NotEvaluable {
                    reason: "ambiguous_signature"
                }
            )),
            "the two components' signatures must never collide: {verdicts:?}"
        );
    }
}
