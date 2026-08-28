use std::collections::BinaryHeap;
use std::sync::Arc;

use anyhow::Result;
use chematic::chem::{molecular_weight, sa_score};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use serde::Serialize;
use smallvec::{SmallVec, smallvec};

use crate::chem_env::{
    ChemEnv, RetroRule, TemplateBondIndex, canonical_stock_identity_from_smiles, mol_from_smiles,
    to_canonical,
};
use crate::evidence::{EvidenceScope, MetadataSource, StepEvidence, TemplateMetadataEntry};
use crate::score::{step_cost, template_bonus};
use crate::spectator_bond::SpectatorBondPolicy;
use crate::synthesizability::{ElementAccountingStatus, compute_element_accounting};

/// Cached expansion for one (target_smiles, rule) combination.
struct RetroEntry {
    rule_name: String,
    template_id: String,
    step_cost: f64,
    precursor_smiles: Vec<String>,
}
type RetroCache = FxHashMap<String, Arc<Vec<RetroEntry>>>;

/// Suggested reaction conditions for a synthesis step (rule-based, hand-crafted rules only).
#[derive(Debug, Clone, Serialize)]
pub struct ReactionConditions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalyst: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solvent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Why `ReactionStep::atom_economy` is (or isn't) populated. `AboveExpectedRange`
/// means MW(target) / Σ MW(precursors) computed to more than 100% -- the
/// precursor set RENKIN represents supplies less mass than the target
/// needs. This is **not** proof of target-atom loss on its own: the
/// denominator is only the precursors a template names, not every reactant
/// or reagent the real reaction would use. An omitted reactant or reagent
/// (a leaving-group source, a catalyst, a deprotection's H2) can
/// contribute mass that is absent from the represented precursor set and
/// push this ratio over 100% for a perfectly valid route -- this is not
/// "atoms the reagent never carries": H2, for instance, can very much
/// supply hydrogen to the target. What actually keeps such a case safe is
/// that the independent directional element-accounting check
/// (`synthesizability::element_accounting::compute_element_accounting`)
/// is heavy-element-only (hydrogen excluded by design, see that module's
/// doc comment), so it may still report `Accounted` when the omitted
/// contribution is hydrogen-only -- even though this MW ratio alone
/// can't tell that case apart from genuine atom loss (Issue #79). Earlier
/// behaviour silently clamped this ratio down to 100.0, which looked
/// identical to a genuinely perfect-economy route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomEconomyStatus {
    Normal,
    AboveExpectedRange,
    NotEvaluable,
}

/// Computes the raw (unclamped) MW(target)/Σ MW(precursors)×100 ratio for a
/// step, or `None` if either side isn't cleanly evaluable. All-or-nothing on
/// the precursor side: a single unparseable precursor must not silently
/// shrink the denominator and inflate the ratio (a `filter_map`-based sum
/// would do exactly that). Never returns a non-finite value.
fn compute_atom_economy_raw(target_smiles: &str, precursors: &[String]) -> Option<f64> {
    let target_weight = mol_from_smiles(target_smiles)
        .ok()
        .map(|m| molecular_weight(&m))?;
    let precursor_weights: Vec<f64> = precursors
        .iter()
        .map(|s| mol_from_smiles(s).ok().map(|m| molecular_weight(&m)))
        .collect::<Option<Vec<f64>>>()?;
    let precursor_weight: f64 = precursor_weights.iter().sum();
    if !target_weight.is_finite()
        || !precursor_weight.is_finite()
        || target_weight < 0.0
        || precursor_weight <= 0.0
    {
        return None;
    }
    let ratio = target_weight / precursor_weight * 100.0;
    ratio.is_finite().then_some(ratio)
}

/// Classifies a raw (unclamped) MW(target)/Σ MW(precursors)×100 ratio into
/// (status, display value). `display` is `Some(raw)` only for `Normal` --
/// never a clamped substitute, so a caller can't mistake "not evaluable in
/// the normal sense" for a genuinely perfect route. A non-finite `raw`
/// (NaN/±Infinity) is defensively treated as `NotEvaluable` -- callers are
/// expected to already guard against this (see `find_routes`'s
/// post-processing step), but this pure function must never trust that.
fn classify_atom_economy(raw: Option<f64>) -> (AtomEconomyStatus, Option<f64>) {
    let status = match raw {
        Some(r) if r.is_finite() && r > 100.0 + 1e-6 => AtomEconomyStatus::AboveExpectedRange,
        Some(r) if r.is_finite() => AtomEconomyStatus::Normal,
        _ => AtomEconomyStatus::NotEvaluable,
    };
    let display = match status {
        AtomEconomyStatus::Normal => raw,
        AtomEconomyStatus::AboveExpectedRange | AtomEconomyStatus::NotEvaluable => None,
    };
    (status, display)
}

#[derive(Debug, Clone, Serialize)]
pub struct ReactionStep {
    pub rule: String,
    /// Stable identity of the template used (see `RetroRule::template_id`).
    /// Always populated -- `rule:<name>` for hand-crafted rules,
    /// `smirks-sha256:<hex>` for extracted templates.
    pub template_id: String,
    pub target: String,
    pub precursors: Vec<String>,
    /// Suggested conditions for the forward reaction (None for extracted templates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<ReactionConditions>,
    /// Atom economy: MW(target) / Σ MW(precursors) × 100 — fraction of atoms
    /// retained. `None` whenever `atom_economy_status` isn't `Normal`: never
    /// clamped down to fit an expected range (see `atom_economy_raw_percent`
    /// for the unclamped ratio, and `atom_economy_status` for why).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atom_economy: Option<f64>,
    /// The unclamped MW(target) / Σ MW(precursors) × 100 ratio, populated
    /// whenever both molecular weights are computable regardless of
    /// `atom_economy_status` -- the honest number `atom_economy` is derived
    /// from, kept even when that ratio exceeds the expected [0, 100] range
    /// under the represented-precursor convention (see `AtomEconomyStatus`
    /// for why exceeding it isn't proof of anything on its own).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atom_economy_raw_percent: Option<f64>,
    /// See `AtomEconomyStatus`.
    pub atom_economy_status: AtomEconomyStatus,
    /// Per-step template confidence: rule_weight / max_rule_weight ∈ [0, 1].
    /// Hand-crafted rules (weight=1.0) yield lower values when high-frequency extracted
    /// templates are present; all weights equal → all step_confidence = 1.0.
    pub step_confidence: f64,
    /// Suggested experimental procedure hint for the forward reaction.
    /// Populated for hand-crafted rules; None for extracted templates.
    /// Placeholder for QFANG-style structured procedure generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure_hint: Option<String>,
    /// Reaction family for this step (e.g. "suzuki_coupling", "esterification").
    /// None for extracted templates that have no manual assignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_family: Option<String>,
    /// Provenance of `conditions`/`reaction_family`. `None` for extracted templates --
    /// nothing is fabricated for them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_source: Option<MetadataSource>,
    /// Scope at which `metadata_source` was assigned. `None` for extracted templates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_scope: Option<EvidenceScope>,
    /// Curated external evidence (conditions/yields/warnings/references) matched
    /// by `template_id` from an optional metadata sidecar. `None` unless a
    /// sidecar was supplied and matched -- nothing is fabricated for templates
    /// without an entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<StepEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Route {
    pub steps: Vec<ReactionStep>,
    pub depth: u32,
    /// Cumulative A* step cost (lower = better). Included in JSON output.
    pub score: f64,
    /// Leaf building blocks for this route (precursors not expanded further).
    pub building_blocks: Vec<String>,
    /// Template confidence: min(step template frequency) / max frequency in rule set.
    /// 0 = route uses very rare templates; 1 = all templates are maximally common.
    pub confidence: f64,
    /// Convergency score: 1.0 = all branches same depth (parallel synthesis possible);
    /// 0.0 = purely linear route.
    pub convergency: f64,
    /// Product of step_confidence values (Retro-prob style): a
    /// frequency-derived route ranking score, not a calibrated experimental
    /// success probability -- decays with route length purely because rarer
    /// templates compound, not because of any measured or predicted failure
    /// rate. Single-step: equals step_confidence. Multi-step: decays
    /// multiplicatively.
    pub success_probability: f64,
    /// Estimated synthesis cost: Σ(BB complexity or price) + step_count × 0.5.
    /// Uses SA Score as complexity proxy when no price file is provided.
    /// Lower = cheaper / simpler route.
    pub route_cost: f64,
}

/// Why a just-completed candidate was rejected at the acceptance boundary
/// (RENKIN Bridge PR1, `fix(routes): reject structurally invalid completed
/// routes`) -- the last check before a candidate becomes a returned
/// [`Route`]. A rejected candidate is discarded, never surfaced; the search
/// continues from the same node exactly as it would have otherwise (see
/// `find_routes_with_control`'s frontier loop, which falls through to
/// further expansion regardless of whether this push happened). Stock-leaf
/// membership is deliberately NOT re-checked here: `n_unsolved == 0` (this
/// gate's only call site) already re-verified every frontier leaf via
/// `is_bb` under the same `env` in the same call, so re-checking it would
/// be tautological, not a real defense.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteIntegrityDefect {
    /// The reconstructed root step's target doesn't canonicalize to the
    /// searched-for target.
    RootMismatch,
    /// A step's target or precursor SMILES fails to parse.
    UnparseableSmiles,
    /// A step recorded zero precursors -- a malformed/degenerate expansion.
    EmptyPrecursorList,
    /// A molecule reappears as its own descendant's precursor when the
    /// flat step list is reconstructed into a tree (target/precursor
    /// string matching, the same convention `display::build_tree` and
    /// `extract_building_blocks` already use). Not prevented by
    /// construction: `find_routes_with_control`'s `closed` set dedupes
    /// whole-frontier states, not per-molecule path membership.
    Cycle,
    /// A step's target is never reachable from the route's root via that
    /// same tree reconstruction -- would otherwise silently corrupt
    /// `extract_building_blocks`/`display::build_tree`'s output rather
    /// than fail loud.
    Disconnected,
    /// `synthesizability::compute_element_accounting` found a step where
    /// the target needs more of some heavy element than its precursors
    /// collectively supply (Issue #72/L984's real, still-reproducible
    /// failure mode with the default `ring_context_policy = disabled`: an
    /// extracted template mis-disconnects a ring bond and drops a whole
    /// ring-fused fragment).
    UnaccountedTargetElement,
}

/// Acceptance-boundary rejections of structurally invalid completed routes
/// (RENKIN Bridge PR1). Always accumulated -- the checks are cheap and run
/// once per completed candidate, not per search node -- so callers can tell
/// "genuinely unsolved" apart from "every candidate found was structurally
/// broken and discarded" even without `--search-diagnostics`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RouteIntegrityDiagnostics {
    pub root_mismatch: u64,
    pub unparseable_smiles: u64,
    pub empty_precursor_list: u64,
    pub cycle: u64,
    pub disconnected: u64,
    pub unaccounted_target_element: u64,
    /// Completed candidates discarded for at least one of the above
    /// reasons (a candidate with multiple defects still counts once here).
    pub routes_rejected: u64,
}

impl RouteIntegrityDiagnostics {
    fn record(&mut self, defects: &[RouteIntegrityDefect]) {
        if defects.is_empty() {
            return;
        }
        self.routes_rejected += 1;
        for defect in defects {
            match defect {
                RouteIntegrityDefect::RootMismatch => self.root_mismatch += 1,
                RouteIntegrityDefect::UnparseableSmiles => self.unparseable_smiles += 1,
                RouteIntegrityDefect::EmptyPrecursorList => self.empty_precursor_list += 1,
                RouteIntegrityDefect::Cycle => self.cycle += 1,
                RouteIntegrityDefect::Disconnected => self.disconnected += 1,
                RouteIntegrityDefect::UnaccountedTargetElement => {
                    self.unaccounted_target_element += 1
                }
            }
        }
    }
}

/// Walks the route's implicit tree (target/precursor string matching, same
/// reconstruction convention as `display::build_tree`) from `node`,
/// recording every reachable target in `visited` and flagging `has_cycle`
/// if `node` reappears among its own ancestors on this path.
fn walk_route_tree<'a>(
    step_map: &FxHashMap<&'a str, &'a [String]>,
    node: &'a str,
    visited: &mut FxHashSet<&'a str>,
    on_path: &mut FxHashSet<&'a str>,
    has_cycle: &mut bool,
) {
    if on_path.contains(node) {
        *has_cycle = true;
        return;
    }
    if !visited.insert(node) {
        // Already fully explored via another branch (a shared leaf/BB
        // reused by two steps) -- not a cycle.
        return;
    }
    on_path.insert(node);
    if let Some(precursors) = step_map.get(node) {
        for p in *precursors {
            walk_route_tree(step_map, p.as_str(), visited, on_path, has_cycle);
        }
    }
    on_path.remove(node);
}

/// Acceptance-boundary integrity check for a just-completed candidate route
/// (RENKIN Bridge PR1). `target_canonical` is the search's own root target,
/// already canonicalized once at the top of `find_routes_with_control`.
/// A depth-0 route (`route.steps` empty -- the target itself is already a
/// stock leaf) has nothing to validate structurally and always passes.
fn route_integrity_defects(route: &Route, target_canonical: &str) -> Vec<RouteIntegrityDefect> {
    let mut defects = Vec::new();

    if route.steps.is_empty() {
        return defects;
    }

    match mol_from_smiles(&route.steps[0].target) {
        Ok(m) if to_canonical(&m) == target_canonical => {}
        _ => defects.push(RouteIntegrityDefect::RootMismatch),
    }

    let mut any_unparseable = false;
    for step in &route.steps {
        if mol_from_smiles(&step.target).is_err() {
            any_unparseable = true;
        }
        if step.precursors.is_empty() {
            defects.push(RouteIntegrityDefect::EmptyPrecursorList);
        }
        for p in &step.precursors {
            if mol_from_smiles(p).is_err() {
                any_unparseable = true;
            }
        }
    }
    if any_unparseable {
        defects.push(RouteIntegrityDefect::UnparseableSmiles);
    }

    let step_map: FxHashMap<&str, &[String]> = route
        .steps
        .iter()
        .map(|s| (s.target.as_str(), s.precursors.as_slice()))
        .collect();
    let mut visited: FxHashSet<&str> = FxHashSet::default();
    let mut on_path: FxHashSet<&str> = FxHashSet::default();
    let mut has_cycle = false;
    walk_route_tree(
        &step_map,
        route.steps[0].target.as_str(),
        &mut visited,
        &mut on_path,
        &mut has_cycle,
    );
    if has_cycle {
        defects.push(RouteIntegrityDefect::Cycle);
    }
    // Every step's target must actually have been reached walking down
    // from the root -- `visited` also contains leaf precursors that are
    // never step targets themselves, so this can't be a length comparison.
    if step_map.keys().any(|target| !visited.contains(target)) {
        defects.push(RouteIntegrityDefect::Disconnected);
    }

    if compute_element_accounting(route).status == ElementAccountingStatus::UnaccountedTargetElement
    {
        defects.push(RouteIntegrityDefect::UnaccountedTargetElement);
    }

    defects
}

/// Statistics returned alongside routes from [`find_routes`].
#[derive(Debug, Default, Serialize)]
pub struct SearchStats {
    pub nodes_expanded: u64,
    pub max_depth_reached: bool,
    pub beam_limit_hit: bool,
    /// Total template-molecule matches across all expansions.
    pub matched_templates: u64,
    /// Total building-block hits seen in node frontiers.
    pub stock_hits: u64,
    /// retro_cache hits (same intermediate seen before → O(1) reuse).
    pub retro_cache_hits: u64,
    /// retro_cache misses (new intermediate → full apply_retro run).
    pub retro_cache_misses: u64,
    /// Ring-context safety guard counters (Issue #72), accumulated across
    /// every extracted-template application in this search. All-zero
    /// unless `SearchConfig::ring_context_policy` is not `Disabled`.
    pub ring_context_diagnostics: crate::ring_context::RingContextDiagnostics,
    /// Acceptance-boundary rejections of structurally invalid completed
    /// routes (RENKIN Bridge PR1). Always accumulated, independent of
    /// `--search-diagnostics`.
    pub route_integrity: RouteIntegrityDiagnostics,
    /// Beam/crowd-out diagnostics (Issue #101). Always accumulated (the
    /// bookkeeping cost is a handful of integer ops at points the search
    /// loop already visits); the CLI only *surfaces* this behind
    /// `--search-diagnostics` so default JSON output is unchanged.
    pub crowd_out: CrowdOutDiagnostics,
    /// Issue #101 Task 35: number of expansions where the configured
    /// reranker failed (model/table already loaded, but `score_pool`
    /// erred on this pool) and this search fell back to legacy ordering
    /// for the remainder of the run. Always `0` when `SearchConfig::reranker`
    /// is `None`. Expected to be `0` even when a reranker is configured --
    /// a nonzero value means a mixed-mode run (part reranked, part legacy)
    /// happened and should be investigated, not silently accepted.
    pub reranker_failures: u64,
}

/// Human-readable "why no route was found" diagnosis for a zero-route
/// search: likely causes plus actionable suggestions, derived from
/// `stats`/`max_depth`. Shared by the `renkin` CLI and the `find_routes`
/// Python binding's empty-route JSON output -- moved here (from the CLI
/// binary, where it originated) so both can call it, since `src/python.rs`
/// is part of this library crate and cannot reach a function defined only
/// in the `renkin` binary crate.
pub fn diagnose(stats: &SearchStats, max_depth: u32) -> (Vec<&'static str>, Vec<String>) {
    let mut causes: Vec<&'static str> = Vec::new();
    let mut suggestions: Vec<String> = Vec::new();
    if stats.stock_hits == 0 {
        causes.push("no matching building block in stock");
        suggestions.push("add a custom stock file with --building-blocks".to_string());
    }
    if stats.max_depth_reached {
        causes.push("search depth exhausted");
        suggestions.push(format!("try --depth {}", max_depth + 2));
    }
    if stats.beam_limit_hit {
        causes.push("beam width too narrow — candidates were pruned");
        suggestions.push("try --beam-width 200".to_string());
    }
    if stats.matched_templates < 5 {
        causes.push("few or no templates matched the target");
        suggestions.push("try --templates data/templates_extracted_50000.smi".to_string());
    }
    if stats.route_integrity.routes_rejected > 0 {
        causes.push(
            "completed candidate route(s) failed the structural-integrity check and were discarded",
        );
        suggestions.push(format!(
            "{} candidate route(s) were rejected (unaccounted_target_element={}, cycle={}, disconnected={}, unparseable_smiles={}, empty_precursor_list={}, root_mismatch={})",
            stats.route_integrity.routes_rejected,
            stats.route_integrity.unaccounted_target_element,
            stats.route_integrity.cycle,
            stats.route_integrity.disconnected,
            stats.route_integrity.unparseable_smiles,
            stats.route_integrity.empty_precursor_list,
            stats.route_integrity.root_mismatch,
        ));
    }
    (causes, suggestions)
}

/// Diagnostics-only counters (Issue #101): computing these does not change
/// which candidates are expanded, scored, kept, or in what order --- only
/// bookkeeping is added at points [`find_routes`] already visits. Not used
/// by ranking or pruning itself.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CrowdOutDiagnostics {
    /// Number of times [`beam_prune`] actually truncated the open-node heap.
    pub beam_prune_invocations: u64,
    /// Total nodes evicted across all `beam_prune` invocations.
    pub candidates_evicted_total: u64,
    /// Lowest f() = g + h (best/most-promising, since lower is better)
    /// among all ever-evicted nodes, aggregated across every `beam_prune`
    /// call in this search. `None` if nothing was ever evicted. Because g
    /// grows with depth, this running minimum usually comes from an early
    /// (shallow, cheap) prune -- it is *not* comparable to
    /// `final_beam_boundary_f` (a single later invocation) to judge "how
    /// close" a miss was; use `beam_prune`'s per-call return value for that
    /// within one invocation.
    pub evicted_f_min: Option<f64>,
    /// Highest (worst) f() among all ever-evicted nodes, same
    /// cross-invocation aggregation as `evicted_f_min`.
    pub evicted_f_max: Option<f64>,
    /// f() of the worst node still retained after the *final* `beam_prune`
    /// call in this search -- the beam's closing cutoff score, one number
    /// from one invocation (not an aggregate).
    pub final_beam_boundary_f: Option<f64>,
    /// Sum of `active_rules.len()` across every retro-cache-miss expansion
    /// (unique intermediates only -- a cache hit reuses a prior expansion's
    /// proposals without re-attempting any rule). Counts *RetroRule entries
    /// attempted*, not concrete SMIRKS applications: a single hash-atom
    /// `RetroRule` can internally try several `[#N]`-variant SMIRKS strings
    /// inside `apply_retro` (see `chem_env::expand_hash_atom_variants`), so
    /// this is a lower bound on raw SMIRKS-match attempts, and must not be
    /// read as a logical-template count (already-expanded variants are
    /// separate `RetroRule` entries in `rules`, same as `matched_templates`).
    pub rules_attempted_total: u64,
    /// Cumulative wall-clock microseconds spent inside the retro-cache-miss
    /// expansion block (rule matching/candidate generation via
    /// `candidate::raw_propose`, plus its NN-reranking and dedup-counting
    /// overhead) -- i.e. the same "unique intermediates only" population as
    /// [`Self::rules_attempted_total`], but timed instead of counted.
    /// Diagnostic instrument for Issue #128 (search dramatically slower
    /// per-node than the Phase 31 baseline, root cause unconfirmed): that
    /// issue's own bisection plan needed a wall-clock-noise-resistant signal
    /// -- comparing two full runs' total elapsed time on a shared,
    /// non-dedicated machine was already tried and found too noisy to
    /// isolate a cause (see the issue's own comment). This field instead
    /// answers "what fraction of one run's own total time is spent in
    /// rule-matching/candidate-generation" from a *single* run, which
    /// doesn't depend on comparing absolute durations across runs. `0`
    /// unless [`SearchConfig::timing_diagnostics`] is `true` -- see that
    /// field's own doc for why this one specific field must stay opt-in
    /// (wall-clock timing is inherently non-deterministic, unlike every
    /// other field here). Also always `0` on `wasm32` regardless of that
    /// flag (`std::time::Instant::now()` is unavailable there, matching
    /// this function's own existing `#[cfg(not(target_arch = "wasm32"))]`
    /// `t0`/`nodes_popped` timing, which has the same restriction).
    pub retro_expansion_wall_time_us: u64,
    /// Every [`crate::spectator_bond::SpectatorBondLossFinding`] detected
    /// across every retro-cache-miss expansion in this search -- always
    /// empty unless [`SearchConfig::spectator_bond_policy`] is
    /// `DiagnosticsOnly` or `Gated` (see that field's own doc for why this
    /// one, like `retro_expansion_wall_time_us`, must stay opt-in rather
    /// than unconditional like this struct's pure-count fields). Recorded
    /// identically under both policies -- policy only changes whether a
    /// confident finding also excludes its candidate (see
    /// [`Self::spectator_bond_gated_out`]), never which findings are
    /// detected.
    pub spectator_bond_loss_findings: Vec<crate::spectator_bond::SpectatorBondLossFinding>,
    /// Every candidate [`SearchConfig::spectator_bond_policy`]'s `Gated`
    /// setting actually excluded from the search, with the finding(s) that
    /// justified it -- always empty unless the policy is `Gated`. An
    /// exclusion is never silent: this is its trail even though the
    /// candidate itself no longer appears anywhere else in this run.
    pub spectator_bond_gated_out: Vec<crate::spectator_bond::GatedCandidateRecord>,
    /// Diversity-reserved beam (Issue #101,
    /// `docs/design/diversity-reserved-beam-v0.md`), accumulated across
    /// every `beam_prune` call in this search -- always default/empty
    /// unless [`SearchConfig::beam_diversity_policy`] is `DiagnosticsOnly`
    /// or `Active`. An independent axis from `spectator_bond_gated_out`
    /// above, never conflated with it.
    pub beam_diversity_stats: DiversityReservationStats,
    /// Same-parent proposals from *different* templates whose precursor
    /// SMILES multiset (sorted) is identical -- these push distinct heap
    /// nodes that would collapse to the same downstream synthesis state.
    /// Diagnostics-only: this count does not merge or drop any node.
    pub cross_template_duplicate_precursor_signatures: u64,
    /// Frontier entries confirmed in stock, counted the same way as
    /// [`SearchStats::stock_hits`] (kept alongside `non_stock_candidates`
    /// so the terminal/open ratio doesn't need re-deriving elsewhere).
    pub stock_terminal_candidates: u64,
    /// Frontier entries NOT in stock -- still-open synthesis lines.
    pub non_stock_candidates: u64,
    /// Depth → (nodes expanded at that depth, children produced from them).
    /// `children_produced / nodes_expanded` at a given depth is that
    /// depth's mean branching factor. `BTreeMap` for deterministic
    /// (depth-ordered) JSON output.
    pub branching_by_depth: std::collections::BTreeMap<u32, DepthBranching>,
    /// Sum of raw proposals across every expansion, before any collapsing
    /// by precursor-set identity. Equal to [`SearchStats::matched_templates`]
    /// (same underlying count, named here for self-containment within this
    /// diagnostics block) -- included so a reader doesn't have to
    /// cross-reference the parent struct to get the "before" side of the
    /// two dedup counts below.
    pub candidates_generated_before_dedup: u64,
    /// Sum, across every expansion, of the number of *distinct*
    /// `(template_id, sorted precursor multiset)` pairs among that
    /// expansion's proposals -- i.e. what would remain if only exact
    /// same-template repeats (e.g. a symmetric template matching two
    /// equivalent sites and re-deriving the same precursor set) collapsed.
    /// Diagnostics-only: nothing is actually collapsed; `find_routes` still
    /// pushes one heap node per raw proposal.
    pub candidates_after_same_template_dedup: u64,
    /// Sum, across every expansion, of the number of *distinct* sorted
    /// precursor multisets among that expansion's proposals, regardless of
    /// which template produced them -- what would remain under a
    /// same-parent cross-template dedup (Phase 1E candidate #1). Always
    /// `<= candidates_after_same_template_dedup`. Diagnostics-only.
    pub candidates_after_cross_template_dedup: u64,
    /// Per-candidate trace records (competitive-diagnostics program, Phase
    /// 1B), collected only when [`SearchConfig::candidate_trace_cap`] is
    /// `Some` -- empty by default, zero collection cost when the cap is
    /// `None`. Bounded by that cap, filled in first-generated order
    /// (deterministic, not sampled).
    pub candidate_trace: Vec<CandidateTraceRecord>,
}

/// One depth's branching-factor accumulator; see
/// [`CrowdOutDiagnostics::branching_by_depth`].
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DepthBranching {
    pub nodes_expanded: u64,
    pub children_produced: u64,
}

/// Where a template originated (competitive-diagnostics program, Phase 1B).
/// Derived read-only from `RetroRule::template_id`/`smirks`; does not
/// require any change to rule loading or application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateProvenance {
    /// `template_id` starts with `rule:` -- a hand-crafted rule.
    Handcrafted,
    /// Extracted template (`smirks-sha256:` id) whose SMIRKS contains no
    /// bare atomic-number (`[#N]`) primitive -- applied directly, no
    /// variant expansion needed.
    FileBacked,
    /// Extracted template whose SMIRKS contains a bare atomic-number
    /// primitive (`[#N]`) -- applied via `chem_env::application_smirks_variants`
    /// (Issue #88/#89/#91 hash-atom fix).
    HashAtom,
}

/// One candidate node's trace record (competitive-diagnostics program,
/// Phase 1B): what it was, where it ranked at each beam-prune it was
/// subject to, and whether it ultimately contributed to a returned route.
/// Collected only under [`SearchConfig::candidate_trace_cap`]; never
/// affects which candidates are expanded, scored, kept, or in what order.
#[derive(Debug, Clone, Serialize)]
pub struct CandidateTraceRecord {
    /// Depth this candidate sits at (its parent node's depth + 1).
    pub depth: u32,
    /// Canonical SMILES of the intermediate this candidate was proposed from.
    pub parent_smiles: String,
    pub template_id: String,
    pub rule_name: String,
    pub provenance: CandidateProvenance,
    /// Sorted precursor SMILES multiset -- the identity used for the dedup
    /// counts above and for `later_reached_stock` matching.
    pub precursor_signature: Vec<String>,
    /// f() = g + h at creation -- the score `beam_prune` ranks on.
    pub f_score: f64,
    /// This candidate's 0-based rank (ascending f()) at the last
    /// `beam_prune` call that actually truncated the heap while this
    /// candidate was present. `None` if `beam_width == 0`, the search ended
    /// before any `beam_prune` call processed it, or every `beam_prune`
    /// call it was subject to found the heap already within `beam_width`
    /// (no sort happens in that case, so there is no real rank to report).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_before_prune: Option<usize>,
    /// `true` unless a `beam_prune` call evicted this candidate (i.e. its
    /// rank at that call was >= the beam width). Starts `true` at creation.
    pub survived_beam: bool,
    /// Set in post-processing: `true` iff this exact
    /// `(parent_smiles, template_id, precursor_signature)` step appears in
    /// one of the routes this search ultimately returned.
    pub later_reached_stock: bool,
}

fn extract_building_blocks(steps: &[ReactionStep]) -> Vec<String> {
    let targets: std::collections::HashSet<&str> =
        steps.iter().map(|s| s.target.as_str()).collect();
    let mut bbs: Vec<String> = steps
        .iter()
        .flat_map(|s| s.precursors.iter())
        .filter(|p| !targets.contains(p.as_str()))
        .cloned()
        .collect();
    bbs.sort_unstable();
    bbs.dedup();
    bbs
}

#[derive(Debug, Clone)]
struct FEntry {
    smiles: String,
}

/// Persistent linked-list node for synthesis path sharing.
/// Children share the parent's prefix via Arc::clone (pointer copy only).
#[derive(Debug, Clone)]
struct PathNode {
    step: ReactionStep,
    prev: Option<Arc<PathNode>>,
}

fn collect_path(mut cur: Option<&Arc<PathNode>>) -> Vec<ReactionStep> {
    let mut steps = Vec::new();
    while let Some(node) = cur {
        steps.push(node.step.clone());
        cur = node.prev.as_ref();
    }
    steps.reverse();
    steps
}

#[derive(Debug, Clone)]
struct Node {
    frontier: SmallVec<[FEntry; 6]>,
    path: Option<Arc<PathNode>>,
    depth: u32,
    g: f64,
    h: f64,
    /// Index into `crowd_out.candidate_trace`, set only when
    /// `SearchConfig::candidate_trace_cap` is `Some` and the cap hasn't been
    /// reached yet. `None` (the default) costs nothing beyond this field's
    /// own size -- see [`CandidateTraceRecord`].
    trace_id: Option<u64>,
    /// The `template_id` of the rule that produced this node's own step,
    /// i.e. the last edge on `path` -- `None` for the root (no step) or any
    /// other node with no well-defined rule identity of its own (see
    /// `docs/design/diversity-reserved-beam-v0.md` §3: such nodes are never
    /// eligible for family-diversity beam-slot reservation, only for the
    /// pure-score portion). Populated once at push time from the same
    /// `entry.template_id` `CandidateTraceRecord` already reads, at no
    /// extra per-node cost. Read by `select_beam_survivors`, called from
    /// `beam_prune` whenever `SearchConfig::beam_diversity_policy != Off`.
    family_key: Option<String>,
}

impl Node {
    fn f(&self) -> f64 {
        self.g + self.h
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.f().to_bits() == other.f().to_bits()
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap by f = g + h (best = lowest cost first).
        other
            .f()
            .partial_cmp(&self.f())
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// Build a bitmask of atomic numbers present in a canonical SMILES string.
/// Conservative: may over-report (false positives) but never under-reports (no false negatives).
/// Used to skip rules whose required elements are absent from the target molecule.
pub(crate) fn elem_mask_from_smiles(smiles: &str) -> u64 {
    const TWO_CHAR: &[(&str, u64)] = &[
        ("Cl", 17),
        ("Br", 35),
        ("Si", 14),
        ("Se", 34),
        ("Te", 52),
        ("Sn", 50),
        ("Zn", 30),
        ("Pd", 46),
        ("Cu", 29),
        ("Fe", 26),
    ];
    const ONE_CHAR: &[(char, u64)] = &[
        ('B', 5),
        ('C', 6),
        ('N', 7),
        ('O', 8),
        ('F', 9),
        ('P', 15),
        ('S', 16),
        ('I', 53),
    ];
    let mut mask: u64 = 0;
    for (sym, an) in TWO_CHAR {
        if smiles.contains(*sym) {
            mask |= 1u64 << an;
        }
    }
    for (ch, an) in ONE_CHAR {
        let lo = ch.to_ascii_lowercase();
        if smiles.chars().any(|c| c == *ch || c == lo) {
            mask |= 1u64 << an;
        }
    }
    mask
}

/// Hash the sorted frontier SMILES into a u64 for closed-set deduplication.
/// Avoids String allocation per node vs. the former join-based state_key.
/// Collision probability is 2^-64 per node pair — negligible in practice.
fn state_hash(frontier: &[FEntry]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut keys: Vec<&str> = frontier.iter().map(|e| e.smiles.as_str()).collect();
    keys.sort_unstable();
    let mut h = FxHasher::default();
    for k in &keys {
        k.hash(&mut h);
    }
    h.finish()
}

fn is_bb(smiles: &str, env: &ChemEnv) -> bool {
    // Fast path: direct HashSet lookup (FEntry.smiles is always canonical SMILES).
    if env.is_building_block_smiles(smiles) {
        return true;
    }
    // Slow path: re-parse and re-standardize under the same shared
    // stock-identity policy `ChemEnv` itself uses. Exact identity only (no
    // subgraph matching) — see `chem_env::canonical_stock_identity`.
    canonical_stock_identity_from_smiles(smiles)
        .map(|canon| env.is_building_block_smiles(&canon))
        .unwrap_or(false)
}

/// Pluggable molecule value estimator for the A* heuristic (Retro*-style).
///
/// Returns the estimated synthesis cost for a SMILES string (≥ 0.0; higher = harder).
/// The default implementation uses SA Score. Implement this trait to plug in a neural
/// value function without changing the search algorithm.
pub trait MoleculeValueEstimator: Send + Sync {
    fn estimate_cost(&self, smiles: &str) -> f64;
}

/// Default estimator: SA Score-based heuristic (h ∈ [1.0, 1.5] per unsolved molecule).
/// Admissible because step_cost ≥ 1.0 per step, so h ≤ 1.5 < true cost.
pub struct SaScoreEstimator;

impl MoleculeValueEstimator for SaScoreEstimator {
    fn estimate_cost(&self, smiles: &str) -> f64 {
        let v = mol_from_smiles(smiles)
            .map(|m| sa_score(&m).clamp(1.0, 10.0))
            .unwrap_or(5.5);
        1.0 + 0.5 * (v - 1.0) / 9.0
    }
}

/// Pluggable template prior for A* expansion scoring (Retro*-style).
///
/// Returns a bonus ≥ 0.0: how relevant `template_name` is for expanding `target_smiles`.
/// Higher bonus → smaller effective step cost → template is tried earlier in A\* search.
/// The default implementation (`FrequencyPrior`) uses log-frequency from training data.
pub trait ReactionPrior: Send + Sync {
    fn prior(&self, template_name: &str, target_smiles: &str) -> f64;
}

/// Default prior: log-frequency weight from USPTO training data (same as pre-v0.9 behavior).
///
/// `weight = ln(count + 1)` for extracted templates; hand-crafted rules use `weight = 1.0`.
/// The bonus is `template_bonus(weight, max_weight)` ∈ [0.0, 0.2].
pub struct FrequencyPrior {
    pub rule_weights: std::collections::HashMap<String, f64>,
    pub max_weight: f64,
}

impl FrequencyPrior {
    pub fn from_rules(rules: &[RetroRule]) -> Self {
        let max_weight = rules.iter().map(|r| r.weight).fold(1.0_f64, f64::max);
        let rule_weights = rules.iter().map(|r| (r.name.clone(), r.weight)).collect();
        Self {
            rule_weights,
            max_weight,
        }
    }
}

impl ReactionPrior for FrequencyPrior {
    fn prior(&self, template_name: &str, _target_smiles: &str) -> f64 {
        let w = self.rule_weights.get(template_name).copied().unwrap_or(1.0);
        template_bonus(w, self.max_weight)
    }
}

fn compute_h(
    frontier: &[FEntry],
    env: &ChemEnv,
    sa_cache: &mut FxHashMap<String, f64>,
    estimator: Option<&std::sync::Arc<dyn MoleculeValueEstimator>>,
) -> f64 {
    frontier
        .iter()
        .filter(|e| !is_bb(&e.smiles, env))
        .map(|e| {
            if let Some(est) = estimator {
                return est.estimate_cost(&e.smiles);
            }
            // Default: SA Score (cached)
            if let Some(&v) = sa_cache.get(&e.smiles) {
                return 1.0 + 0.5 * (v - 1.0) / 9.0;
            }
            let v = mol_from_smiles(&e.smiles)
                .map(|m| sa_score(&m).clamp(1.0, 10.0))
                .unwrap_or(5.5);
            sa_cache.insert(e.smiles.clone(), v);
            1.0 + 0.5 * (v - 1.0) / 9.0
        })
        .sum()
}

/// Classify a rule name into a human-readable reaction family.
/// Hand-crafted rules only; extracted templates return None.
fn reaction_family_for_rule(rule: &str) -> Option<&'static str> {
    match rule {
        "ester_cleavage" => Some("esterification"),
        "amide_cleavage" => Some("amide_coupling"),
        "friedel_crafts_acylation_retro" => Some("friedel_crafts_acylation"),
        "aryl_carboxylation_retro" => Some("decarboxylation"),
        "aryl_ether_retro" => Some("ullmann_ether"),
        // aryl_chloride_retro / aryl_iodide_retro / aryl_fluoride_snAr_retro
        // removed from default_rules() (31.11, chem_env.rs) — atom-loss bug,
        // no tracked reagent. aryl_amine_retro and buchwald_hartwig_retro
        // removed the same way (issue #77, ring-fused-nitrogen atom loss --
        // buchwald_hartwig_retro's surviving fragment came back corrupted
        // too). Arms deleted so this stays dead-code-free.
        "aryl_chloride_to_bromide" => Some("halogen_exchange"),
        "suzuki_retro" => Some("suzuki_coupling"),
        "heck_retro" | "heck_retro_terminal" => Some("heck_reaction"),
        "wittig_retro" => Some("wittig_reaction"),
        "reductive_amination_retro" => Some("reductive_amination"),
        "sonogashira_retro" => Some("sonogashira_coupling"),
        "sulfonamide_retro" => Some("sulfonamide_formation"),
        "diaryl_sulfone_retro" => Some("friedel_crafts_sulfonylation"),
        "boc_deprotection_retro" => Some("boc_deprotection"),
        "cbz_deprotection_retro" => Some("cbz_deprotection"),
        // n_benzylation_retro / michael_retro / negishi_retro /
        // grignard_addition_retro removed from default_rules() (v0.36.0
        // rule-safety census, chem_env.rs) -- same ring-fused
        // bare-fragment atom-duplication defect as aryl_amine_retro/
        // buchwald_hartwig_retro above. Arms deleted so this stays
        // dead-code-free.
        "claisen_retro" => Some("claisen_condensation"),
        "acyl_chloride_from_acid" => Some("acyl_chloride_formation"),
        "alcohol_oxidation_retro" => Some("carbonyl_reduction"),
        _ => None,
    }
}

/// True iff `rule` came from `chem_env::load_rules_from_file` (always named
/// `extracted_{i}`) rather than `default_rules()`. Used for `metadata_source`/
/// `metadata_scope` tagging (unchanged since PR #48): `conditions_for_rule`/
/// `reaction_family_for_rule` both return `None` for 3 legitimately-hand-crafted
/// generic-cleavage rules, so `.is_some()` on either would mis-tag those 3 as
/// extracted. (`RetroRule::template_id`'s `rule:`/`smirks-sha256:` prefix is
/// also a reliable discriminator now, but this function's name-prefix check
/// is kept as-is to avoid changing existing tagging behavior.)
pub(crate) fn is_extracted_template(rule: &str) -> bool {
    rule.starts_with("extracted_")
}

/// Rule-based reaction conditions for hand-crafted retro rules.
/// Returns None for extracted templates (conditions unknown without ML).
fn conditions_for_rule(rule: &str) -> Option<ReactionConditions> {
    macro_rules! cond {
        ($cat:expr, $sol:expr, $tmp:expr) => {
            Some(ReactionConditions {
                catalyst: Some($cat.into()),
                solvent: Some($sol.into()),
                temperature: Some($tmp.into()),
                notes: None,
            })
        };
        ($cat:expr, $sol:expr, $tmp:expr, $note:expr) => {
            Some(ReactionConditions {
                catalyst: Some($cat.into()),
                solvent: Some($sol.into()),
                temperature: Some($tmp.into()),
                notes: Some($note.into()),
            })
        };
    }
    match rule {
        "ester_cleavage" => cond!("NaOH or LiOH (2 eq)", "THF/H₂O (2:1)", "rt → 60 °C"),
        "amide_cleavage" => cond!("LiOH (3 eq)", "THF/H₂O (3:1)", "60 °C"),
        "friedel_crafts_acylation_retro" => cond!("AlCl₃ (1.2 eq)", "DCM", "0 °C → rt"),
        "aryl_carboxylation_retro" => {
            cond!("none", "water", "150 °C", "Kolbe-Schmitt / decarboxylation")
        }
        // aryl_amine_retro's and buchwald_hartwig_retro's condition entries
        // removed with the rules (issue #77) — see reaction_family_for_rule
        // above.
        "aryl_ether_retro" => cond!("Cs₂CO₃ (2 eq)", "DMF", "110 °C", "Ullmann ether retro"),
        "aryl_chloride_to_bromide" => cond!("NaBr (excess)", "DMF", "120 °C", "halogen exchange"),
        "suzuki_retro" => cond!("Pd(PPh₃)₄ (5 mol%)", "EtOH/H₂O (3:1)", "80 °C"),
        "heck_retro" => cond!("Pd(OAc)₂ / PPh₃ (5 mol%)", "DMF", "100 °C"),
        "heck_retro_terminal" => cond!("Pd(OAc)₂ / PPh₃ (5 mol%)", "DMF", "100 °C"),
        // negishi_retro's condition entry removed with the rule (v0.36.0
        // rule-safety census) -- see reaction_family_for_rule above.
        "cc_single_cleavage" => None, // retrosynthetic disconnection only
        "wittig_retro" => cond!("Ph₃P (1.2 eq)", "toluene", "0 °C → rt"),
        "reductive_amination_retro" => cond!("NaBH₃CN (1.5 eq)", "MeOH", "rt"),
        "cn_aliphatic_cleavage" => None,
        "co_aliphatic_cleavage" => None,
        "alcohol_oxidation_retro" => {
            cond!("NaBH₄ (1.2 eq)", "EtOH", "0 °C → rt", "retro = reduction")
        }
        "sonogashira_retro" => cond!("Pd(PPh₃)₂Cl₂ / CuI (5 mol%)", "Et₃N", "60 °C"),
        "sulfonamide_retro" => cond!("Et₃N (2 eq)", "DCM", "0 °C → rt"),
        "diaryl_sulfone_retro" => cond!(
            "AlCl₃ (1.2 eq)",
            "DCM",
            "0 °C → rt",
            "Friedel-Crafts sulfonylation"
        ),
        "boc_deprotection_retro" => cond!("TFA (20 % in DCM)", "DCM", "rt"),
        // n_benzylation_retro's, michael_retro's, and
        // grignard_addition_retro's condition entries removed with the
        // rules (v0.36.0 rule-safety census) -- see reaction_family_for_rule
        // above.
        "claisen_retro" => cond!("LDA (2.0 eq)", "THF (dry)", "−78 °C"),
        "acyl_chloride_from_acid" => cond!("(COCl)₂ (1.2 eq) + cat. DMF", "DCM", "0 °C → rt"),
        "cbz_deprotection_retro" => cond!("H₂ (1 atm), Pd/C (10 %)", "EtOH", "rt"),
        _ => None,
    }
}

/// One-line experimental procedure hint for hand-crafted retro rules (forward direction).
/// Placeholder infrastructure for QFANG-style structured procedure generation.
fn procedure_hint_for_rule(rule: &str) -> Option<&'static str> {
    match rule {
        "ester_cleavage" => {
            Some("Dissolve in THF/H₂O, add NaOH (2 eq), stir at 60 °C, acidify to pH 2.")
        }
        "amide_cleavage" => Some("Reflux in 6M HCl or add LiOH (3 eq) in THF/H₂O at 60 °C."),
        "friedel_crafts_acylation_retro" => {
            Some("Add acid chloride to arene + AlCl₃ (1.2 eq) in DCM at 0 °C, warm to rt.")
        }
        // buchwald_hartwig_retro's entry removed with the rule (issue #77)
        // — see reaction_family_for_rule above. aryl_amine_retro never had
        // an entry here (this function's coverage was always partial).
        "aryl_ether_retro" => {
            Some("Mix aryl halide + phenol + Cs₂CO₃ (2 eq) in DMF, heat at 110 °C.")
        }
        "suzuki_retro" => {
            Some("Combine aryl boronate + aryl halide + Pd(PPh₃)₄ in EtOH/H₂O, reflux at 80 °C.")
        }
        "heck_retro" | "heck_retro_terminal" => {
            Some("Add alkene + aryl halide + Pd(OAc)₂/PPh₃ in DMF with Et₃N at 100 °C.")
        }
        "wittig_retro" => {
            Some("Add aldehyde to Ph₃P=CHR (Wittig ylide) in toluene at 0 °C, warm to rt.")
        }
        "reductive_amination_retro" => {
            Some("Mix aldehyde + amine in MeOH, add NaBH₃CN (1.5 eq), stir at rt.")
        }
        "sonogashira_retro" => {
            Some("Combine terminal alkyne + aryl halide + Pd/CuI in Et₃N at 60 °C.")
        }
        "sulfonamide_retro" => Some("Add sulfonyl chloride to amine + Et₃N (2 eq) in DCM at 0 °C."),
        "boc_deprotection_retro" => {
            Some("Treat with TFA (20% in DCM) at rt for 1 h, then evaporate.")
        }
        "cbz_deprotection_retro" => Some("Hydrogenate (H₂, 1 atm) over Pd/C (10%) in EtOH at rt."),
        // grignard_addition_retro's entry removed with the rule (v0.36.0
        // rule-safety census) -- see reaction_family_for_rule above.
        "acyl_chloride_from_acid" => {
            Some("Add oxalyl chloride (1.2 eq) + cat. DMF to carboxylic acid in DCM at 0 °C.")
        }
        "alcohol_oxidation_retro" => {
            Some("Reduce ketone/aldehyde with NaBH₄ (1.2 eq) in EtOH at 0 °C → rt.")
        }
        "claisen_retro" => Some(
            "Deprotonate ester α-position with LDA (2 eq) in dry THF at −78 °C, add electrophile.",
        ),
        // michael_retro's and n_benzylation_retro's entries removed with
        // the rules (v0.36.0 rule-safety census) -- grignard_addition_retro
        // and negishi_retro never had entries here in the first place --
        // see reaction_family_for_rule above.
        _ => None,
    }
}

/// Convergency score for a route: 1.0 = all leaf branches same depth (ideal parallel
/// synthesis); 0.0 = purely linear. Computed from depth of each leaf in the step tree.
fn convergency_score(steps: &[ReactionStep]) -> f64 {
    if steps.is_empty() {
        return 1.0;
    }
    // BFS: assign depth to every molecule in the tree.
    let mut depth_map: rustc_hash::FxHashMap<&str, u32> = rustc_hash::FxHashMap::default();
    if let Some(first) = steps.first() {
        depth_map.insert(first.target.as_str(), 0);
    }
    for step in steps {
        let d = depth_map.get(step.target.as_str()).copied().unwrap_or(0);
        for prec in &step.precursors {
            depth_map.entry(prec.as_str()).or_insert(d + 1);
        }
    }
    let targets: rustc_hash::FxHashSet<&str> = steps.iter().map(|s| s.target.as_str()).collect();
    let leaf_depths: Vec<u32> = depth_map
        .iter()
        .filter(|(k, _)| !targets.contains(*k))
        .map(|(_, &v)| v)
        .collect();
    if leaf_depths.len() <= 1 {
        return 1.0;
    }
    let max = leaf_depths.iter().copied().max().unwrap_or(0) as f64;
    let min = leaf_depths.iter().copied().min().unwrap_or(0) as f64;
    if max == 0.0 {
        1.0
    } else {
        1.0 - (max - min) / max
    }
}

/// Estimate synthesis cost for a route.
///
/// `Σ(BB complexity or price) + step_count × 0.5`
///
/// BB cost: price from `prices` map if available; otherwise SA Score (1–10 scale).
/// Lower values indicate cheaper / simpler routes.
fn compute_route_cost(
    route: &Route,
    prices: Option<&std::collections::HashMap<String, f64>>,
) -> f64 {
    use chematic::chem::sa_score;

    let bb_cost: f64 = route
        .building_blocks
        .iter()
        .map(|smiles| {
            if let Some(map) = prices
                && let Some(&p) = map.get(smiles.as_str())
            {
                return p;
            }
            mol_from_smiles(smiles)
                .ok()
                .map(|m| sa_score(&m))
                .unwrap_or(5.0)
        })
        .sum();
    bb_cost + route.steps.len() as f64 * 0.5
}

/// Prune the heap to at most `beam_width` nodes (keep the best).
/// Uses sort_unstable_by (lower constant than sort_by) for deterministic ordering.
///
/// `(evicted_count, evicted_f_min, evicted_f_max, boundary_f)`, present only
/// when a truncation actually happened.
type BeamEvictionStats = (usize, f64, f64, f64);
/// `(trace_id, rank, survived)` for one traced node -- see [`beam_prune`].
type TraceRank = (u64, usize, bool);

/// Returns `(eviction_stats, trace_ranks, diversity_stats)`.
/// `eviction_stats` is `Some((evicted_count, evicted_f_min, evicted_f_max,
/// boundary_f))` when a truncation actually happened (diagnostics-only
/// bookkeeping over the sort this function already performs -- does not
/// change which nodes are kept), `None` when nothing was pruned. These
/// three stats are always computed over the pure top-`beam_width`-by-score
/// cutoff, *even under* `BeamDiversityPolicy::Active` (where the actual
/// survivor set can differ from that cutoff) -- this keeps their meaning
/// stable and comparable across every policy; `diversity_stats`
/// (`DiversityReservationStats::default()` unless `diversity_policy` is
/// `DiagnosticsOnly`/`Active`) is the correct place to look for what the
/// diversity mechanism itself did. `trace_ranks` is `(trace_id, rank,
/// survived)` for every node with `Node::trace_id == Some(_)`; `rank` is
/// always the pure-score rank, but `survived` reflects true final-survivor
/// membership (which can differ from `rank < beam_width` under `Active`).
/// Both stats vecs are empty/default when nothing is truncated
/// (`heap.len() <= beam_width`): the heap isn't sorted in that branch, so
/// there is no real rank to report, and a traced node's
/// `CandidateTraceRecord` simply keeps its as-created
/// `rank_before_prune: None, survived_beam: true` rather than being
/// overwritten with a fabricated rank.
fn beam_prune(
    heap: &mut BinaryHeap<Node>,
    beam_width: usize,
    diversity_policy: BeamDiversityPolicy,
    diversity_slots: usize,
) -> (
    Option<BeamEvictionStats>,
    Vec<TraceRank>,
    DiversityReservationStats,
) {
    if beam_width == 0 || heap.len() <= beam_width {
        return (None, Vec::new(), DiversityReservationStats::default());
    }
    let mut nodes: Vec<Node> = heap.drain().collect();
    nodes.sort_unstable_by(|a, b| {
        a.f()
            .partial_cmp(&b.f())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if diversity_policy == BeamDiversityPolicy::Off {
        // Exact pre-existing behavior, byte-for-byte, zero extra cost --
        // no call into select_beam_survivors at all.
        let trace_ranks: Vec<TraceRank> = nodes
            .iter()
            .enumerate()
            .filter_map(|(rank, n)| n.trace_id.map(|id| (id, rank, rank < beam_width)))
            .collect();
        let evicted = &nodes[beam_width..];
        let evicted_f_min = evicted.iter().map(Node::f).fold(f64::INFINITY, f64::min);
        let evicted_f_max = evicted
            .iter()
            .map(Node::f)
            .fold(f64::NEG_INFINITY, f64::max);
        let boundary_f = nodes[beam_width - 1].f();
        let evicted_count = evicted.len();
        nodes.truncate(beam_width);
        *heap = nodes.into_iter().collect();
        return (
            Some((evicted_count, evicted_f_min, evicted_f_max, boundary_f)),
            trace_ranks,
            DiversityReservationStats::default(),
        );
    }

    let trace_ranks_by_rank: Vec<(u64, usize)> = nodes
        .iter()
        .enumerate()
        .filter_map(|(rank, n)| n.trace_id.map(|id| (id, rank)))
        .collect();
    let evicted = &nodes[beam_width..];
    let evicted_f_min = evicted.iter().map(Node::f).fold(f64::INFINITY, f64::min);
    let evicted_f_max = evicted
        .iter()
        .map(Node::f)
        .fold(f64::NEG_INFINITY, f64::max);
    let boundary_f = nodes[beam_width - 1].f();
    let evicted_count = evicted.len();

    let (survivors, diversity_stats) =
        select_beam_survivors(nodes, beam_width, diversity_slots, diversity_policy);
    let survivor_trace_ids: FxHashSet<u64> = survivors.iter().filter_map(|n| n.trace_id).collect();
    let trace_ranks: Vec<TraceRank> = trace_ranks_by_rank
        .into_iter()
        .map(|(id, rank)| (id, rank, survivor_trace_ids.contains(&id)))
        .collect();
    *heap = survivors.into_iter().collect();

    (
        Some((evicted_count, evicted_f_min, evicted_f_max, boundary_f)),
        trace_ranks,
        diversity_stats,
    )
}

/// `docs/design/diversity-reserved-beam-v0.md` §5. Wired into `beam_prune`
/// via [`SearchConfig::beam_diversity_policy`] (rollout stage 3) -- `Off`
/// (the default) reproduces pre-existing behavior exactly, byte-for-byte,
/// zero extra cost. CLI/Python/WASM exposure is stage 4, not yet done --
/// `Active` is reachable only by constructing a `SearchConfig` directly
/// (Rust callers/tests), not from any shipped external surface yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamDiversityPolicy {
    /// Default: identical to today's `beam_prune` (pure top-`beam_width`
    /// by score). No diversity computation happens at all.
    Off,
    /// Computes what `Active` would additionally keep, for measurement,
    /// but returns the exact same survivor set as `Off` -- never changes
    /// which nodes actually continue the search.
    DiagnosticsOnly,
    /// Actually reserves `diversity_slots` beam slots for family diversity
    /// (design doc §6).
    Active,
}

/// See [`BeamDiversityPolicy::Active`]. `families_rescued_by_reservation`
/// counts distinct families given a beam slot that pure top-`score_slots`
/// selection (`beam_width - diversity_slots`) would not have included --
/// this is deliberately relative to the score-only cutoff, not to
/// `Off`'s own full-`beam_width` survivor set, since a handful of
/// diversity picks may coincidentally already rank inside the top
/// `beam_width` anyway; the design doc's own §7 "diversity yield" metric
/// (rescued / diversity_slots) is meant to answer "are the reserved slots
/// doing anything," which this counts correctly either way. Accumulated
/// across every `beam_prune` call in a search via
/// [`CrowdOutDiagnostics::beam_diversity_stats`].
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct DiversityReservationStats {
    pub families_represented_by_score_alone: usize,
    pub families_rescued_by_reservation: usize,
    /// Only traced nodes (`Node::trace_id.is_some()`) are identifiable
    /// here -- matches `beam_prune`'s own `TraceRank` opt-in convention.
    pub rescued_node_trace_ids: Vec<u64>,
}

impl std::ops::AddAssign for DiversityReservationStats {
    fn add_assign(&mut self, other: Self) {
        self.families_represented_by_score_alone += other.families_represented_by_score_alone;
        self.families_rescued_by_reservation += other.families_rescued_by_reservation;
        self.rescued_node_trace_ids
            .extend(other.rescued_node_trace_ids);
    }
}

/// Computes which of `nodes` survive beam pruning under `policy`, given
/// `beam_width` total slots and `diversity_slots` of those reserved for
/// family diversity (design doc §6's 3-step algorithm). `nodes` need not
/// be pre-sorted. Returns the selected survivors (unsorted relative to
/// score -- caller's responsibility to re-sort if needed, matching how
/// `beam_prune` itself rebuilds the heap from an unordered `Vec` today)
/// plus diagnostics. `diversity_slots` is clamped to `beam_width`.
///
/// `Off` and `DiagnosticsOnly` both return byte-for-byte the same survivor
/// set as pure top-`beam_width`-by-score -- `Off` skips all diversity
/// computation entirely (zero cost), `DiagnosticsOnly` still computes it
/// (for `DiversityReservationStats`) but discards its effect on selection.
/// Only `Active` actually changes which nodes survive. Called from
/// `beam_prune` only when `policy != Off` (that case is handled inline
/// there instead, to guarantee zero extra cost for the default policy).
fn select_beam_survivors(
    nodes: Vec<Node>,
    beam_width: usize,
    diversity_slots: usize,
    policy: BeamDiversityPolicy,
) -> (Vec<Node>, DiversityReservationStats) {
    let mut sorted = nodes;
    sorted.sort_unstable_by(|a, b| {
        a.f()
            .partial_cmp(&b.f())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if policy == BeamDiversityPolicy::Off {
        sorted.truncate(beam_width);
        return (sorted, DiversityReservationStats::default());
    }

    // `off_survivors` is what both Off and DiagnosticsOnly must return --
    // computed up front so DiagnosticsOnly can hand it back unmodified
    // regardless of what the diversity computation below finds.
    let mut off_survivors = sorted.clone();
    off_survivors.truncate(beam_width);

    let diversity_slots = diversity_slots.min(beam_width);
    let score_slots = beam_width - diversity_slots;
    let remainder = if sorted.len() <= score_slots {
        Vec::new()
    } else {
        sorted.split_off(score_slots)
    };
    let score_selected = sorted; // now len() <= score_slots

    let mut represented: std::collections::HashSet<String> = score_selected
        .iter()
        .filter_map(|n| n.family_key.clone())
        .collect();
    let families_represented_by_score_alone = represented.len();

    let mut diversity_selected: Vec<Node> = Vec::new();
    let mut rescued_node_trace_ids = Vec::new();
    for node in remainder {
        if diversity_selected.len() >= diversity_slots {
            break;
        }
        let Some(key) = node.family_key.clone() else {
            continue; // design doc §3: no family identity, never eligible
        };
        if represented.contains(&key) {
            continue;
        }
        represented.insert(key);
        if let Some(id) = node.trace_id {
            rescued_node_trace_ids.push(id);
        }
        diversity_selected.push(node);
    }

    let stats = DiversityReservationStats {
        families_represented_by_score_alone,
        families_rescued_by_reservation: diversity_selected.len(),
        rescued_node_trace_ids,
    };

    match policy {
        BeamDiversityPolicy::Off => unreachable!("handled above"),
        BeamDiversityPolicy::DiagnosticsOnly => (off_survivors, stats),
        BeamDiversityPolicy::Active => {
            let mut survivors = score_selected;
            survivors.extend(diversity_selected);
            (survivors, stats)
        }
    }
}

/// Diagnostics-only (Issue #101 / Phase 1B): for one node's expansion,
/// count (a) proposals from *different* templates whose precursor SMILES
/// multiset (sorted) is identical to an already-seen one -- same as the
/// original Issue #101 counter -- and (b) how many candidates would remain
/// under two hypothetical dedup strategies (same-template-only vs.
/// cross-template), without actually merging, dedupeing, or reordering
/// `entries`.
///
/// Returns `(cross_template_duplicates, after_same_template_dedup,
/// after_cross_template_dedup)`.
fn dedup_counts(entries: &[RetroEntry]) -> (u64, u64, u64) {
    let mut cross_template_duplicates = 0u64;
    // Keyed by (template_id, sorted precursor signature): collapses only
    // exact same-template repeats.
    let mut seen_same_template: FxHashSet<(&str, Vec<String>)> = FxHashSet::default();
    // Keyed by sorted precursor signature alone: collapses regardless of template.
    let mut seen_cross_template: FxHashMap<Vec<String>, &str> = FxHashMap::default();
    for e in entries {
        let mut sig = e.precursor_smiles.clone();
        sig.sort_unstable();
        seen_same_template.insert((e.template_id.as_str(), sig.clone()));
        match seen_cross_template.get(sig.as_slice()) {
            Some(&prev_template) if prev_template != e.template_id => {
                cross_template_duplicates += 1;
            }
            Some(_) => {}
            None => {
                seen_cross_template.insert(sig, e.template_id.as_str());
            }
        }
    }
    (
        cross_template_duplicates,
        seen_same_template.len() as u64,
        seen_cross_template.len() as u64,
    )
}

/// Issue #101 Task 35 runtime integration: score one expansion's raw
/// proposals with `reranker`, exactly as the offline pool pipeline scores a
/// candidate pool, and return a `candidate_id -> bonus` map on
/// `template_bonus`'s own [0.0, 0.2] scale.
///
/// Two fidelity requirements, both load-bearing:
///  - Candidates are built via [`crate::candidate::merge_into_candidates`],
///    the same merge the offline exporter (`src/pool_export.rs`) uses --
///    `entries` in the caller stays one `RetroEntry` per raw proposal
///    (crowd-out diagnostics deliberately never merges at search time), but
///    the reranker must see exactly the same candidate identities the model
///    was trained to rank, or the score-to-candidate mapping is not the one
///    that was validated.
///  - `extract_features` is called with `stock: None`, matching
///    `src/bin/pool_gen.rs`'s own `/* stock */ None` -- the offline training
///    pool was built without a stock (`fraction_precursors_in_stock` /
///    `all_precursors_in_stock` were `missing` for every training row), so
///    passing this call site's real `env` stock would feed the frozen model
///    a feature distribution it never saw in training, even though a stock
///    happens to be available here.
///
/// Rank (not raw score) becomes the bonus, via a total, content-based order
/// (`reranker_score` descending, `candidate_id` ascending as tie-break --
/// `apply_retro` runs in parallel on native, so proposal order alone is not
/// a stable tie-break and this must not depend on it). The bonus REPLACES
/// `template_bonus`/`ReactionPrior::prior` at the call site, it is never
/// added on top: summing both would push the effective step-cost bonus
/// outside the calibrated range the A*/beam-prune g/h split assumes, and
/// would stop this from being an ordering-only change.
fn reranker_rank_bonuses(
    reranker: &dyn crate::candidate::CandidateReranker,
    target_smi: &str,
    target_mol: &crate::chem_env::Molecule,
    raw_proposals: &[crate::candidate::RawCandidate],
    templates_by_id: &std::collections::HashMap<String, &RetroRule>,
) -> anyhow::Result<FxHashMap<String, f64>> {
    let mut candidates = crate::candidate::merge_into_candidates(target_smi, raw_proposals)?;
    for c in candidates.iter_mut() {
        c.features = crate::candidate::extract_features(c, target_mol, templates_by_id, None);
    }
    reranker.score_pool(target_smi, &mut candidates)?;
    candidates.sort_by(|a, b| {
        b.reranker_score
            .partial_cmp(&a.reranker_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });
    let n = candidates.len();
    Ok(candidates
        .into_iter()
        .enumerate()
        .map(|(rank, c)| (c.candidate_id, crate::score::rank_bonus(rank, n)))
        .collect())
}

/// Classify a template's provenance from its stable identity and SMIRKS
/// (competitive-diagnostics program, Phase 1B). Read-only: does not touch
/// rule loading or application.
fn classify_provenance(template_id: &str, smirks: &str) -> CandidateProvenance {
    if template_id.starts_with("rule:") {
        CandidateProvenance::Handcrafted
    } else if smirks.contains("[#") {
        CandidateProvenance::HashAtom
    } else {
        CandidateProvenance::FileBacked
    }
}

pub struct SearchConfig {
    pub max_depth: u32,
    pub max_routes: usize,
    /// 0 = unlimited (pure A*). N > 0 = beam search, keep top-N nodes.
    pub beam_width: usize,
    /// Element bitmask (same format as `RetroRule::required_elements`).
    /// Routes whose leaf building blocks contain any forbidden element are dropped.
    /// 0 = no constraint.
    pub forbidden_elements: u64,
    /// Routes are kept only when the union of all leaf BB element masks covers this mask.
    /// 0 = no constraint.
    pub required_element_present: u64,
    /// Print search statistics (nodes expanded, elapsed time) to stderr after search.
    pub verbose: bool,
    /// Bond-center template index (RetroKNN-inspired).
    /// When true, only templates whose SMIRKS bond pairs match bonds present in
    /// the target molecule are tried. Graph-based and fallback rules are always included.
    /// Typically gives ~24% speedup over the full template set with no accuracy loss.
    pub bond_index: bool,
    /// Optional building block price map: canonical SMILES → price per gram.
    /// When Some, route_cost uses these prices; unmatched BBs fall back to SA Score.
    /// When None, route_cost uses SA Score for all BBs.
    pub bb_price_map: Option<std::collections::HashMap<String, f64>>,
    /// Custom molecule value estimator for the A* heuristic.
    /// None = use `SaScoreEstimator` (default SA Score-based behaviour).
    pub value_estimator: Option<std::sync::Arc<dyn MoleculeValueEstimator>>,
    /// Custom reaction prior for template scoring.
    /// None = use `FrequencyPrior` (log-frequency weighting, same as pre-v0.9 behaviour).
    pub reaction_prior: Option<std::sync::Arc<dyn ReactionPrior>>,
    /// Optional template metadata sidecar (`--template-metadata` / Python
    /// `template_metadata_path`), keyed by `RetroRule::template_id`. When Some,
    /// matching steps get `evidence` populated in post-processing; unmatched
    /// templates are left as `None` -- nothing is fabricated. `None` (the
    /// default) reproduces pre-existing search behaviour exactly.
    pub template_metadata: Option<std::collections::HashMap<String, TemplateMetadataEntry>>,
    /// Phase B: ONNX template relevance scorer (CLI/Python only, not WASM).
    /// When Some, pre-filters rules to top-K most relevant before SMARTS matching.
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    pub nn_scorer: Option<std::sync::Arc<crate::scorer::nn::TemplateScorer>>,
    /// Ring-context safety guard configuration (Issue #72). `Disabled` (the
    /// default) reproduces pre-existing behaviour exactly -- extracted
    /// templates are applied via the unmodified legacy `apply_retro` path,
    /// no sidecar is required. `Guarded` always carries a loaded guard
    /// alongside its enforcement policy; "enforce without a guard" is not a
    /// state this type can represent.
    pub ring_context: crate::ring_context::RingContextConfig,
    /// Competitive-diagnostics program, Phase 1B: when `Some(cap)`, collect
    /// up to `cap` per-candidate trace records (`CrowdOutDiagnostics::candidate_trace`)
    /// across the whole search, in first-generated (deterministic) order.
    /// `None` (the default) collects nothing -- zero extra allocation or
    /// bookkeeping beyond the always-on aggregate counters. Offline
    /// diagnostic use only (CLI `--candidate-trace-limit`); does not affect
    /// which candidates are expanded, scored, kept, or in what order.
    pub candidate_trace_cap: Option<usize>,
    /// Ordering-only LightGBM candidate reranker (Issue #101 Task 35).
    /// `None` (the default) reproduces this crate's legacy
    /// `template_bonus`/`reaction_prior` ordering byte-for-byte -- see
    /// `reranker_rank_bonuses`' doc for exactly how a `Some` value changes
    /// step-cost bonuses. "Ordering-only" is precise at the single-
    /// expansion level: for one node, the reranker never adds, drops, or
    /// merges a `RetroEntry` -- the same raw proposals become the same set
    /// of children either way, just under different `step_cost`s. It is
    /// NOT a claim that the whole search explores the identical candidate
    /// set end to end under a nonzero `beam_width`: a changed `step_cost`
    /// changes `f() = g + h`, which changes which open nodes `beam_prune`
    /// evicts, which changes which of THEIR children ever get proposed at
    /// all deeper in the tree. That's the intended mechanism (it's how a
    /// reranker can fix beam-width crowd-out), not a bug -- just don't
    /// read "ordering-only" as "identical search tree regardless of beam
    /// width," which it isn't and was never meant to be.
    pub reranker: Option<std::sync::Arc<dyn crate::candidate::CandidateReranker>>,
    /// Issue #128 diagnostic instrument: when `true`, accumulate
    /// [`CrowdOutDiagnostics::retro_expansion_wall_time_us`] on every
    /// retro-cache-miss expansion. `false` (the default) leaves that field
    /// at `0` -- wall-clock timing is inherently non-deterministic across
    /// repeated runs (unlike every other `CrowdOutDiagnostics` field, which
    /// is a pure count), so it must stay opt-in: `crowd_out_diagnostics_
    /// are_deterministic_across_repeated_runs` and
    /// `wrapper_matches_unlimited_control_exactly` both assert
    /// byte-identical `CrowdOutDiagnostics` JSON for identical inputs, and
    /// neither opts in, so both stay unaffected. Offline diagnostic use
    /// only (no CLI flag yet -- construct `SearchConfig` directly); never
    /// changes which candidates are expanded, scored, kept, or in what
    /// order. No-op on `wasm32` (`std::time::Instant::now()` is
    /// unavailable there) -- the field stays `0` regardless of this flag on
    /// that target.
    pub timing_diagnostics: bool,
    /// Opt-in (default [`SpectatorBondPolicy::Off`]), same rationale as
    /// [`Self::timing_diagnostics`] but for cost rather than
    /// non-determinism: `spectator_bond::detect_case_a`/`detect_case_b`
    /// run real SMARTS matching (and, for Case B, a BFS) per
    /// `(target, rule)` pair on every retro-cache-miss expansion -- real
    /// work, not a free counter increment, so it must stay opt-in like the
    /// existing `CrowdOutDiagnostics` fields that aren't pure counts.
    /// `DiagnosticsOnly` appends every finding either detector returns to
    /// [`CrowdOutDiagnostics::spectator_bond_loss_findings`] verbatim,
    /// never filtering which candidates the search itself considers.
    /// `Gated` additionally excludes candidates
    /// [`crate::spectator_bond::gate_candidates`] resolves with confidence
    /// (v1 scope: `#`-free rules only -- see
    /// `docs/design/spectator-bond-fail-closed-gating-v0.md`), recording
    /// each exclusion in
    /// [`CrowdOutDiagnostics::spectator_bond_gated_out`]. `Off` (the
    /// default) reproduces this field's own pre-existing empty-`Vec`
    /// behavior exactly, byte-for-byte, matching `timing_diagnostics`'s
    /// own determinism-test precedent.
    pub spectator_bond_policy: SpectatorBondPolicy,
    /// Diversity-reserved beam (ROADMAP Item 4, issue #101,
    /// `docs/design/diversity-reserved-beam-v0.md`) -- an independent
    /// axis from `spectator_bond_policy` above, never combined into one
    /// label (same "orthogonal policies" rule this codebase already
    /// applies to `ring_context`). `Off` (the default) reproduces
    /// `beam_prune`'s pre-existing pure-top-K-by-score behavior exactly,
    /// byte-for-byte, with zero extra computation. `DiagnosticsOnly`
    /// computes what `Active` would additionally keep
    /// (`CrowdOutDiagnostics::beam_diversity_stats`) but never changes
    /// which nodes survive pruning. `Active` reserves
    /// `beam_diversity_slots` beam slots for family diversity instead of
    /// pure score. Not yet exposed on any CLI/Python/WASM surface
    /// (design doc stage 4) -- reachable only by constructing a
    /// `SearchConfig` directly.
    pub beam_diversity_policy: BeamDiversityPolicy,
    /// Beam slots reserved for family diversity under `DiagnosticsOnly`/
    /// `Active` (design doc §6); ignored under `Off`. Clamped to
    /// `beam_width` inside `select_beam_survivors` -- never panics if set
    /// larger.
    pub beam_diversity_slots: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_routes: 5,
            beam_width: 0,
            forbidden_elements: 0,
            required_element_present: 0,
            verbose: false,
            bond_index: false,
            bb_price_map: None,
            value_estimator: None,
            reaction_prior: None,
            template_metadata: None,
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            nn_scorer: None,
            ring_context: crate::ring_context::RingContextConfig::Disabled,
            candidate_trace_cap: None,
            reranker: None,
            timing_diagnostics: false,
            spectator_bond_policy: SpectatorBondPolicy::Off,
            beam_diversity_policy: BeamDiversityPolicy::Off,
            beam_diversity_slots: 0,
        }
    }
}

/// Per-node NN template ranking (Phase D). `None` when no scorer is configured,
/// and always `None` on WASM / without the `nn-scoring` feature — callers must
/// fall back to `ranked_rules`/`bond_idx`, preserving the existing WASM
/// frequency/bond-index-only retrieval path unchanged.
#[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
fn nn_rank<'a>(
    config: &SearchConfig,
    rules: &'a [RetroRule],
    smiles: &str,
) -> Option<Vec<&'a RetroRule>> {
    config.nn_scorer.as_ref().map(|sc| {
        sc.top_k_indices(smiles, rules.len())
            .into_iter()
            .filter_map(|i| rules.get(i))
            .collect()
    })
}
#[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
fn nn_rank<'a>(
    _config: &SearchConfig,
    _rules: &'a [RetroRule],
    _smiles: &str,
) -> Option<Vec<&'a RetroRule>> {
    None
}

/// Cooperative cancellation budget for [`find_routes_with_control`]. Additive
/// API (v0.24 coverage-mode foundation) -- deliberately a separate type
/// rather than a new [`SearchConfig`] field, so existing struct-literal
/// callers of `SearchConfig` (this crate's own and any external consumer's)
/// never need to change, and [`find_routes`]'s signature and behaviour stay
/// byte-identical to before this existed.
///
/// `Instant`-based deadlines only work where a monotonic clock is available.
/// On `wasm32` (no safe `Instant::now()` in this crate's supported wasm
/// targets -- see `find_routes`' own pre-existing `#[cfg(not(target_arch =
/// "wasm32"))]`-gated timing locals), [`SearchControl::with_timeout`] and
/// [`SearchControl::with_deadline`] are simply not compiled in: nothing on
/// the wasm-facing surface (`src/wasm.rs`) ever needs cooperative
/// cancellation, and silently accepting a timeout request there without
/// ever honoring it would be a worse footgun than a compile error.
#[derive(Debug, Clone, Copy)]
pub struct SearchControl {
    #[cfg(not(target_arch = "wasm32"))]
    deadline: Option<std::time::Instant>,
}

impl SearchControl {
    /// No deadline -- [`find_routes_with_control`] behaves identically to a
    /// search with cooperative cancellation checks that never trip.
    pub fn unlimited() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            deadline: None,
        }
    }

    /// Deadline `timeout` from now (monotonic clock, native targets only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_timeout(timeout: std::time::Duration) -> Self {
        Self {
            deadline: std::time::Instant::now().checked_add(timeout),
        }
    }

    /// Explicit absolute deadline (native targets only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_deadline(deadline: std::time::Instant) -> Self {
        Self {
            deadline: Some(deadline),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn is_expired(&self) -> bool {
        self.deadline
            .is_some_and(|d| std::time::Instant::now() >= d)
    }
    #[cfg(target_arch = "wasm32")]
    fn is_expired(&self) -> bool {
        false
    }
}

/// Why [`find_routes_with_control`] stopped popping frontier nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchTermination {
    /// The frontier heap emptied or `max_routes` was reached -- same
    /// stopping conditions [`find_routes`] has always had.
    Completed,
    /// [`SearchControl`]'s deadline passed at one of the search's
    /// cooperative-cancellation checkpoints. **A soft, cooperative
    /// deadline -- not a hard real-time bound.** Between any two
    /// checkpoints, the search can run an unbounded (though normally
    /// small) amount of synchronous, uninterruptible work depending on
    /// `SearchConfig`: template application (`raw_propose`), NN template
    /// scoring (`nn_rank`, `nn-scoring` feature), an active reranker's
    /// `CandidateReranker::score_pool` (an arbitrary trait-object
    /// implementation this crate does not control the runtime of),
    /// dedup/diagnostics bookkeeping (`dedup_counts` and related
    /// `crowd_out` updates), and the per-candidate cost/heuristic
    /// computation each proposal goes through before being pushed onto
    /// the frontier. None of these are individually interruptible
    /// mid-call, so the true worst-case overshoot is "however long the
    /// slowest stretch of synchronous work between two checkpoints
    /// takes" for the configuration actually in use -- not a single
    /// fixed operation, and not something this type can bound for a
    /// reranker implementation it doesn't own. Whatever valid routes
    /// were found before the deadline are still returned, never
    /// discarded.
    DeadlineExceeded,
}

/// Return type of [`find_routes_with_control`] -- adds [`SearchTermination`]
/// alongside the same `routes`/`stats` [`find_routes`] has always returned.
#[derive(Debug)]
pub struct SearchRunResult {
    pub routes: Vec<Route>,
    pub stats: SearchStats,
    pub termination: SearchTermination,
}

/// Same search as [`find_routes`], with an explicit cooperative-cancellation
/// budget (`control`). [`find_routes`] is a thin wrapper over this function
/// using [`SearchControl::unlimited`] -- this is the one place the frontier
/// loop actually lives; there is no separate/duplicated search
/// implementation for the timed and untimed paths.
pub fn find_routes_with_control(
    target_smiles: &str,
    env: &ChemEnv,
    rules: &[RetroRule],
    config: &SearchConfig,
    control: &SearchControl,
) -> Result<SearchRunResult> {
    let target_mol = mol_from_smiles(target_smiles)?;
    let target_canonical = to_canonical(&target_mol);

    // Default rule order when no scorer/bond-index retrieval narrows it down.
    // Phase D (2026-07): the NN scorer used to rank ONCE against the root target
    // and reuse that order for every deeper intermediate. Measurement (986 solved
    // targets, 994 depth>=1 ground-truth steps) showed that's a poor proxy for
    // what's actually applicable at an intermediate: top-100 recall of the
    // ground-truth rule was 37.1% under root-only ranking vs 64.1% re-ranked
    // fresh on the intermediate (median rank 304 -> 27). So scoring now happens
    // per-node, right below in the retro_cache-miss branch — which already keys
    // on canonical intermediate SMILES, so each unique intermediate still gets
    // exactly one ONNX call for the whole search (cache hits skip it entirely).
    let ranked_rules: Vec<&RetroRule> = rules.iter().collect();

    let max_rule_weight = rules.iter().map(|r| r.weight).fold(1.0_f64, f64::max);

    // Phase 1B: template_id -> smirks, built once, used only when
    // candidate-trace collection is active (`config.candidate_trace_cap`)
    // to classify each traced candidate's provenance.
    let template_smirks: FxHashMap<&str, &str> = if config.candidate_trace_cap.is_some() {
        rules
            .iter()
            .map(|r| (r.template_id.as_str(), r.smirks.as_str()))
            .collect()
    } else {
        FxHashMap::default()
    };

    // Bond-center template index — built once, queried per-expansion (O(bonds) per node).
    let bond_idx: Option<TemplateBondIndex> = if config.bond_index {
        Some(TemplateBondIndex::build(rules))
    } else {
        None
    };

    // Local, mutable "is the reranker still usable this run" handle,
    // separate from `config.reranker` (which is immutable for the whole
    // call): a mid-run inference error disables it for the remainder of
    // this search rather than retrying every subsequent expansion, and
    // `reranker_failures` records that it happened (expected value: 0).
    let mut active_reranker = config.reranker.as_deref();
    let mut reranker_failures: u64 = 0;

    // Issue #101 Task 35: template_id -> &RetroRule, built once, used only
    // when a reranker is configured (extract_features needs it to compute
    // reaction-center features from each source's SMIRKS). This setup step
    // is itself fallible -- index_rules_by_template_id hard-errors on a
    // corpus with a conflicting duplicate template_id -- and must degrade
    // exactly like a mid-run inference failure does (warn, disable the
    // reranker for this whole run, never a hard error), not propagate via
    // `?` and abort the search entirely just because the reranker happened
    // to be turned on.
    let templates_by_id: std::collections::HashMap<String, &RetroRule> =
        if active_reranker.is_some() {
            match crate::candidate::index_rules_by_template_id(rules) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!(
                        "warning: reranker setup failed ({e:#}); falling back to legacy \
                         ordering for this run"
                    );
                    reranker_failures += 1;
                    active_reranker = None;
                    std::collections::HashMap::new()
                }
            }
        } else {
            std::collections::HashMap::new()
        };

    #[cfg(not(target_arch = "wasm32"))]
    let t0 = std::time::Instant::now();
    #[cfg(not(target_arch = "wasm32"))]
    let mut nodes_popped: u64 = 0;
    let mut nodes_expanded: u64 = 0;
    let mut max_depth_reached = false;
    let mut beam_limit_hit = false;
    let mut matched_templates: u64 = 0;
    let mut stock_hits: u64 = 0;
    let mut retro_cache_hits: u64 = 0;
    let mut ring_context_diagnostics = crate::ring_context::RingContextDiagnostics::default();
    let mut retro_cache_misses: u64 = 0;
    let mut crowd_out = CrowdOutDiagnostics::default();
    let mut route_integrity = RouteIntegrityDiagnostics::default();
    let mut termination = SearchTermination::Completed;

    let mut routes: Vec<Route> = Vec::new();
    let mut closed: FxHashSet<u64> = FxHashSet::default();
    let mut heap: BinaryHeap<Node> = BinaryHeap::new();
    let mut sa_cache: FxHashMap<String, f64> = FxHashMap::default();
    // Opt-D: per-search memoization of apply_retro results.
    // Key: canonical target SMILES. Value: Arc-wrapped filtered expansions.
    // Arc avoids full-Vec cloning on both hit (O(1) Arc::clone) and miss (no extra clone).
    let mut retro_cache: RetroCache = FxHashMap::default();

    let initial: SmallVec<[FEntry; 6]> = smallvec![FEntry {
        smiles: target_canonical.clone(),
    }];
    let h0 = compute_h(
        &initial,
        env,
        &mut sa_cache,
        config.value_estimator.as_ref(),
    );
    heap.push(Node {
        frontier: initial,
        path: None,
        depth: 0,
        g: 0.0,
        h: h0,
        trace_id: None,
        family_key: None,
    });

    'frontier: while let Some(node) = heap.pop() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            nodes_popped += 1;
        }
        // `max_routes` completion is checked *before* the deadline
        // (checkpoint 1 below): a search that already has everything it
        // was asked for is `Completed`, never `DeadlineExceeded`, even if
        // the clock happens to have also passed in the same instant --
        // the work genuinely finished, and callers auditing `termination`
        // should be able to tell "got what it needed" apart from "ran out
        // of time" without a race on which check happened to run first.
        if routes.len() >= config.max_routes {
            break;
        }
        // Checkpoint 1/3 (main frontier-loop top): cheapest, most frequent
        // check -- before any work happens for this popped node. The only
        // checkpoint reached on this loop's several `continue` paths
        // (depth cap, closed-set hit, empty/unparseable frontier entry)
        // that never reach checkpoint 2 or the child-processing check at
        // all -- not just a redundant early copy of them.
        if control.is_expired() {
            termination = SearchTermination::DeadlineExceeded;
            break;
        }

        // Single pass: count unsolved + find first unsolved entry simultaneously.
        let mut n_unsolved = 0usize;
        let mut first_unsolved: Option<&FEntry> = None;
        for e in node.frontier.iter() {
            if !is_bb(&e.smiles, env) {
                n_unsolved += 1;
                if first_unsolved.is_none() {
                    first_unsolved = Some(e);
                }
                crowd_out.non_stock_candidates += 1;
            } else {
                stock_hits += 1;
                crowd_out.stock_terminal_candidates += 1;
            }
        }

        if n_unsolved == 0 {
            let steps = collect_path(node.path.as_ref());
            let building_blocks = extract_building_blocks(&steps);
            let candidate = Route {
                steps,
                depth: node.depth,
                score: node.g,
                building_blocks,
                confidence: 0.0,          // computed below
                convergency: 0.0,         // computed below
                success_probability: 0.0, // computed below
                route_cost: 0.0,          // computed below
            };
            // Acceptance boundary (RENKIN Bridge PR1): a structurally
            // invalid candidate is discarded here and only here -- the
            // search itself is untouched, so it falls through to the same
            // depth-cap/expansion logic below exactly as an accepted
            // candidate would, and keeps looking for a valid one.
            let defects = route_integrity_defects(&candidate, &target_canonical);
            if defects.is_empty() {
                routes.push(candidate);
            } else {
                route_integrity.record(&defects);
            }
        }

        if node.depth >= config.max_depth {
            max_depth_reached = true;
            continue;
        }

        let key = state_hash(&node.frontier);
        if closed.contains(&key) {
            continue;
        }
        closed.insert(key);
        #[cfg(not(target_arch = "wasm32"))]
        {
            nodes_expanded += 1;
        }

        let Some(target_entry) = first_unsolved.or_else(|| node.frontier.first()) else {
            continue;
        };
        let target_smi = target_entry.smiles.clone();

        let Ok(target_mol) = mol_from_smiles(&target_smi) else {
            continue;
        };

        // Opt-D: look up the memoized expansion for this target molecule.
        // On cache miss: run apply_retro in parallel (native) / sequential (WASM),
        // filter invalid results, precompute net step cost, and store.
        // On cache hit: O(1) Arc::clone — no Vec data is copied.
        let expansions: Arc<Vec<RetroEntry>> = if let Some(cached) = retro_cache.get(&target_smi) {
            retro_cache_hits += 1;
            Arc::clone(cached) // O(1): pointer copy only, no Vec clone
        } else {
            retro_cache_misses += 1;
            #[cfg(not(target_arch = "wasm32"))]
            let expansion_t0 = config.timing_diagnostics.then(std::time::Instant::now);
            // Bond-center retrieval: filter ranked_rules to those relevant to this molecule's bonds.
            // Else, per-node NN ranking (Phase D) — scored fresh against THIS intermediate,
            // not the root; this whole branch only runs once per unique canonical
            // `target_smi` (retro_cache dedupes repeat visits), so it's exactly one ONNX
            // inference call per unique intermediate for the whole search, same as a
            // dedicated SMILES-keyed cache would give, with no extra cache to maintain.
            // Falls back to ranked_rules unchanged when neither is configured.
            let retrieved: Vec<&RetroRule>;
            let per_node: Vec<&RetroRule>;
            let active_rules: &[&RetroRule] = if let Some(ref idx) = bond_idx {
                retrieved = idx
                    .retrieve(&target_mol, 0, rules) // top_k=0 = no truncation
                    .into_iter()
                    .filter_map(|i| rules.get(i))
                    .collect();
                &retrieved
            } else if let Some(v) = nn_rank(config, rules, &target_smi) {
                per_node = v;
                &per_node
            } else {
                &ranked_rules
            };
            crowd_out.rules_attempted_total += active_rules.len() as u64;

            // Shared with the standalone `propose_one_step` candidate-pool API
            // (`crate::candidate::raw_propose`) so route search and offline
            // candidate generation apply the exact same rule-application
            // logic -- this must stay a call, not a re-inlined copy.
            // find_routes' own active-rule selection (above) is *not* a
            // ProposalMode -- it has its own bond_idx/nn_rank/ranked_rules
            // fallback chain, including per-node NN re-ranking that
            // ProposalMode::ScorerConditioned deliberately does not
            // reproduce (see candidate module doc) -- so these scores are
            // marked NotApplicable rather than reusing UpstreamScoreStatus's
            // Available variant, which is reserved for candidate-pool
            // generation going through an explicit ProposalMode.
            let scored_active_rules: Vec<crate::candidate::ScoredRuleRef<'_>> = active_rules
                .iter()
                .enumerate()
                .map(|(rank, &rule)| crate::candidate::ScoredRuleRef {
                    rule,
                    source_rank: rank,
                    upstream_score: None,
                    upstream_score_status: crate::candidate::UpstreamScoreStatus::NotApplicable,
                })
                .collect();
            let (raw_proposals, step_ring_diag, step_sbl_findings, step_gated_out) =
                crate::candidate::raw_propose(
                    &target_mol,
                    &target_smi,
                    &scored_active_rules,
                    crate::ring_context::RingContextArgs {
                        config: config.ring_context.clone(),
                    },
                    config.spectator_bond_policy,
                );
            ring_context_diagnostics.merge(&step_ring_diag);
            crowd_out
                .spectator_bond_loss_findings
                .extend(step_sbl_findings);
            crowd_out.spectator_bond_gated_out.extend(step_gated_out);

            // Issue #101 Task 35: score this expansion's candidate pool once
            // (not per-proposal) when a reranker is still active. A failure
            // here disables the reranker for the rest of this search (see
            // `active_reranker`'s doc) and falls back to legacy ordering for
            // this expansion and every later one -- never a hard error.
            let reranker_bonus_by_id: Option<FxHashMap<String, f64>> =
                if let Some(reranker) = active_reranker {
                    match reranker_rank_bonuses(
                        reranker,
                        &target_smi,
                        &target_mol,
                        &raw_proposals,
                        &templates_by_id,
                    ) {
                        Ok(map) => Some(map),
                        Err(e) => {
                            eprintln!(
                                "warning: reranker inference failed ({e:#}); falling back to \
                                 legacy ordering for the remainder of this search"
                            );
                            reranker_failures += 1;
                            active_reranker = None;
                            None
                        }
                    }
                } else {
                    None
                };

            let entries: Vec<RetroEntry> = raw_proposals
                .into_iter()
                .map(|p| {
                    let bonus = if let Some(ref map) = reranker_bonus_by_id {
                        let mut key: Vec<String> =
                            p.precursors.iter().map(|pm| pm.smiles.clone()).collect();
                        key.sort_unstable();
                        // A miss here would mean this exact raw_proposals
                        // slice produced a different candidate_id set when
                        // merged for scoring above than when re-keyed here
                        // -- an internal inconsistency between this
                        // function and merge_into_candidates/
                        // candidate_id_for, not a runtime/data condition to
                        // degrade gracefully from (unlike a genuinely
                        // external reranker failure, which IS handled by
                        // falling back). 0.0 would be silently
                        // indistinguishable from a legitimate worst-rank
                        // bonus (rank_bonus(count-1, count) == 0.0), so
                        // fail loudly instead of masking it.
                        *map.get(&crate::candidate::candidate_id_for(&target_smi, &key))
                            .unwrap_or_else(|| {
                                panic!(
                                    "candidate_id for proposal (rule {:?}, precursors {:?}) \
                                     missing from reranker_bonus_by_id -- this is a bug in \
                                     reranker_rank_bonuses/candidate_id_for consistency, not a \
                                     reranker failure",
                                    p.rule_name, key
                                )
                            })
                    } else if let Some(ref prior) = config.reaction_prior {
                        prior.prior(&p.rule_name, &target_smi)
                    } else {
                        template_bonus(p.rule_weight, max_rule_weight)
                    };
                    let step_c =
                        step_cost(&p.precursors.iter().map(|pm| &pm.mol).collect::<Vec<_>>())
                            - bonus;
                    let smiles_list: Vec<String> =
                        p.precursors.iter().map(|pm| pm.smiles.clone()).collect();
                    RetroEntry {
                        rule_name: p.rule_name,
                        template_id: p.template_id,
                        step_cost: step_c,
                        precursor_smiles: smiles_list,
                    }
                })
                .collect();

            // Diagnostics-only (Issue #101 / Phase 1B): same-parent
            // proposals from different templates landing on an identical
            // precursor multiset, plus what would remain under two
            // hypothetical dedup strategies. Does not merge or drop any entry.
            let (cross_dup, after_same_template, after_cross_template) = dedup_counts(&entries);
            crowd_out.cross_template_duplicate_precursor_signatures += cross_dup;
            crowd_out.candidates_generated_before_dedup += entries.len() as u64;
            crowd_out.candidates_after_same_template_dedup += after_same_template;
            crowd_out.candidates_after_cross_template_dedup += after_cross_template;

            let arc = Arc::new(entries);
            retro_cache.insert(target_smi.clone(), Arc::clone(&arc));
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(t0) = expansion_t0 {
                crowd_out.retro_expansion_wall_time_us += t0.elapsed().as_micros() as u64;
            }
            arc // no extra clone: Arc move
        };

        // Checkpoint 2/3 (right after the expansion block completes): on a
        // cache miss this follows template application (`raw_propose`),
        // NN scoring (`nn_rank`), and -- when a reranker is configured --
        // `CandidateReranker::score_pool`, none of which are individually
        // interruptible mid-call (see `SearchTermination::DeadlineExceeded`'s
        // doc for the full list this checkpoint can trail). On a cache hit
        // this check is nearly free, same as checkpoint 1.
        if control.is_expired() {
            termination = SearchTermination::DeadlineExceeded;
            break;
        }

        matched_templates += expansions.len() as u64;
        {
            let depth_entry = crowd_out.branching_by_depth.entry(node.depth).or_default();
            depth_entry.nodes_expanded += 1;
            depth_entry.children_produced += expansions.len() as u64;
        }

        for entry in expansions.iter() {
            // Checkpoint 3/3 (per child, before its heavier processing):
            // `expansions` can hold thousands of raw proposals at high
            // template counts, and each one below does real work (a fresh
            // `compute_h` heuristic call, path-node allocation, an
            // optional trace record) before ever reaching the next outer
            // iteration's checkpoint 1 -- checked per entry, not once
            // before the loop, so this loop itself stays boundable rather
            // than being the one place nothing gets checked between
            // outer-loop iterations. A labeled break is required here
            // (plain `break` would only exit this inner `for`) to actually
            // stop the search, not just this one node's expansion.
            if control.is_expired() {
                termination = SearchTermination::DeadlineExceeded;
                break 'frontier;
            }

            let new_frontier: SmallVec<[FEntry; 6]> = node
                .frontier
                .iter()
                .filter(|e| e.smiles != target_smi)
                .cloned()
                .chain(
                    entry
                        .precursor_smiles
                        .iter()
                        .map(|s| FEntry { smiles: s.clone() }),
                )
                .collect();

            let new_h = compute_h(
                &new_frontier,
                env,
                &mut sa_cache,
                config.value_estimator.as_ref(),
            );

            // O(1) Arc::clone — shares the parent prefix without copying.
            let new_path = Some(Arc::new(PathNode {
                step: ReactionStep {
                    rule: entry.rule_name.clone(),
                    template_id: entry.template_id.clone(),
                    target: target_smi.clone(),
                    precursors: entry.precursor_smiles.clone(),
                    conditions: conditions_for_rule(&entry.rule_name),
                    atom_economy: None,             // populated in post-processing
                    atom_economy_raw_percent: None, // populated in post-processing
                    atom_economy_status: AtomEconomyStatus::NotEvaluable, // populated in post-processing
                    step_confidence: 0.0, // populated in post-processing
                    reaction_family: reaction_family_for_rule(&entry.rule_name).map(str::to_string),
                    procedure_hint: procedure_hint_for_rule(&entry.rule_name).map(str::to_string),
                    metadata_source: (!is_extracted_template(&entry.rule_name))
                        .then_some(MetadataSource::HandcraftedDefault),
                    metadata_scope: (!is_extracted_template(&entry.rule_name))
                        .then_some(EvidenceScope::ReactionFamily),
                    evidence: None, // populated in post-processing, routes actually returned only
                },
                prev: node.path.clone(),
            }));

            // In-search pruning: skip expansions where a BB-precursor contains a
            // forbidden element. Avoids pushing dead-end nodes onto the heap.
            if config.forbidden_elements != 0 {
                let mask = config.forbidden_elements;
                if entry
                    .precursor_smiles
                    .iter()
                    .filter(|p| is_bb(p, env))
                    .any(|p| (elem_mask_from_smiles(p) & mask) != 0)
                {
                    continue;
                }
            }

            // Phase 1B: opt-in candidate-level trace record. `trace_id`
            // stays `None` (no record, no lookup) whenever
            // `candidate_trace_cap` is `None` or has already been reached --
            // the common case, costing one `Option` check.
            let trace_id = config.candidate_trace_cap.and_then(|cap| {
                if crowd_out.candidate_trace.len() >= cap {
                    return None;
                }
                let mut precursor_signature = entry.precursor_smiles.clone();
                precursor_signature.sort_unstable();
                let smirks = template_smirks
                    .get(entry.template_id.as_str())
                    .copied()
                    .unwrap_or("");
                let id = crowd_out.candidate_trace.len() as u64;
                crowd_out.candidate_trace.push(CandidateTraceRecord {
                    depth: node.depth + 1,
                    parent_smiles: target_smi.clone(),
                    template_id: entry.template_id.clone(),
                    rule_name: entry.rule_name.clone(),
                    provenance: classify_provenance(&entry.template_id, smirks),
                    precursor_signature,
                    f_score: node.g + entry.step_cost + new_h,
                    rank_before_prune: None,
                    survived_beam: true,
                    later_reached_stock: false,
                });
                Some(id)
            });

            heap.push(Node {
                frontier: new_frontier,
                path: new_path,
                depth: node.depth + 1,
                g: node.g + entry.step_cost,
                h: new_h,
                trace_id,
                family_key: Some(entry.template_id.clone()),
            });
        }

        // --- Phase 3.2: Beam search pruning ---
        if config.beam_width > 0 && heap.len() > config.beam_width {
            beam_limit_hit = true;
        }
        let (eviction_stats, trace_ranks, diversity_stats) = beam_prune(
            &mut heap,
            config.beam_width,
            config.beam_diversity_policy,
            config.beam_diversity_slots,
        );
        if let Some((evicted_n, evicted_min, evicted_max, boundary)) = eviction_stats {
            crowd_out.beam_prune_invocations += 1;
            crowd_out.candidates_evicted_total += evicted_n as u64;
            crowd_out.evicted_f_min = Some(
                crowd_out
                    .evicted_f_min
                    .map_or(evicted_min, |m| m.min(evicted_min)),
            );
            crowd_out.evicted_f_max = Some(
                crowd_out
                    .evicted_f_max
                    .map_or(evicted_max, |m| m.max(evicted_max)),
            );
            crowd_out.final_beam_boundary_f = Some(boundary);
            crowd_out.beam_diversity_stats += diversity_stats;
        }
        for (trace_id, rank, survived) in trace_ranks {
            if let Some(record) = crowd_out.candidate_trace.get_mut(trace_id as usize) {
                record.rank_before_prune = Some(rank);
                record.survived_beam = survived;
            }
        }
    }

    // Phase 1B post-processing: mark each traced candidate that ended up
    // part of a route this search actually returned. Cheap even for a full
    // trace (`candidate_trace_cap` records against `routes.len() <=
    // config.max_routes` steps) -- skipped entirely when nothing was traced.
    if !crowd_out.candidate_trace.is_empty() {
        // Keyed on owned, *sorted* precursor signatures -- `ReactionStep::precursors`
        // preserves the template's original ordering, while
        // `CandidateTraceRecord::precursor_signature` is sorted at creation, so
        // both sides must be normalized the same way to compare equal.
        let mut solved_steps: FxHashSet<(String, String, Vec<String>)> = FxHashSet::default();
        for route in &routes {
            for step in &route.steps {
                let mut sig = step.precursors.clone();
                sig.sort_unstable();
                solved_steps.insert((step.target.clone(), step.template_id.clone(), sig));
            }
        }
        for record in &mut crowd_out.candidate_trace {
            let key = (
                record.parent_smiles.clone(),
                record.template_id.clone(),
                record.precursor_signature.clone(),
            );
            record.later_reached_stock = solved_steps.contains(&key);
        }
    }

    // Post-processing: confidence, atom economy, convergency.
    {
        let rule_weights: FxHashMap<&str, f64> =
            rules.iter().map(|r| (r.name.as_str(), r.weight)).collect();
        for route in &mut routes {
            let min_w = route
                .steps
                .iter()
                .map(|s| rule_weights.get(s.rule.as_str()).copied().unwrap_or(1.0))
                .fold(f64::INFINITY, f64::min);
            route.confidence = if min_w.is_infinite() {
                1.0
            } else {
                (min_w / max_rule_weight).clamp(0.0, 1.0)
            };

            for step in &mut route.steps {
                let w = rule_weights.get(step.rule.as_str()).copied().unwrap_or(1.0);
                step.step_confidence = (w / max_rule_weight).clamp(0.0, 1.0);

                let raw = compute_atom_economy_raw(&step.target, &step.precursors);
                let (status, display) = classify_atom_economy(raw);
                step.atom_economy_raw_percent = raw;
                step.atom_economy_status = status;
                step.atom_economy = display;

                step.evidence = config
                    .template_metadata
                    .as_ref()
                    .and_then(|m| m.get(&step.template_id))
                    .and_then(|e| e.to_step_evidence(&step.target, &step.precursors));
            }

            route.success_probability = route
                .steps
                .iter()
                .map(|s| s.step_confidence)
                .product::<f64>()
                .clamp(0.0, 1.0);

            route.convergency = convergency_score(&route.steps);
            route.route_cost = compute_route_cost(route, config.bb_price_map.as_ref());
        }
    }

    if config.forbidden_elements != 0 {
        let mask = config.forbidden_elements;
        routes.retain(|route| {
            let all_targets: std::collections::HashSet<&str> =
                route.steps.iter().map(|s| s.target.as_str()).collect();
            route.steps.iter().all(|step| {
                step.precursors.iter().all(|prec| {
                    all_targets.contains(prec.as_str()) || (elem_mask_from_smiles(prec) & mask) == 0
                })
            })
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    if config.verbose {
        eprintln!(
            "[renkin] search complete\n  nodes popped   : {}\n  nodes expanded : {}\n  routes found   : {}\n  retro cache    : {}/{} hits ({:.0}%)\n  elapsed        : {:.2} s",
            nodes_popped,
            nodes_expanded,
            routes.len(),
            retro_cache_hits,
            retro_cache_hits + retro_cache_misses,
            if retro_cache_hits + retro_cache_misses > 0 {
                retro_cache_hits as f64 / (retro_cache_hits + retro_cache_misses) as f64 * 100.0
            } else {
                0.0
            },
            t0.elapsed().as_secs_f64()
        );
        if !matches!(
            config.ring_context,
            crate::ring_context::RingContextConfig::Disabled
        ) {
            eprintln!(
                "[renkin] ring_context_diagnostics: {}",
                serde_json::to_string(&ring_context_diagnostics).unwrap_or_default()
            );
        }
    }

    if config.required_element_present != 0 {
        let need = config.required_element_present;
        routes.retain(|route| {
            let all_targets: std::collections::HashSet<&str> =
                route.steps.iter().map(|s| s.target.as_str()).collect();
            let leaf_union: u64 = route
                .steps
                .iter()
                .flat_map(|s| s.precursors.iter())
                .filter(|p| !all_targets.contains(p.as_str()))
                .fold(0u64, |acc, p| acc | elem_mask_from_smiles(p));
            (leaf_union & need) == need
        });
    }

    Ok(SearchRunResult {
        routes,
        stats: SearchStats {
            nodes_expanded,
            max_depth_reached,
            beam_limit_hit,
            matched_templates,
            stock_hits,
            retro_cache_hits,
            retro_cache_misses,
            ring_context_diagnostics,
            crowd_out,
            route_integrity,
            reranker_failures,
        },
        termination,
    })
}

/// Find retrosynthetic routes for `target_smiles`, unchanged since before
/// [`find_routes_with_control`] existed -- this signature, and every byte of
/// its output, is untouched by the addition of cooperative cancellation.
/// A thin wrapper over [`find_routes_with_control`] with
/// [`SearchControl::unlimited`]; there is no separate search implementation
/// behind this name.
pub fn find_routes(
    target_smiles: &str,
    env: &ChemEnv,
    rules: &[RetroRule],
    config: &SearchConfig,
) -> Result<(Vec<Route>, SearchStats)> {
    let result = find_routes_with_control(
        target_smiles,
        env,
        rules,
        config,
        &SearchControl::unlimited(),
    )?;
    Ok((result.routes, result.stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::apply_retro;
    use crate::chem_env::{ChemEnv, default_rules};

    fn aspirin_env() -> ChemEnv {
        ChemEnv::load("data/building_blocks.smi").unwrap_or_else(|_| {
            ChemEnv::in_memory(&["CC(=O)O", "Oc1ccccc1C(=O)O", "c1ccccc1C(=O)O", "C", "O"])
        })
    }

    fn cfg(depth: u32) -> SearchConfig {
        SearchConfig {
            max_depth: depth,
            max_routes: 5,
            beam_width: 0,
            ..Default::default()
        }
    }

    #[test]
    fn aspirin_finds_route_depth1() {
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        assert!(
            !routes.is_empty(),
            "must find at least one route for aspirin"
        );
        assert!(
            routes.iter().any(|r| r.depth <= 2),
            "must find a route with depth ≤ 2"
        );
    }

    #[test]
    fn building_block_target_returns_depth0() {
        let env = aspirin_env();
        let rules = default_rules();
        // Acetic acid is a building block → expect a depth-0 route (empty steps).
        let routes = find_routes("CC(=O)O", &env, &rules, &cfg(2)).unwrap().0;
        assert!(
            routes.iter().any(|r| r.depth == 0),
            "building block must return depth-0 route"
        );
    }

    #[test]
    fn anthranilic_acid_recognized_as_bb() {
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("c1ccc(N)cc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        assert!(
            routes.iter().any(|r| r.depth == 0),
            "anthranilic acid is in building blocks"
        );
    }

    #[test]
    fn beam_width_limits_does_not_panic() {
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_beam = SearchConfig {
            max_depth: 3,
            max_routes: 3,
            beam_width: 10,
            ..Default::default()
        };
        // With a very tight beam, search may find fewer routes but must not panic.
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam);
        assert!(routes.is_ok());
    }

    #[test]
    fn no_routes_for_unknown_target_within_depth() {
        let env = ChemEnv::in_memory(&["O"]); // only water as BB
        let rules = default_rules();
        // Aspirin with depth=1 and only water as BB: unlikely to fully solve.
        // At minimum should return the trivially solved (depth=0) only if aspirin IS water (it isn't).
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(1))
            .unwrap()
            .0;
        // depth=0 not possible (aspirin ≠ water); we just check it doesn't panic.
        let _ = routes;
    }

    // ── Layer 3: search behaviour tests ──────────────────────────────────────

    #[test]
    fn invalid_smiles_returns_err() {
        let env = aspirin_env();
        let rules = default_rules();
        // Unclosed bracket is guaranteed to be rejected by SMILES parsers.
        let result = find_routes("[C(", &env, &rules, &cfg(3));
        assert!(result.is_err(), "invalid SMILES must return Err");
    }

    #[test]
    fn max_depth_one_caps_all_routes() {
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(1))
            .unwrap()
            .0;
        // No route should exceed depth=1 when max_depth=1.
        for r in &routes {
            assert!(
                r.depth <= 1,
                "route with depth {} exceeds max_depth=1",
                r.depth
            );
        }
    }

    #[test]
    fn beam_width_one_does_not_exceed_unrestricted() {
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_beam = SearchConfig {
            max_depth: 3,
            max_routes: 10,
            beam_width: 1,
            ..Default::default()
        };
        let cfg_full = SearchConfig {
            max_depth: 3,
            max_routes: 10,
            beam_width: 0,
            ..Default::default()
        };
        let routes_beam = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam)
            .unwrap()
            .0;
        let routes_full = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_full)
            .unwrap()
            .0;
        assert!(
            routes_beam.len() <= routes_full.len(),
            "beam=1 ({}) should find ≤ routes than beam=0 ({})",
            routes_beam.len(),
            routes_full.len()
        );
    }

    // ── Crowd-out diagnostics tests (Issue #101) ─────────────────────────────

    fn node(f: f64) -> Node {
        Node {
            frontier: smallvec![FEntry {
                smiles: "C".to_string(),
            }],
            path: None,
            depth: 0,
            g: f,
            h: 0.0,
            trace_id: None,
            family_key: None,
        }
    }

    fn traced_node(f: f64, trace_id: u64) -> Node {
        Node {
            trace_id: Some(trace_id),
            ..node(f)
        }
    }

    fn family_node(f: f64, family: &str) -> Node {
        Node {
            family_key: Some(family.to_string()),
            ..node(f)
        }
    }

    fn traced_family_node(f: f64, family: &str, trace_id: u64) -> Node {
        Node {
            family_key: Some(family.to_string()),
            trace_id: Some(trace_id),
            ..node(f)
        }
    }

    // ── select_beam_survivors tests (diversity-reserved beam, rollout
    //    stage 2 -- see docs/design/diversity-reserved-beam-v0.md) ─────────

    #[test]
    fn select_beam_survivors_off_matches_pure_top_k_regardless_of_family() {
        // 3 nodes all family "A" (best scores) + 1 node family "B" (worst) --
        // Off must return exactly the top 3 by score, ignoring family
        // entirely, exactly like beam_prune does today.
        let nodes = vec![
            family_node(1.0, "A"),
            family_node(2.0, "A"),
            family_node(3.0, "A"),
            family_node(4.0, "B"),
        ];
        let (survivors, stats) = select_beam_survivors(nodes, 3, 1, BeamDiversityPolicy::Off);
        let mut fs: Vec<f64> = survivors.iter().map(Node::f).collect();
        fs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(fs, vec![1.0, 2.0, 3.0]);
        assert_eq!(stats, DiversityReservationStats::default());
    }

    #[test]
    fn select_beam_survivors_diagnostics_only_never_changes_selection() {
        // Same contrived case as the rescue test below, but DiagnosticsOnly
        // must return byte-for-byte the same survivor set as Off, even
        // though a rescue is clearly available.
        let nodes = vec![
            family_node(1.0, "A"),
            family_node(2.0, "A"),
            family_node(3.0, "A"),
            family_node(4.0, "B"),
        ];
        let (off_survivors, _) =
            select_beam_survivors(nodes.clone(), 3, 1, BeamDiversityPolicy::Off);
        let (diag_survivors, diag_stats) =
            select_beam_survivors(nodes, 3, 1, BeamDiversityPolicy::DiagnosticsOnly);
        let mut off_fs: Vec<f64> = off_survivors.iter().map(Node::f).collect();
        let mut diag_fs: Vec<f64> = diag_survivors.iter().map(Node::f).collect();
        off_fs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        diag_fs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            off_fs, diag_fs,
            "DiagnosticsOnly must never change selection"
        );
        // But it must still have computed what a rescue WOULD do.
        assert_eq!(diag_stats.families_rescued_by_reservation, 1);
    }

    #[test]
    fn select_beam_survivors_active_rescues_underrepresented_family() {
        // beam_width=3, diversity_slots=1 -> score_slots=2. Pure score
        // would keep the 2 best (family A, f=1.0/2.0), fully excluding
        // family B (f=4.0) and the 3rd-best family A (f=3.0). Active must
        // reserve 1 slot for the best-scoring not-yet-represented family,
        // which is B (f=4.0) -- not the weaker 3rd-place A, even though
        // it's a better score, because A is already represented.
        let nodes = vec![
            family_node(1.0, "A"),
            family_node(2.0, "A"),
            family_node(3.0, "A"),
            family_node(4.0, "B"),
        ];
        let (survivors, stats) = select_beam_survivors(nodes, 3, 1, BeamDiversityPolicy::Active);
        let mut fs: Vec<f64> = survivors.iter().map(Node::f).collect();
        fs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(fs, vec![1.0, 2.0, 4.0], "family B must be rescued");
        assert_eq!(stats.families_represented_by_score_alone, 1);
        assert_eq!(stats.families_rescued_by_reservation, 1);
    }

    #[test]
    fn select_beam_survivors_nodes_without_family_key_never_diversity_selected() {
        // The remainder pool's only candidate has no family_key (e.g. a
        // root-shaped node) -- it must never be picked for a diversity
        // slot, even though it would technically be "underrepresented."
        let nodes = vec![
            family_node(1.0, "A"),
            family_node(2.0, "A"),
            node(3.0), // family_key: None
        ];
        let (survivors, stats) = select_beam_survivors(nodes, 2, 1, BeamDiversityPolicy::Active);
        let mut fs: Vec<f64> = survivors.iter().map(Node::f).collect();
        fs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(fs, vec![1.0], "score_slots=1 keeps only the best A");
        assert_eq!(stats.families_rescued_by_reservation, 0);
    }

    #[test]
    fn select_beam_survivors_diversity_slots_clamped_to_beam_width() {
        // diversity_slots > beam_width must not panic or under-flow
        // (beam_width - diversity_slots).
        let nodes = vec![family_node(1.0, "A"), family_node(2.0, "B")];
        let (survivors, _) = select_beam_survivors(nodes, 1, 5, BeamDiversityPolicy::Active);
        assert_eq!(survivors.len(), 1);
    }

    #[test]
    fn select_beam_survivors_fewer_nodes_than_beam_width_returns_all() {
        let nodes = vec![family_node(1.0, "A"), family_node(2.0, "B")];
        for policy in [
            BeamDiversityPolicy::Off,
            BeamDiversityPolicy::DiagnosticsOnly,
            BeamDiversityPolicy::Active,
        ] {
            let (survivors, _) = select_beam_survivors(nodes.clone(), 10, 2, policy);
            assert_eq!(survivors.len(), 2, "{policy:?}");
        }
    }

    #[test]
    fn select_beam_survivors_rescued_node_trace_ids_only_covers_traced_nodes() {
        // beam_width=2, diversity_slots=1 -> score_slots=1: family "A"
        // (f=1.0) already fills the one score slot, leaving the traced "B"
        // node (f=2.0) as the only, and therefore rescued, remainder
        // candidate.
        let nodes = vec![family_node(1.0, "A"), traced_family_node(2.0, "B", 42)];
        let (survivors, stats) = select_beam_survivors(nodes, 2, 1, BeamDiversityPolicy::Active);
        assert_eq!(survivors.len(), 2);
        assert_eq!(stats.rescued_node_trace_ids, vec![42]);
    }

    #[test]
    fn beam_prune_returns_none_when_beam_width_zero() {
        let mut heap: BinaryHeap<Node> = (0..5).map(|i| node(i as f64)).collect();
        let (stats, trace_ranks, _) = beam_prune(&mut heap, 0, BeamDiversityPolicy::Off, 0);
        assert_eq!(stats, None);
        assert!(trace_ranks.is_empty());
        assert_eq!(heap.len(), 5, "beam_width=0 must not truncate");
    }

    #[test]
    fn beam_prune_returns_none_when_heap_within_beam_width() {
        let mut heap: BinaryHeap<Node> = (0..3).map(|i| node(i as f64)).collect();
        let (stats, trace_ranks, _) = beam_prune(&mut heap, 10, BeamDiversityPolicy::Off, 0);
        assert_eq!(stats, None);
        assert!(trace_ranks.is_empty());
        assert_eq!(heap.len(), 3);
    }

    #[test]
    fn beam_prune_reports_exact_eviction_stats() {
        // f = 0.0, 1.0, 2.0, 3.0, 4.0 -- keep the best 2 (lowest f).
        let mut heap: BinaryHeap<Node> = (0..5).map(|i| node(i as f64)).collect();
        let (evicted_n, evicted_min, evicted_max, boundary) =
            beam_prune(&mut heap, 2, BeamDiversityPolicy::Off, 0)
                .0
                .unwrap();
        assert_eq!(evicted_n, 3, "5 nodes - beam_width 2 = 3 evicted");
        assert_eq!(evicted_min, 2.0, "lowest f among the evicted (f=2,3,4)");
        assert_eq!(evicted_max, 4.0, "highest f among the evicted");
        assert_eq!(boundary, 1.0, "f of the worst *retained* node (f=0,1)");
        assert_eq!(heap.len(), 2);
        // Retained nodes must indeed be the two lowest-f ones.
        let mut retained: Vec<f64> = heap.iter().map(Node::f).collect();
        retained.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(retained, vec![0.0, 1.0]);
    }

    #[test]
    fn beam_prune_reports_survived_and_evicted_trace_ranks() {
        // f = 0,1,2,3,4; beam_width=2 keeps rank 0,1 (f=0,1), evicts rank 2,3,4.
        let mut heap: BinaryHeap<Node> = vec![
            traced_node(0.0, 100),
            traced_node(2.0, 101),
            node(1.0),
            node(3.0),
            node(4.0),
        ]
        .into_iter()
        .collect();
        let (_, trace_ranks, _) = beam_prune(&mut heap, 2, BeamDiversityPolicy::Off, 0);
        let mut by_id: FxHashMap<u64, (usize, bool)> = trace_ranks
            .into_iter()
            .map(|(id, rank, survived)| (id, (rank, survived)))
            .collect();
        assert_eq!(by_id.remove(&100), Some((0, true)), "f=0.0 -> rank 0, kept");
        assert_eq!(
            by_id.remove(&101),
            Some((2, false)),
            "f=2.0 -> rank 2, evicted (beam_width=2)"
        );
    }

    #[test]
    fn beam_prune_reports_no_trace_ranks_when_nothing_evicted() {
        // No truncation means no sort, so there is no real rank to report --
        // a traced node here must keep its as-created default rather than
        // being reported as a fabricated rank 0.
        let mut heap: BinaryHeap<Node> = vec![traced_node(0.0, 7), node(1.0)].into_iter().collect();
        let (stats, trace_ranks, _) = beam_prune(&mut heap, 10, BeamDiversityPolicy::Off, 0);
        assert_eq!(stats, None, "heap smaller than beam_width -> no eviction");
        assert!(trace_ranks.is_empty());
    }

    // ── beam_prune integration tests for BeamDiversityPolicy (rollout
    //    stage 3 -- wiring select_beam_survivors into beam_prune itself) ──

    fn crowded_family_heap() -> BinaryHeap<Node> {
        // 3 nodes family "A" (best scores) + 1 node family "B" (worst) --
        // pure top-3 excludes B entirely; a diversity slot should rescue it.
        vec![
            family_node(1.0, "A"),
            family_node(2.0, "A"),
            family_node(3.0, "A"),
            family_node(4.0, "B"),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn beam_prune_diagnostics_only_matches_off_survivor_set_exactly() {
        let mut off_heap = crowded_family_heap();
        let (off_stats, _, off_diversity) =
            beam_prune(&mut off_heap, 3, BeamDiversityPolicy::Off, 1);

        let mut diag_heap = crowded_family_heap();
        let (diag_stats, _, diag_diversity) =
            beam_prune(&mut diag_heap, 3, BeamDiversityPolicy::DiagnosticsOnly, 1);

        let mut off_fs: Vec<f64> = off_heap.iter().map(Node::f).collect();
        let mut diag_fs: Vec<f64> = diag_heap.iter().map(Node::f).collect();
        off_fs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        diag_fs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            off_fs, diag_fs,
            "DiagnosticsOnly must never change beam_prune's actual survivors"
        );
        assert_eq!(
            off_stats, diag_stats,
            "eviction bookkeeping stays identical too"
        );
        assert_eq!(off_diversity, DiversityReservationStats::default());
        assert_eq!(diag_diversity.families_rescued_by_reservation, 1);
    }

    #[test]
    fn beam_prune_active_rescues_underrepresented_family() {
        let mut heap = crowded_family_heap();
        let (_, _, diversity) = beam_prune(&mut heap, 3, BeamDiversityPolicy::Active, 1);
        let mut fs: Vec<f64> = heap.iter().map(Node::f).collect();
        fs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(fs, vec![1.0, 2.0, 4.0], "family B (f=4.0) must be rescued");
        assert_eq!(diversity.families_rescued_by_reservation, 1);
    }

    #[test]
    fn dedup_counts_ignores_same_template_repeats_for_cross_template_duplicates() {
        let entries = vec![
            RetroEntry {
                rule_name: "extracted_1".to_string(),
                template_id: "smirks-sha256:aaa".to_string(),
                step_cost: 1.0,
                precursor_smiles: vec!["CC".to_string(), "O".to_string()],
            },
            RetroEntry {
                rule_name: "extracted_1".to_string(),
                template_id: "smirks-sha256:aaa".to_string(),
                step_cost: 1.0,
                precursor_smiles: vec!["O".to_string(), "CC".to_string()],
            },
        ];
        let (cross_dup, after_same_template, after_cross_template) = dedup_counts(&entries);
        assert_eq!(
            cross_dup, 0,
            "identical signature from the SAME template is not cross-template duplication"
        );
        assert_eq!(
            after_same_template, 1,
            "both entries collapse to one (template_id, signature) pair"
        );
        assert_eq!(after_cross_template, 1);
    }

    #[test]
    fn dedup_counts_detects_cross_template_collision() {
        let entries = vec![
            RetroEntry {
                rule_name: "extracted_1".to_string(),
                template_id: "smirks-sha256:aaa".to_string(),
                step_cost: 1.0,
                precursor_smiles: vec!["CC".to_string(), "O".to_string()],
            },
            RetroEntry {
                rule_name: "extracted_2".to_string(),
                template_id: "smirks-sha256:bbb".to_string(),
                // Same multiset, different order -- must still collide (sorted signature).
                precursor_smiles: vec!["O".to_string(), "CC".to_string()],
                step_cost: 1.0,
            },
            RetroEntry {
                rule_name: "extracted_3".to_string(),
                template_id: "smirks-sha256:ccc".to_string(),
                precursor_smiles: vec!["N".to_string()],
                step_cost: 1.0,
            },
        ];
        let (cross_dup, after_same_template, after_cross_template) = dedup_counts(&entries);
        assert_eq!(
            cross_dup, 1,
            "extracted_2 duplicates extracted_1's signature; extracted_3 is distinct"
        );
        assert_eq!(
            after_same_template, 3,
            "all 3 have distinct (template_id, signature) pairs"
        );
        assert_eq!(
            after_cross_template, 2,
            "extracted_1/extracted_2 share one signature; extracted_3 is the second"
        );
    }

    #[test]
    fn classify_provenance_distinguishes_handcrafted_file_backed_and_hash_atom() {
        assert_eq!(
            classify_provenance("rule:esterification", "[C:1](=O)O.[O:2]>>..."),
            CandidateProvenance::Handcrafted
        );
        assert_eq!(
            classify_provenance("smirks-sha256:abc", "[C:1](=O)O.[O:2]>>..."),
            CandidateProvenance::FileBacked
        );
        assert_eq!(
            classify_provenance("smirks-sha256:abc", "[#7:2]:[c:1]>>..."),
            CandidateProvenance::HashAtom
        );
    }

    #[test]
    fn crowd_out_diagnostics_default_off_when_beam_unlimited() {
        let env = aspirin_env();
        let rules = default_rules();
        let (_, stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
        assert_eq!(stats.crowd_out.beam_prune_invocations, 0);
        assert_eq!(stats.crowd_out.candidates_evicted_total, 0);
        assert_eq!(stats.crowd_out.evicted_f_min, None);
        assert_eq!(stats.crowd_out.evicted_f_max, None);
        assert_eq!(stats.crowd_out.final_beam_boundary_f, None);
    }

    #[test]
    fn crowd_out_diagnostics_records_eviction_under_tight_beam() {
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_beam = SearchConfig {
            max_depth: 3,
            max_routes: 3,
            beam_width: 1,
            ..Default::default()
        };
        let (_, stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam).unwrap();
        assert!(
            stats.crowd_out.beam_prune_invocations > 0,
            "beam_width=1 on a multi-rule target must trigger at least one prune"
        );
        assert!(stats.crowd_out.candidates_evicted_total > 0);
        // evicted_f_min/max are running aggregates across every beam_prune
        // invocation in the search (g grows with depth, so an early, cheap
        // prune's evicted_min can legitimately be lower than a later, deeper
        // prune's boundary -- these are not from the same invocation, so
        // only self-consistency is checked here; see
        // `beam_prune_reports_exact_eviction_stats` for the single-invocation
        // ordering guarantee).
        let evicted_min = stats.crowd_out.evicted_f_min.expect("must be Some");
        let evicted_max = stats.crowd_out.evicted_f_max.expect("must be Some");
        assert!(evicted_min <= evicted_max);
        assert!(stats.crowd_out.final_beam_boundary_f.is_some());
    }

    #[test]
    fn crowd_out_diagnostics_stock_and_non_stock_candidates_are_counted() {
        let env = aspirin_env();
        let rules = default_rules();
        let (_, stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
        assert!(
            stats.crowd_out.stock_terminal_candidates > 0,
            "aspirin's search must encounter stock hits (acetic/salicylic acid)"
        );
        assert!(
            stats.crowd_out.stock_terminal_candidates + stats.crowd_out.non_stock_candidates > 0
        );
        assert!(stats.crowd_out.rules_attempted_total > 0);
    }

    #[test]
    fn crowd_out_diagnostics_branching_by_depth_sums_match_top_level_stats() {
        let env = aspirin_env();
        let rules = default_rules();
        let (_, stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
        assert!(!stats.crowd_out.branching_by_depth.is_empty());
        let sum_expanded: u64 = stats
            .crowd_out
            .branching_by_depth
            .values()
            .map(|d| d.nodes_expanded)
            .sum();
        let sum_children: u64 = stats
            .crowd_out
            .branching_by_depth
            .values()
            .map(|d| d.children_produced)
            .sum();
        assert_eq!(
            sum_expanded, stats.nodes_expanded,
            "per-depth nodes_expanded must sum to the top-level total"
        );
        assert_eq!(
            sum_children, stats.matched_templates,
            "per-depth children_produced must sum to matched_templates \
             (both are bumped at the same call site)"
        );
    }

    #[test]
    fn crowd_out_diagnostics_are_deterministic_across_repeated_runs() {
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_beam = SearchConfig {
            max_depth: 3,
            max_routes: 3,
            beam_width: 2,
            ..Default::default()
        };
        let (_, stats1) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam).unwrap();
        let (_, stats2) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam).unwrap();
        let j1 = serde_json::to_string(&stats1.crowd_out).unwrap();
        let j2 = serde_json::to_string(&stats2.crowd_out).unwrap();
        assert_eq!(
            j1, j2,
            "identical inputs must yield byte-identical diagnostics"
        );
    }

    #[test]
    fn timing_diagnostics_defaults_to_zero() {
        let env = aspirin_env();
        let rules = default_rules();
        let (_, stats) =
            find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).expect("search runs");
        assert_eq!(
            stats.crowd_out.retro_expansion_wall_time_us, 0,
            "must stay 0 -- and therefore deterministic across repeated runs -- unless \
             SearchConfig::timing_diagnostics is explicitly opted into"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn timing_diagnostics_opt_in_records_real_time() {
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_timed = SearchConfig {
            timing_diagnostics: true,
            ..cfg(3)
        };
        let (_, stats) =
            find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_timed).expect("search runs");
        assert!(
            stats.retro_cache_misses > 0,
            "sanity check: this target must produce at least one retro-cache-miss \
             expansion for this test to be meaningful"
        );
        assert!(
            stats.crowd_out.retro_expansion_wall_time_us > 0,
            "opted into timing_diagnostics with a real cache-miss expansion, so real \
             elapsed time must have been recorded: {:?}",
            stats.crowd_out
        );
    }

    #[test]
    fn spectator_bond_policy_off_by_default_produces_empty() {
        let env = aspirin_env();
        let rules = default_rules();
        let (_, stats) =
            find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).expect("search runs");
        assert!(
            stats.crowd_out.spectator_bond_loss_findings.is_empty(),
            "must stay empty unless SearchConfig::spectator_bond_policy is explicitly opted \
             into: {:?}",
            stats.crowd_out.spectator_bond_loss_findings
        );
        assert!(stats.crowd_out.spectator_bond_gated_out.is_empty());
    }

    #[test]
    fn spectator_bond_policy_diagnostics_only_runs_without_error_on_default_rules() {
        // default_rules() are hand-crafted and not expected to exhibit
        // spectator bond loss themselves (real positive controls are all
        // extracted templates, covered end-to-end by candidate.rs's
        // raw_propose_spectator_bond_policy_diagnostics_only_detects_known_positive_control).
        // This test only confirms the opt-in policy is wired all the way
        // through find_routes without panicking or changing route output.
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_diag = SearchConfig {
            spectator_bond_policy: SpectatorBondPolicy::DiagnosticsOnly,
            ..cfg(3)
        };
        let (routes_on, stats_on) =
            find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_diag).expect("search runs");
        let (routes_off, _) =
            find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).expect("search runs");
        assert_eq!(
            routes_on.len(),
            routes_off.len(),
            "opting into spectator_bond_policy: DiagnosticsOnly must never change which routes \
             are found"
        );
        assert!(stats_on.crowd_out.spectator_bond_loss_findings.is_empty());
        assert!(stats_on.crowd_out.spectator_bond_gated_out.is_empty());
    }

    #[test]
    fn spectator_bond_policy_gated_runs_without_error_on_default_rules() {
        // Same rationale as the DiagnosticsOnly test above: default_rules()
        // exhibit no known spectator-bond defect on this target, so Gated
        // must be a pure no-op here too -- this only confirms the policy is
        // wired through without panicking or changing route output when
        // there is nothing to gate. Gated's actual exclusion behavior is
        // covered end-to-end by candidate.rs's
        // raw_propose_spectator_bond_policy_gated_excludes_known_positive_control.
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_gated = SearchConfig {
            spectator_bond_policy: SpectatorBondPolicy::Gated,
            ..cfg(3)
        };
        let (routes_gated, stats_gated) =
            find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_gated).expect("search runs");
        let (routes_off, _) =
            find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).expect("search runs");
        assert_eq!(
            routes_gated.len(),
            routes_off.len(),
            "Gated must never change route output when nothing it can evaluate is defective"
        );
        assert!(stats_gated.crowd_out.spectator_bond_gated_out.is_empty());
    }

    #[test]
    fn route_steps_are_populated() {
        // Non-BB target must produce routes whose steps are non-empty.
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        let non_zero: Vec<_> = routes.iter().filter(|r| r.depth > 0).collect();
        assert!(
            !non_zero.is_empty(),
            "must find at least one multi-step route"
        );
        for r in non_zero {
            assert!(
                !r.steps.is_empty(),
                "route with depth>0 must have non-empty steps"
            );
            for step in &r.steps {
                assert!(!step.rule.is_empty(), "step.rule must be non-empty");
                assert!(!step.target.is_empty(), "step.target must be non-empty");
                assert!(
                    !step.precursors.is_empty(),
                    "step.precursors must be non-empty"
                );
            }
        }
    }

    #[test]
    fn is_extracted_template_detects_name_prefix_only() {
        assert!(is_extracted_template("extracted_0"));
        assert!(is_extracted_template("extracted_1234"));
        assert!(!is_extracted_template("suzuki_retro"));
        assert!(!is_extracted_template("cc_single_cleavage"));
    }

    #[test]
    fn absent_metadata_fields_are_omitted_from_json() {
        // An extracted-template-shaped step (metadata_source/scope both None) must
        // serialize with neither key present, so pre-existing JSON consumers see no
        // change from before these fields were added.
        let step = ReactionStep {
            rule: "extracted_0".to_string(),
            template_id: "smirks-sha256:deadbeef".to_string(),
            target: "CC(=O)O".to_string(),
            precursors: vec!["C".to_string(), "O=C=O".to_string()],
            conditions: None,
            atom_economy: None,
            atom_economy_raw_percent: None,
            atom_economy_status: AtomEconomyStatus::NotEvaluable,
            step_confidence: 0.5,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        };
        let json = serde_json::to_string(&step).unwrap();
        assert!(
            !json.contains("metadata_source")
                && !json.contains("metadata_scope")
                && !json.contains("evidence"),
            "absent metadata fields must be omitted from JSON, got: {json}"
        );
    }

    // ── Issue #79: atom_economy must never silently clamp ──────────────

    #[test]
    fn classify_atom_economy_normal_case_unchanged() {
        let (status, display) = classify_atom_economy(Some(87.5));
        assert_eq!(status, AtomEconomyStatus::Normal);
        assert_eq!(display, Some(87.5));
    }

    #[test]
    fn classify_atom_economy_exactly_100_is_normal() {
        let (status, display) = classify_atom_economy(Some(100.0));
        assert_eq!(status, AtomEconomyStatus::Normal);
        assert_eq!(display, Some(100.0));
    }

    #[test]
    fn classify_atom_economy_above_range_is_never_clamped_into_display() {
        // The historical bug: (raw).min(100.0) silently turned 183.4 into
        // 100.0, making a route with an unrepresented mass gap look like a
        // perfect one.
        let (status, display) = classify_atom_economy(Some(183.4));
        assert_eq!(status, AtomEconomyStatus::AboveExpectedRange);
        assert_eq!(
            display, None,
            "a ratio above the expected range must never be reported as a display value, clamped or otherwise"
        );
    }

    #[test]
    fn classify_atom_economy_not_evaluable_when_no_raw_ratio() {
        let (status, display) = classify_atom_economy(None);
        assert_eq!(status, AtomEconomyStatus::NotEvaluable);
        assert_eq!(display, None);
    }

    #[test]
    fn classify_atom_economy_nan_is_not_evaluable() {
        let (status, display) = classify_atom_economy(Some(f64::NAN));
        assert_eq!(status, AtomEconomyStatus::NotEvaluable);
        assert_eq!(display, None);
    }

    #[test]
    fn classify_atom_economy_positive_infinity_is_not_evaluable() {
        let (status, display) = classify_atom_economy(Some(f64::INFINITY));
        assert_eq!(status, AtomEconomyStatus::NotEvaluable);
        assert_eq!(display, None);
    }

    #[test]
    fn classify_atom_economy_negative_infinity_is_not_evaluable() {
        let (status, display) = classify_atom_economy(Some(f64::NEG_INFINITY));
        assert_eq!(status, AtomEconomyStatus::NotEvaluable);
        assert_eq!(display, None);
    }

    // ── compute_atom_economy_raw: the all-or-nothing denominator ────────

    #[test]
    fn compute_raw_one_unparseable_precursor_is_not_evaluable() {
        // Must not silently drop the malformed entry and compute a ratio
        // over just the remaining (valid) precursor -- that would inflate
        // the ratio exactly the way the historical clamp did.
        let raw =
            compute_atom_economy_raw("CCO", &["not_a_smiles(((".to_string(), "C".to_string()]);
        assert_eq!(raw, None);
    }

    #[test]
    fn compute_raw_unparseable_target_is_not_evaluable() {
        let raw = compute_atom_economy_raw("not_a_smiles(((", &["CCO".to_string()]);
        assert_eq!(raw, None);
    }

    #[test]
    fn compute_raw_empty_precursors_is_not_evaluable() {
        // Also exercises the zero-denominator path: an empty precursor list
        // sums to a weight of exactly 0.0.
        let raw = compute_atom_economy_raw("CCO", &[]);
        assert_eq!(raw, None);
    }

    #[test]
    fn compute_raw_normal_case_matches_direct_molecular_weight_ratio() {
        let target_w = molecular_weight(&mol_from_smiles("CCO").unwrap());
        let precursor_w = molecular_weight(&mol_from_smiles("CC=O").unwrap());
        let raw = compute_atom_economy_raw("CCO", &["CC=O".to_string()]).unwrap();
        assert!((raw - target_w / precursor_w * 100.0).abs() < 1e-9);
    }

    #[test]
    fn compute_raw_reagent_omission_lands_above_expected_range() {
        // Retro step: target = cyclohexane, precursor = benzene (H2 omitted
        // from the precursor list, as a common reagent never is tracked).
        // Every heavy (carbon) atom the target needs is supplied, but the
        // precursor list is lighter than the target -- this must classify
        // as AboveExpectedRange, not silently pass as Normal or crash.
        let raw = compute_atom_economy_raw("C1CCCCC1", &["c1ccccc1".to_string()]).unwrap();
        assert!(raw > 100.0, "expected > 100%, got {raw}");
        let (status, display) = classify_atom_economy(Some(raw));
        assert_eq!(status, AtomEconomyStatus::AboveExpectedRange);
        assert_eq!(display, None);
    }

    #[test]
    fn above_range_step_omits_atom_economy_but_keeps_raw_and_status_in_json() {
        let raw = 183.4;
        let (status, display) = classify_atom_economy(Some(raw));
        let step = ReactionStep {
            rule: "extracted_0".to_string(),
            template_id: "smirks-sha256:deadbeef".to_string(),
            target: "CC(=O)O".to_string(),
            precursors: vec!["C".to_string()],
            conditions: None,
            atom_economy: display,
            atom_economy_raw_percent: Some(raw),
            atom_economy_status: status,
            step_confidence: 0.5,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        };
        let json = serde_json::to_string(&step).unwrap();
        assert!(
            !json.contains("\"atom_economy\":"),
            "atom_economy must be absent (never a clamped 100.0), got: {json}"
        );
        assert!(
            json.contains("\"atom_economy_raw_percent\":183.4"),
            "the honest raw ratio must still be reported, got: {json}"
        );
        assert!(
            json.contains("\"atom_economy_status\":\"above_expected_range\""),
            "got: {json}"
        );
    }

    #[test]
    fn compute_raw_precursor_excess_is_normal_well_under_100() {
        // Opposite direction: precursors heavier than the target (a leaving
        // group is dropped) is the ordinary, expected case, not a status of
        // its own.
        let raw = compute_atom_economy_raw("c1ccccc1", &["C1CCCCC1".to_string()]).unwrap();
        assert!(raw < 100.0, "expected < 100%, got {raw}");
        let (status, _) = classify_atom_economy(Some(raw));
        assert_eq!(status, AtomEconomyStatus::Normal);
    }

    #[test]
    fn atom_economy_fields_json_round_trip_by_status() {
        fn step_with(
            status: AtomEconomyStatus,
            display: Option<f64>,
            raw: Option<f64>,
        ) -> ReactionStep {
            ReactionStep {
                rule: "extracted_0".to_string(),
                template_id: "smirks-sha256:deadbeef".to_string(),
                target: "CC(=O)O".to_string(),
                precursors: vec!["C".to_string()],
                conditions: None,
                atom_economy: display,
                atom_economy_raw_percent: raw,
                atom_economy_status: status,
                step_confidence: 0.5,
                procedure_hint: None,
                reaction_family: None,
                metadata_source: None,
                metadata_scope: None,
                evidence: None,
            }
        }

        let normal = step_with(AtomEconomyStatus::Normal, Some(87.5), Some(87.5));
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&normal).unwrap()).unwrap();
        assert_eq!(v["atom_economy"], serde_json::json!(87.5));
        assert_eq!(v["atom_economy_raw_percent"], serde_json::json!(87.5));
        assert_eq!(v["atom_economy_status"], serde_json::json!("normal"));

        let above = step_with(AtomEconomyStatus::AboveExpectedRange, None, Some(183.4));
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&above).unwrap()).unwrap();
        assert!(v.get("atom_economy").is_none());
        assert_eq!(v["atom_economy_raw_percent"], serde_json::json!(183.4));
        assert_eq!(
            v["atom_economy_status"],
            serde_json::json!("above_expected_range")
        );

        let not_evaluable = step_with(AtomEconomyStatus::NotEvaluable, None, None);
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&not_evaluable).unwrap()).unwrap();
        assert!(v.get("atom_economy").is_none());
        assert!(v.get("atom_economy_raw_percent").is_none());
        assert_eq!(v["atom_economy_status"], serde_json::json!("not_evaluable"));
    }

    #[test]
    fn handcrafted_rule_step_is_tagged() {
        // default_rules() contains only hand-crafted rules (no extracted templates
        // loaded), so every step of every route found here must be hand-crafted.
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        let non_zero: Vec<_> = routes.iter().filter(|r| r.depth > 0).collect();
        assert!(
            !non_zero.is_empty(),
            "must find at least one multi-step route"
        );
        for r in non_zero {
            for step in &r.steps {
                assert_eq!(
                    step.metadata_source,
                    Some(MetadataSource::HandcraftedDefault),
                    "step using hand-crafted rule {:?} must be tagged HandcraftedDefault",
                    step.rule
                );
                assert_eq!(
                    step.metadata_scope,
                    Some(EvidenceScope::ReactionFamily),
                    "step using hand-crafted rule {:?} must be scoped ReactionFamily",
                    step.rule
                );
            }
        }
    }

    #[test]
    fn no_metadata_configured_means_no_evidence() {
        // config.template_metadata defaults to None -- every step.evidence must be
        // None, reproducing pre-existing (pre-evidence-sidecar) behavior exactly.
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        assert!(!routes.is_empty());
        for route in &routes {
            for step in &route.steps {
                assert!(
                    step.evidence.is_none(),
                    "no metadata sidecar configured -- step.evidence must stay None"
                );
                assert!(
                    !step.template_id.is_empty(),
                    "template_id must always be populated"
                );
            }
        }
    }

    #[test]
    fn evidence_attached_only_to_matching_template_id() {
        let env = aspirin_env();
        let rules = default_rules();
        let target_template_id = rules
            .iter()
            .find(|r| r.name == "ester_cleavage")
            .unwrap()
            .template_id
            .clone();

        let mut templates = std::collections::HashMap::new();
        templates.insert(
            target_template_id.clone(),
            crate::evidence::TemplateMetadataEntry {
                warnings: vec![crate::evidence::ReactionWarning {
                    code: "test_code".to_string(),
                    severity: crate::evidence::WarningSeverity::Low,
                    message: "test warning".to_string(),
                    source: MetadataSource::Literature,
                    scope: EvidenceScope::Template,
                    reference_ids: vec![],
                }],
                ..Default::default()
            },
        );
        let config = SearchConfig {
            template_metadata: Some(templates),
            ..cfg(3)
        };
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &config)
            .unwrap()
            .0;

        let mut saw_match = false;
        let mut saw_non_match = false;
        for route in &routes {
            for step in &route.steps {
                if step.template_id == target_template_id {
                    assert!(
                        step.evidence.is_some(),
                        "step using the metadata-matched template must get evidence"
                    );
                    saw_match = true;
                } else {
                    assert!(
                        step.evidence.is_none(),
                        "step using a non-matched template must not get evidence"
                    );
                    saw_non_match = true;
                }
            }
        }
        assert!(saw_match, "expected at least one step using ester_cleavage");
        assert!(
            saw_non_match,
            "expected at least one step using a different rule"
        );
    }

    #[test]
    fn symmetric_biaryl_routes_deduplicated() {
        // Biphenyl is symmetric: both orientations of Suzuki retro yield the same
        // precursor set {Brc1ccccc1, OB(O)c1ccccc1}. The search must dedup to ≤ 1 route.
        let env = ChemEnv::in_memory(&["Brc1ccccc1", "OB(O)c1ccccc1"]);
        let rules = default_rules();
        let cfg = SearchConfig {
            max_depth: 2,
            max_routes: 10,
            beam_width: 0,
            ..Default::default()
        };
        let routes = find_routes("c1ccc(-c2ccccc2)cc1", &env, &rules, &cfg)
            .unwrap()
            .0;
        // Both orientations resolve to identical BB sets — expect exactly 1 unique route.
        assert_eq!(
            routes.len(),
            1,
            "symmetric biphenyl should produce exactly 1 deduplicated route; got {}",
            routes.len()
        );
    }

    #[test]
    fn confidence_is_between_zero_and_one() {
        let env = aspirin_env();
        let rules = default_rules();
        let (routes, _) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
        assert!(!routes.is_empty(), "must find at least one route");
        for route in &routes {
            assert!(
                (0.0..=1.0).contains(&route.confidence),
                "confidence {} out of [0,1]",
                route.confidence
            );
        }
    }

    #[test]
    fn search_stats_nodes_expanded_nonzero() {
        let env = ChemEnv::in_memory(&["O"]); // only water — aspirin unsolvable
        let rules = default_rules();
        let (routes, stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(2)).unwrap();
        assert!(
            routes.is_empty(),
            "aspirin should be unsolvable with only water as BB"
        );
        assert!(
            stats.nodes_expanded > 0,
            "nodes_expanded must be > 0 even for failed search"
        );
    }

    #[test]
    fn avoid_elements_removes_forbidden_bbs() {
        let env = aspirin_env();
        let rules = default_rules();
        let config = SearchConfig {
            forbidden_elements: crate::chem_env::elem_symbols_to_mask("Cl"),
            ..cfg(3)
        };
        let (routes, _) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &config).unwrap();
        for route in &routes {
            for bb in &route.building_blocks {
                assert!(!bb.contains("Cl"), "BB {bb} contains forbidden element Cl");
            }
        }
    }

    #[test]
    fn find_routes_returns_stats_tuple() {
        let env = aspirin_env();
        let rules = default_rules();
        // Just verify the return type is a tuple and stats has a reasonable value.
        let (routes, stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
        assert!(!routes.is_empty());
        assert!(stats.nodes_expanded >= routes.len() as u64);
    }

    // ── E2 closed-set correctness: proven LATENT bug reproduction ───────────
    //
    // `closed: FxHashSet<u64>` is a boolean "already visited" set keyed only
    // by frontier molecule content (`state_hash`) — no `g` is stored, so
    // there is no reopen-on-lower-g. For a *consistent* heuristic, A* graph
    // search guarantees the first pop of a state has optimal `g`, so a plain
    // closed set is safe.
    //
    // IMPORTANT — this test is a LATENT bug demonstration, not a live one:
    // it requires an injected `ReactionPrior`/`MoleculeValueEstimator` (bonus
    // 5.0, h 100.0) to force the pop order needed. Every production entry
    // point (CLI, renkin-bench, Python, WASM) passes `reaction_prior: None`
    // / `value_estimator: None` today (grep confirms), so this exact
    // mechanism does not fire in current production runs. Separately, E4
    // (below) shows the *default* cost formula is already inadmissible
    // (net step cost can be 0.8 < the heuristic's assumed 1.0 floor) — but
    // algebraically, that bounded 0.2 gap can never make a longer path to
    // the same single-molecule state cheaper than a direct one (extra hop
    // costs >=0.8, max bonus saving is 0.2), so today's default config
    // cannot trigger *this specific* construction either. The risk is real
    // but currently dormant: `ReactionPrior`/`MoleculeValueEstimator` are
    // unbounded public hooks meant for future NN-based scoring (Track D/E3)
    // — the day one is wired up without a floor clamp, this closed set will
    // silently drop better paths in production.
    //
    // Minimal deterministic reproduction using the real `find_routes` (real
    // heap, real closed set, real chematic SMIRKS chemistry), with an
    // injected prior/estimator to force the exact pop order needed to prove
    // the mechanism:
    //
    //   T (ClCCI) --r_direct (bonus 0)--------------> M (BrCCBr)
    //   T (ClCCI) --r_step1  (bonus 5)--> Y (BrCCI)
    //                 Y      --r_step2  (bonus 5)--> M (BrCCBr)
    //   M --r_final--------------------------------> Z (FCCF, the only BB)
    //
    // h(Y) is set artificially high (100) so the direct T->M arrival pops
    // and closes state {M} *before* the much cheaper T->Y->M arrival is even
    // generated. When the cheaper arrival is later popped, it finds {M}
    // already closed and is discarded without expansion — the true-optimal
    // route (T->Y->M->Z, deeply negative g) is never found; only the worse
    // route (T->M->Z, g≈2.29) is returned.
    //
    // (RENKIN Bridge PR1: each rule below now also emits its displaced
    // halogen as an explicit byproduct fragment, per that test's own
    // comment further down -- this shifted every g value from the numbers
    // originally hand-derived for the E2 investigation below by a constant
    // per-hop offset, without changing the mechanism being demonstrated.
    // `best_score`'s new value is verified experimentally, not hand-derived;
    // the deeply-negative-optimum claim two paragraphs up is qualitative.)
    //
    // NOTE for whoever implements the E2 fix: this test asserts the CURRENT
    // (buggy) behavior and will start FAILING once the closed set reopens on
    // a lower g. At that point, invert the assertions below to pin the
    // fixed behavior (re-derive the new optimum experimentally; the
    // pre-PR1 hand-derived value of -6.755613 no longer applies).
    #[test]
    fn closed_set_discards_better_path_reaching_same_state() {
        fn rr(name: &str, smirks: &str) -> RetroRule {
            RetroRule {
                name: name.to_string(),
                template_id: format!("rule:{name}"),
                smirks: smirks.to_string(),
                weight: 1.0,
                required_elements: 0,
            }
        }

        // Each halogen swap explicitly tracks its displaced halogen as a
        // second output fragment (bare "Cl"/"Br"/"I" -- implicit-H SMILES
        // for HCl/HBr/HI) so every step is heavy-atom-conserving: RENKIN
        // Bridge PR1's completed-route integrity gate rejects any candidate
        // where `synthesizability::compute_element_accounting` finds a step
        // whose target needs a heavy element its precursors don't jointly
        // supply, and the original single-fragment "just relabel the
        // halogen" rules below tripped exactly that (correctly -- see PR1).
        let rules = vec![
            rr("r_direct", "[Cl][C:1][C:2][I]>>[Br][C:1][C:2][Br].Cl.I"),
            rr("r_step1", "[Cl][C:1][C:2][I]>>[Br][C:1][C:2][I].Cl"),
            rr("r_step2", "[Br][C:1][C:2][I]>>[Br][C:1][C:2][Br].I"),
            rr("r_final", "[Br][C:1][C:2][Br]>>[F][C:1][C:2][F].Br.Br"),
        ];

        // Discover Y's canonical SMILES dynamically — don't hardcode a
        // chematic-version-dependent canonical string (chematic has already
        // moved 0.4.25 -> 0.4.30 once in this repo's history).
        let t_mol = mol_from_smiles("ClCCI").unwrap();
        let y_smiles = apply_retro(&t_mol, &rules[1])[0][0].smiles.clone();

        // FCCF is the target chain's terminal building block; Cl/Br/I
        // (HCl/HBr/HI) are the displaced-halogen byproducts every step now
        // emits -- trivially depth-0 stock hits, same as FCCF.
        let env = ChemEnv::in_memory(&["FCCF", "Cl", "Br", "I"]);

        struct FixedPrior;
        impl ReactionPrior for FixedPrior {
            fn prior(&self, template_name: &str, _target_smiles: &str) -> f64 {
                match template_name {
                    "r_step1" | "r_step2" => 5.0,
                    _ => 0.0,
                }
            }
        }

        struct FixedEstimator {
            y_smiles: String,
        }
        impl MoleculeValueEstimator for FixedEstimator {
            fn estimate_cost(&self, smiles: &str) -> f64 {
                if smiles == self.y_smiles { 100.0 } else { 0.0 }
            }
        }

        let config = SearchConfig {
            max_depth: 5,
            max_routes: 10,
            beam_width: 0,
            reaction_prior: Some(std::sync::Arc::new(FixedPrior)),
            value_estimator: Some(std::sync::Arc::new(FixedEstimator {
                y_smiles: y_smiles.clone(),
            })),
            ..Default::default()
        };

        let (routes, _stats) = find_routes("ClCCI", &env, &rules, &config).unwrap();

        assert!(!routes.is_empty(), "must find at least the direct route");
        let best_score = routes.iter().map(|r| r.score).fold(f64::INFINITY, f64::min);

        // The true optimum (T->Y->M->Z) is deeply negative (dominated by the
        // two 5.0 template bonuses on r_step1/r_step2). If the closed set
        // reopened on a better g, `best_score` would land there. Instead
        // only the worse direct route (g ≈ 2.29) is ever recorded — proving
        // the cheaper re-arrival at {M} was discarded unexpanded.
        assert!(
            best_score > -1.0,
            "expected the boolean closed-set bug to discard the better \
             (deeply negative g) route, leaving only the worse (g≈2.29) \
             route — but best_score={best_score} suggests the optimal \
             route WAS found (bug fixed, or test assumptions stale)"
        );
        assert!(
            (best_score - 2.290).abs() < 0.05,
            "expected the only recorded route to be the direct-path route \
             (g≈2.13), got best_score={best_score}"
        );
    }

    // ---- Issue #101 Task 35: runtime reranker integration ----

    /// A `CandidateReranker` test double: no LightGBM model involved --
    /// `RuntimeReranker`'s own bit-exactness against `lightgbm.Booster` is
    /// already covered by `reranker.rs`'s tests and a 3000-row real-data
    /// check (see that module's tests + `scripts/reranker_golden_fixture.py`
    /// on the sibling reranker-real-data-gate branch). What THIS reranker
    /// double isolates is search.rs's own new glue: does
    /// `reranker_rank_bonuses` merge/featurize/score/rank raw proposals
    /// exactly the same way calling those same shared primitives directly,
    /// in the textbook order, would? Its score is a deterministic function
    /// of candidate identity alone (sorted precursor SMILES joined length),
    /// independently recomputable by the test without re-deriving anything
    /// `reranker_rank_bonuses` itself computed.
    struct DeterministicReranker;
    impl crate::candidate::CandidateReranker for DeterministicReranker {
        fn score_pool(
            &self,
            _target: &str,
            candidates: &mut [crate::candidate::ReactionCandidate],
        ) -> anyhow::Result<()> {
            for c in candidates.iter_mut() {
                c.reranker_score = Some(c.precursor_smiles.join(".").len() as f64);
            }
            Ok(())
        }
    }

    fn deterministic_score(precursor_smiles: &[String]) -> f64 {
        let mut sorted = precursor_smiles.to_vec();
        sorted.sort_unstable();
        sorted.join(".").len() as f64
    }

    #[test]
    fn reranker_rank_bonuses_matches_the_canonical_merge_extract_score_pipeline() {
        let rules = default_rules();
        let target_smi = "CC(=O)Oc1ccccc1C(=O)O"; // aspirin: multiple rules apply
        let target_mol = mol_from_smiles(target_smi).unwrap();
        let scored_active_rules: Vec<crate::candidate::ScoredRuleRef<'_>> = rules
            .iter()
            .enumerate()
            .map(|(rank, rule)| crate::candidate::ScoredRuleRef {
                rule,
                source_rank: rank,
                upstream_score: None,
                upstream_score_status: crate::candidate::UpstreamScoreStatus::NotApplicable,
            })
            .collect();
        let (raw_proposals, _diag, _sbl_findings, _gated_out) = crate::candidate::raw_propose(
            &target_mol,
            target_smi,
            &scored_active_rules,
            crate::ring_context::RingContextArgs {
                config: crate::ring_context::RingContextConfig::Disabled,
            },
            SpectatorBondPolicy::Off,
        );
        assert!(
            raw_proposals.len() >= 2,
            "fixture must exercise a multi-candidate pool, got {}",
            raw_proposals.len()
        );

        let templates_by_id = crate::candidate::index_rules_by_template_id(&rules).unwrap();

        // Path A: exactly what search.rs's hot loop calls.
        let via_search_rs = reranker_rank_bonuses(
            &DeterministicReranker,
            target_smi,
            &target_mol,
            &raw_proposals,
            &templates_by_id,
        )
        .unwrap();

        // Path B: the same primitives, called directly in the textbook
        // (offline-pool-export-equivalent) order, independently of
        // `reranker_rank_bonuses`.
        let mut candidates =
            crate::candidate::merge_into_candidates(target_smi, &raw_proposals).unwrap();
        for c in candidates.iter_mut() {
            c.features = crate::candidate::extract_features(c, &target_mol, &templates_by_id, None);
        }
        assert!(
            candidates.len() >= 2,
            "merge must still produce a multi-candidate pool, got {}",
            candidates.len()
        );
        candidates.sort_by(|a, b| {
            deterministic_score(&b.precursor_smiles)
                .partial_cmp(&deterministic_score(&a.precursor_smiles))
                .unwrap()
                .then_with(|| a.candidate_id.cmp(&b.candidate_id))
        });
        let n = candidates.len();
        let via_direct: FxHashMap<String, f64> = candidates
            .into_iter()
            .enumerate()
            .map(|(rank, c)| (c.candidate_id, crate::score::rank_bonus(rank, n)))
            .collect();

        assert_eq!(
            via_search_rs.len(),
            via_direct.len(),
            "same candidate_id set expected from both paths"
        );
        for (id, direct_bonus) in &via_direct {
            let search_bonus = via_search_rs
                .get(id)
                .unwrap_or_else(|| panic!("candidate_id {id} missing from search.rs's map"));
            assert!(
                (search_bonus - direct_bonus).abs() < 1e-12,
                "bonus mismatch for {id}: search.rs={search_bonus}, direct={direct_bonus}"
            );
        }
        // Bonus values must actually span the scale, not all collapse to 0 --
        // otherwise this test would pass trivially without exercising rank.
        let distinct: std::collections::BTreeSet<u64> =
            via_direct.values().map(|v| v.to_bits()).collect();
        assert!(
            distinct.len() >= 2,
            "fixture must produce differentiated ranks, got {distinct:?}"
        );
    }

    #[test]
    fn reranker_changes_ordering_only_not_the_candidate_set() {
        let env = aspirin_env();
        let rules = default_rules();
        let target_smi = "CC(=O)Oc1ccccc1C(=O)O";

        let legacy_cfg = cfg(2);
        let (_routes_legacy, stats_legacy) =
            find_routes(target_smi, &env, &rules, &legacy_cfg).unwrap();

        let reranked_cfg = SearchConfig {
            reranker: Some(std::sync::Arc::new(DeterministicReranker)),
            ..cfg(2)
        };
        let (_routes_reranked, stats_reranked) =
            find_routes(target_smi, &env, &rules, &reranked_cfg).unwrap();

        assert_eq!(
            stats_reranked.reranker_failures, 0,
            "the deterministic test double must never fail"
        );
        // Ordering-only at the unbounded (beam_width: 0, via cfg()) search
        // this fixture uses: the reranker must never change how many
        // templates matched or how many nodes got expanded here, only
        // which f() they're explored under. This is NOT a claim that a
        // beam-limited search explores the identical tree regardless of
        // beam width -- see SearchConfig::reranker's doc for why a
        // reordering-induced difference in beam_prune's eviction choices
        // under a real beam width is the intended mechanism, not something
        // this test (or "ordering-only" generally) rules out.
        assert_eq!(
            stats_legacy.matched_templates,
            stats_reranked.matched_templates
        );
        assert_eq!(stats_legacy.nodes_expanded, stats_reranked.nodes_expanded);
    }

    #[test]
    fn reranker_under_tight_beam_prunes_safely_and_stays_deterministic() {
        // The tests above all use cfg()'s beam_width: 0 (unlimited A*),
        // where beam_prune is a no-op -- they can't exercise the actual
        // reranker/beam-pruning interaction (reordering changing which
        // nodes survive beam_prune, hence which candidates ever get
        // proposed deeper in the tree) that is the real mechanism behind
        // this PR's L1541 result. Mirrors
        // crowd_out_diagnostics_records_eviction_under_tight_beam's
        // fixture (same target/beam_width=1, known to trigger real
        // evictions) but with a reranker configured, to prove that
        // combination doesn't panic, doesn't fail, and stays deterministic
        // -- not to assert a specific behavioral divergence from legacy
        // ordering, which would be fixture-dependent and flaky to pin down
        // at this small a scale.
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_beam = SearchConfig {
            max_depth: 3,
            max_routes: 3,
            beam_width: 1,
            reranker: Some(std::sync::Arc::new(DeterministicReranker)),
            ..Default::default()
        };
        let (routes_a, stats_a) =
            find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam).unwrap();
        assert_eq!(stats_a.reranker_failures, 0);
        assert!(
            stats_a.crowd_out.beam_prune_invocations > 0,
            "fixture must actually exercise beam pruning, or this test isn't testing anything \
             the beam_width=0 tests don't already cover"
        );
        let (routes_b, stats_b) =
            find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam).unwrap();
        assert_eq!(
            serde_json::to_string(&routes_a).unwrap(),
            serde_json::to_string(&routes_b).unwrap(),
            "reranker + tight beam must still be deterministic"
        );
        assert_eq!(stats_b.reranker_failures, 0);
    }

    #[test]
    fn reranker_none_is_byte_identical_to_pre_reranker_ordering() {
        let env = aspirin_env();
        let rules = default_rules();
        let target_smi = "CC(=O)Oc1ccccc1C(=O)O";
        let (routes_a, stats_a) = find_routes(target_smi, &env, &rules, &cfg(3)).unwrap();
        let (routes_b, stats_b) = find_routes(target_smi, &env, &rules, &cfg(3)).unwrap();
        assert_eq!(
            serde_json::to_string(&routes_a).unwrap(),
            serde_json::to_string(&routes_b).unwrap(),
            "reranker: None must be fully deterministic (a stand-in for byte-diffing \
             against the pre-wiring binary's output on the same input)"
        );
        assert_eq!(stats_a.reranker_failures, 0);
        assert_eq!(stats_b.reranker_failures, 0);
    }

    #[test]
    fn reranker_some_is_also_fully_deterministic_across_repeated_runs() {
        // Covers the "determinism" line item for the paired route-search
        // gate without needing a second full external 100-target run:
        // raw_propose's rayon par_iter().map().collect() preserves
        // active_rules' input order regardless of completion order (same
        // guarantee the legacy, already-deterministic path relies on), and
        // reranker_rank_bonuses' sort key is a total, content-based order
        // (score desc, candidate_id asc) with no dependency on iteration/
        // hashmap order -- so a reranker-configured run should be exactly
        // as deterministic as the legacy path.
        let env = aspirin_env();
        let rules = default_rules();
        let target_smi = "CC(=O)Oc1ccccc1C(=O)O";
        let reranked_cfg = SearchConfig {
            reranker: Some(std::sync::Arc::new(DeterministicReranker)),
            ..cfg(3)
        };
        let (routes_a, stats_a) = find_routes(target_smi, &env, &rules, &reranked_cfg).unwrap();
        let (routes_b, stats_b) = find_routes(target_smi, &env, &rules, &reranked_cfg).unwrap();
        assert_eq!(
            serde_json::to_string(&routes_a).unwrap(),
            serde_json::to_string(&routes_b).unwrap(),
            "reranker: Some(..) must be just as deterministic as the legacy path"
        );
        assert_eq!(stats_a.reranker_failures, 0);
        assert_eq!(stats_b.reranker_failures, 0);
    }
}

/// Cooperative cancellation foundation (v0.24 coverage-mode Phase 41.18A):
/// [`SearchControl`]/[`SearchTermination`]/[`SearchRunResult`]/
/// [`find_routes_with_control`]. Additive-only -- these tests exist
/// specifically to pin down that nothing about [`find_routes`]'s existing
/// contract moved.
#[cfg(test)]
mod cooperative_cancellation_tests {
    use super::*;
    use crate::chem_env::{ChemEnv, default_rules};

    fn env() -> ChemEnv {
        ChemEnv::load("data/building_blocks.smi").unwrap_or_else(|_| {
            ChemEnv::in_memory(&["CC(=O)O", "Oc1ccccc1C(=O)O", "c1ccccc1C(=O)O", "C", "O"])
        })
    }

    fn cfg(beam_width: usize) -> SearchConfig {
        SearchConfig {
            max_depth: 5,
            max_routes: 5,
            beam_width,
            ..Default::default()
        }
    }

    const TARGET: &str = "CC(=O)Oc1ccccc1C(=O)O";

    /// Requirement 1: the pre-existing `find_routes` wrapper and
    /// `find_routes_with_control(.., &SearchControl::unlimited())` must
    /// produce byte-identical routes and stats -- there is exactly one
    /// search implementation, not two.
    #[test]
    fn wrapper_matches_unlimited_control_exactly() {
        let env = env();
        let rules = default_rules();
        for beam_width in [0, 10, 100] {
            let config = cfg(beam_width);
            let (wrapper_routes, wrapper_stats) =
                find_routes(TARGET, &env, &rules, &config).unwrap();
            let controlled = find_routes_with_control(
                TARGET,
                &env,
                &rules,
                &config,
                &SearchControl::unlimited(),
            )
            .unwrap();

            assert_eq!(controlled.termination, SearchTermination::Completed);
            assert_eq!(
                serde_json::to_string(&wrapper_routes).unwrap(),
                serde_json::to_string(&controlled.routes).unwrap(),
                "wrapper vs. find_routes_with_control(unlimited) routes diverged at beam_width={beam_width}"
            );
            assert_eq!(
                serde_json::to_string(&wrapper_stats).unwrap(),
                serde_json::to_string(&controlled.stats).unwrap(),
                "wrapper vs. find_routes_with_control(unlimited) stats diverged at beam_width={beam_width}"
            );
        }
    }

    /// Requirement 2: this file's pre-existing test suite (53 tests, none
    /// modified by this change) is the golden/default-behavior regression
    /// suite -- run alongside these as `cargo test --lib search::`. This
    /// test adds one more direct check: unlimited-control search on a
    /// known target reproduces a fixed expectation independent of how
    /// `SearchControl` is threaded through.
    #[test]
    fn unlimited_control_reproduces_known_golden_result() {
        let env = env();
        let rules = default_rules();
        let result =
            find_routes_with_control(TARGET, &env, &rules, &cfg(0), &SearchControl::unlimited())
                .unwrap();
        assert_eq!(result.termination, SearchTermination::Completed);
        assert!(
            !result.routes.is_empty(),
            "aspirin must find at least one route"
        );
        assert!(
            result.routes.iter().any(|r| r.depth <= 2),
            "must find a route with depth <= 2, same as aspirin_finds_route_depth1"
        );
    }

    /// Requirement 3 + 4: a deadline already in the past trips immediately
    /// (deterministic regardless of machine speed -- any nonzero code
    /// execution between capturing `Instant::now()` and the first
    /// checkpoint's own `Instant::now()` call strictly advances a
    /// monotonic clock) and the call returns `Ok` rather than panicking.
    #[test]
    fn already_past_deadline_returns_deadline_exceeded_without_panicking() {
        let env = env();
        let rules = default_rules();
        let control = SearchControl::with_deadline(std::time::Instant::now());
        let result = find_routes_with_control(TARGET, &env, &rules, &cfg(0), &control);
        assert!(
            result.is_ok(),
            "must not panic or error, even with zero budget"
        );
        let result = result.unwrap();
        assert_eq!(result.termination, SearchTermination::DeadlineExceeded);
    }

    /// Isolates checkpoint 1 specifically: `max_depth: 0` means every
    /// popped node hits the `node.depth >= config.max_depth` `continue`
    /// path immediately -- expansion (and therefore checkpoints 2 and 3)
    /// is never reached at all, for any node, for the whole search.
    /// Checkpoint 1 is the *only* thing that can observe the deadline
    /// here. If it were removed, this search would run to natural
    /// completion (the heap empties after the one no-op root iteration)
    /// and report `Completed` instead -- i.e. this test is expected to
    /// fail under exactly that mutation, not just under "delete
    /// everything."
    #[test]
    fn checkpoint_one_alone_catches_a_deadline_no_expansion_ever_reaches() {
        let env = env();
        let rules = default_rules();
        let config = SearchConfig {
            max_depth: 0,
            ..cfg(0)
        };
        let control = SearchControl::with_deadline(std::time::Instant::now());
        let result = find_routes_with_control(TARGET, &env, &rules, &config, &control).unwrap();
        assert_eq!(result.termination, SearchTermination::DeadlineExceeded);
    }

    /// Requirement 4 (panic-safety) again, on a config that guarantees
    /// several loop iterations run before the deadline check fires
    /// (`beam_width` unrestricted at `max_depth=5`) -- exercises more of
    /// the loop body, including the post-loop confidence/atom-economy/
    /// convergency pass running over a partial or empty `routes`.
    #[test]
    fn microsecond_timeout_on_a_real_search_does_not_panic() {
        let env = env();
        let rules = default_rules();
        let control = SearchControl::with_timeout(std::time::Duration::from_micros(1));
        let result = find_routes_with_control(TARGET, &env, &rules, &cfg(0), &control);
        assert!(result.is_ok());
    }

    /// Requirement 5: routes found before the deadline are never discarded.
    /// Deliberately does not depend on hitting one exact wall-clock window
    /// -- machine speed varies (this whole program has direct, repeated
    /// evidence of that: load average swinging 4-22 on the same 10-core
    /// box within a single session). Instead: sample several fractions of
    /// a freshly-measured per-attempt baseline elapsed time, assert the
    /// core invariant (never more routes than the full baseline, every
    /// returned route genuinely exists in the baseline -- no fabrication)
    /// unconditionally on every sample, and additionally require that at
    /// least one sampled fraction actually caught a nonempty partial
    /// result under `DeadlineExceeded` -- so the property this test exists
    /// to check is positively exercised, not just vacuously not-violated.
    #[test]
    fn valid_routes_found_before_deadline_are_not_discarded() {
        let env = env();
        let rules = default_rules();
        let config = cfg(0);

        let baseline =
            find_routes_with_control(TARGET, &env, &rules, &config, &SearchControl::unlimited())
                .unwrap();
        assert_eq!(baseline.termination, SearchTermination::Completed);
        assert!(
            !baseline.routes.is_empty(),
            "fixture must find at least one route to be a meaningful test"
        );

        // Percent-of-baseline fractions, deliberately dense in the 50-90%
        // band: empirically, that's where a nonempty `DeadlineExceeded`
        // partial actually lands for this fixture -- below ~50% nothing has
        // been found yet (empty partial, doesn't count), above ~90% the
        // search has usually already finished (`Completed`, also doesn't
        // count). More samples in the productive band raises the odds that
        // at least one lands there in any single sweep.
        //
        // Still a wall-clock race in the end (this whole test's premise is
        // timing-based), so under heavy scheduling contention a single
        // sweep can still miss the window entirely -- confirmed empirically
        // under this project's own repeated runs (Issue #130). Retrying the
        // whole sweep with a fresh baseline measurement, rather than
        // widening the fraction set further, directly targets that failure
        // mode: an unlucky sweep coinciding with a contention spike, not a
        // logic error in the search or the deadline mechanism itself.
        const FRACTIONS: [u32; 11] = [30, 40, 50, 55, 60, 65, 70, 75, 80, 85, 90];
        const MAX_SWEEPS: u32 = 3;
        let mut saw_nonempty_partial = false;
        for _sweep in 0..MAX_SWEEPS {
            for frac in FRACTIONS {
                let t0 = std::time::Instant::now();
                let _ = find_routes_with_control(
                    TARGET,
                    &env,
                    &rules,
                    &config,
                    &SearchControl::unlimited(),
                )
                .unwrap();
                let baseline_elapsed = t0.elapsed();

                let partial = find_routes_with_control(
                    TARGET,
                    &env,
                    &rules,
                    &config,
                    &SearchControl::with_timeout(baseline_elapsed * frac / 100),
                )
                .unwrap();

                assert!(
                    partial.routes.len() <= baseline.routes.len(),
                    "must never return more routes than the full search finds (frac={frac})"
                );
                for r in &partial.routes {
                    assert!(
                        baseline.routes.iter().any(|br| br.depth == r.depth
                            && br.steps.len() == r.steps.len()
                            && (br.score - r.score).abs() < 1e-9),
                        "a route present in the deadline-cut result must also exist in the \
                         unlimited baseline (no fabrication/corruption), frac={frac}"
                    );
                }
                if partial.termination == SearchTermination::DeadlineExceeded
                    && !partial.routes.is_empty()
                {
                    saw_nonempty_partial = true;
                }
            }
            if saw_nonempty_partial {
                break;
            }
        }
        assert!(
            saw_nonempty_partial,
            "expected at least one sampled deadline fraction, across up to {MAX_SWEEPS} sweeps, \
             to catch a nonempty partial route set before full completion -- if this ever \
             flakes, the sampled fraction set may need widening for the machine it's running on"
        );
    }

    /// Requirement 6: no thread/task survives past the call returning.
    /// This design has no threading at all in the cancellation path itself
    /// (a single blocking call per loop iteration, checked cooperatively
    /// between calls -- never a spawned/detached thread), so there is
    /// structurally nothing to leak. What's actually observable from a
    /// test is that the call returns promptly once the deadline passes,
    /// rather than continuing to block on unrelated background work: the
    /// wall-clock time actually spent inside the call must not appreciably
    /// outlive one worst-case checkpoint interval.
    #[test]
    fn call_returns_promptly_after_deadline_leaves_nothing_running() {
        let env = env();
        let rules = default_rules();
        let control = SearchControl::with_timeout(std::time::Duration::from_micros(1));
        let t0 = std::time::Instant::now();
        let result = find_routes_with_control(TARGET, &env, &rules, &cfg(0), &control).unwrap();
        let call_elapsed = t0.elapsed();
        assert_eq!(result.termination, SearchTermination::DeadlineExceeded);
        // Generous bound (this specific fixture's full unlimited search is
        // on the order of tens of milliseconds, not seconds) -- this is a
        // smoke check against "hung waiting on something," not a tight
        // performance assertion.
        assert!(
            call_elapsed < std::time::Duration::from_secs(5),
            "call took {call_elapsed:?} after an immediate deadline -- looks like it's \
             blocking on something instead of returning promptly"
        );
    }

    /// Minimal local reranker test double -- deliberately not reusing
    /// `tests::DeterministicReranker` (private to its own module); this one
    /// only needs to exercise the reranker-active code path, not match any
    /// particular scoring behavior.
    struct StubReranker;
    impl crate::candidate::CandidateReranker for StubReranker {
        fn score_pool(
            &self,
            _target: &str,
            candidates: &mut [crate::candidate::ReactionCandidate],
        ) -> anyhow::Result<()> {
            for c in candidates.iter_mut() {
                c.reranker_score = Some(c.precursor_smiles.join(".").len() as f64);
            }
            Ok(())
        }
    }

    /// Requirement 7: safe (no panic, sensible termination) both with and
    /// without a reranker configured.
    #[test]
    fn safe_with_and_without_reranker() {
        let env = env();
        let rules = default_rules();

        let no_reranker = cfg(0);
        let with_reranker = SearchConfig {
            reranker: Some(std::sync::Arc::new(StubReranker)),
            ..cfg(0)
        };

        for config in [&no_reranker, &with_reranker] {
            let unlimited =
                find_routes_with_control(TARGET, &env, &rules, config, &SearchControl::unlimited())
                    .unwrap();
            assert_eq!(unlimited.termination, SearchTermination::Completed);
            assert_eq!(unlimited.stats.reranker_failures, 0);

            let timed_out = find_routes_with_control(
                TARGET,
                &env,
                &rules,
                config,
                &SearchControl::with_deadline(std::time::Instant::now()),
            )
            .unwrap();
            assert_eq!(timed_out.termination, SearchTermination::DeadlineExceeded);
        }
    }

    /// Requirement 8: both beam-limited and unlimited (A*) search modes.
    #[test]
    fn safe_with_beam_width_zero_and_nonzero() {
        let env = env();
        let rules = default_rules();
        for beam_width in [0usize, 10, 100] {
            let config = cfg(beam_width);
            let unlimited = find_routes_with_control(
                TARGET,
                &env,
                &rules,
                &config,
                &SearchControl::unlimited(),
            )
            .unwrap();
            assert_eq!(unlimited.termination, SearchTermination::Completed);

            let timed_out = find_routes_with_control(
                TARGET,
                &env,
                &rules,
                &config,
                &SearchControl::with_deadline(std::time::Instant::now()),
            )
            .unwrap();
            assert_eq!(timed_out.termination, SearchTermination::DeadlineExceeded);
        }
    }

    /// Requirement (Finding 4): `max_routes` completion is classified
    /// `Completed`, never `DeadlineExceeded`, even when the deadline has
    /// also already passed by the time that's checked. Deterministic --
    /// `max_routes: 0` means `routes.len() >= config.max_routes` is `true`
    /// from the very first loop iteration, before any expansion, so this
    /// doesn't depend on racing real search progress against a timing
    /// window. Runs through the actual `find_routes_with_control` control
    /// flow, not an extracted ordering-only helper.
    #[test]
    fn max_routes_completion_wins_over_an_already_expired_deadline() {
        let env = env();
        let rules = default_rules();
        let config = SearchConfig {
            max_routes: 0,
            ..cfg(0)
        };
        // Already past by the time the loop's first iteration checks it --
        // same deterministic reasoning as
        // `already_past_deadline_returns_deadline_exceeded_without_panicking`.
        let control = SearchControl::with_deadline(std::time::Instant::now());
        let result = find_routes_with_control(TARGET, &env, &rules, &config, &control).unwrap();
        assert_eq!(
            result.termination,
            SearchTermination::Completed,
            "max_routes was already satisfied (trivially, at 0) -- must report Completed \
             even though the deadline had also already passed"
        );
        assert!(result.routes.is_empty());
    }
}

/// RENKIN Bridge PR1: completed-route structural-integrity gate
/// (`RouteIntegrityDefect`/`RouteIntegrityDiagnostics`/
/// `route_integrity_defects`) -- the acceptance-boundary check wired into
/// `find_routes_with_control`'s frontier loop.
#[cfg(test)]
mod route_integrity_tests {
    use super::*;

    /// The gate compares canonical forms, not raw strings -- fixtures below
    /// always canonicalize their own root so a hand-written (possibly
    /// non-canonical) SMILES literal never trips a spurious `RootMismatch`.
    fn canon(smiles: &str) -> String {
        to_canonical(&mol_from_smiles(smiles).unwrap())
    }

    fn step(target: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: "test_rule".to_string(),
            template_id: "rule:test_rule".to_string(),
            target: target.to_string(),
            precursors: precursors.iter().map(|s| s.to_string()).collect(),
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
            steps,
            depth: 1,
            score: 0.0,
            building_blocks: vec![],
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 1.0,
        }
    }

    #[test]
    fn clean_route_has_no_defects() {
        // Ester hydrolysis, same fixture as
        // `synthesizability::element_accounting`'s own clean case: root
        // matches, both fragments parse, no cycle, both precursors are
        // unreachable-as-a-step-target (leaves), every heavy element is
        // accounted for.
        let root = canon("CC(=O)Oc1ccccc1");
        let r = route(vec![step(&root, &["CC(=O)O", "Oc1ccccc1"])]);
        let defects = route_integrity_defects(&r, &root);
        assert!(defects.is_empty(), "expected no defects, got {defects:?}");
    }

    #[test]
    fn empty_steps_is_depth_zero_and_always_passes() {
        let r = route(vec![]);
        assert!(route_integrity_defects(&r, &canon("CC(=O)O")).is_empty());
    }

    #[test]
    fn flags_root_mismatch() {
        // The route's first step decomposes a molecule that isn't the
        // searched-for target at all.
        let r = route(vec![step("CCO", &["CC=O"])]);
        let defects = route_integrity_defects(&r, &canon("c1ccccc1"));
        assert!(defects.contains(&RouteIntegrityDefect::RootMismatch));
    }

    #[test]
    fn flags_unparseable_target_smiles() {
        // Unclosed bracket -- guaranteed parser rejection, same convention
        // as `element_accounting`'s own tests.
        let r = route(vec![step("[C(", &["CCO"])]);
        let defects = route_integrity_defects(&r, "[C(");
        assert!(defects.contains(&RouteIntegrityDefect::UnparseableSmiles));
    }

    #[test]
    fn flags_unparseable_precursor_smiles() {
        let root = canon("CC(=O)O");
        let r = route(vec![step(&root, &["[C(", "O"])]);
        let defects = route_integrity_defects(&r, &root);
        assert!(defects.contains(&RouteIntegrityDefect::UnparseableSmiles));
    }

    #[test]
    fn flags_empty_precursor_list() {
        let root = canon("CC(=O)O");
        let r = route(vec![step(&root, &[])]);
        let defects = route_integrity_defects(&r, &root);
        assert!(defects.contains(&RouteIntegrityDefect::EmptyPrecursorList));
    }

    #[test]
    fn flags_cycle() {
        // A (target) decomposes to B, and B is later (mis)decomposed back
        // to A -- a molecule reappearing as its own descendant's
        // precursor. Not prevented by construction (see
        // `RouteIntegrityDefect::Cycle`'s doc comment) and would otherwise
        // infinite-loop `display::build_tree`'s unguarded recursion.
        let a = canon("CCO");
        let b = canon("CC=O");
        let r = route(vec![step(&a, &[b.as_str()]), step(&b, &[a.as_str()])]);
        let defects = route_integrity_defects(&r, &a);
        assert!(defects.contains(&RouteIntegrityDefect::Cycle));
    }

    #[test]
    fn flags_disconnected_step() {
        // Second step's target ("c1ccccc1") is never referenced as any
        // ancestor's precursor -- unreachable from the root by the same
        // target/precursor string-matching walk `extract_building_blocks`
        // and `display::build_tree` already rely on.
        let root = canon("CC(=O)Oc1ccccc1");
        let orphan = canon("c1ccccc1");
        let r = route(vec![
            step(&root, &["CC(=O)O", "Oc1ccccc1"]),
            step(&orphan, &["C1=CC=CC=C1"]),
        ]);
        let defects = route_integrity_defects(&r, &root);
        assert!(defects.contains(&RouteIntegrityDefect::Disconnected));
    }

    #[test]
    fn flags_unaccounted_target_element() {
        // Same "clear violation" fixture as
        // `synthesizability::element_accounting`'s own test: target
        // carries a bromine no precursor supplies.
        let root = canon("Brc1ccccc1");
        let r = route(vec![step(&root, &["c1ccccc1"])]);
        let defects = route_integrity_defects(&r, &root);
        assert!(defects.contains(&RouteIntegrityDefect::UnaccountedTargetElement));
    }

    /// End-to-end regression for Issue #72/L984 (`uspto50k_test#L984`,
    /// the isoindolinone ring-disconnection case): reproduced live
    /// against the real 500-template corpus with the default
    /// `ring_context_policy = Disabled` (Issue #72's match-level guard
    /// is opt-in, not on by default), confirming this is not a
    /// hypothetical failure mode. Before this gate existed, this target
    /// returned 5 routes, top-ranked by `extracted_9` --
    /// "N-methylisoindolin-1-one" -> "benzoic acid" alone, silently
    /// dropping the target's nitrogen and its whole ring-fused CH2
    /// extension. Every one of the 5 candidates this corpus finds at
    /// this depth/beam has the same defect, so the gate correctly
    /// abstains (`routes_found: 0`) rather than surface any of them.
    #[test]
    fn isoindolinone_ring_disconnection_is_rejected_not_returned() {
        let env = ChemEnv::load("data/building_blocks.smi").unwrap_or_else(|_| {
            ChemEnv::in_memory(&["CC(=O)O", "Oc1ccccc1C(=O)O", "c1ccccc1C(=O)O", "C", "O"])
        });
        let rules = crate::chem_env::load_rules_from_file("data/templates_extracted_500.smi");
        assert!(
            !rules.is_empty(),
            "requires the committed 500-template corpus"
        );
        let config = SearchConfig {
            max_depth: 2,
            max_routes: 5,
            beam_width: 50,
            ..Default::default()
        };
        let (routes, stats) = find_routes("O=C1N(C)Cc2ccccc21", &env, &rules, &config).unwrap();
        assert!(
            routes.is_empty(),
            "every candidate at this depth/beam is known to drop the \
                 target's nitrogen -- the gate must reject all of them, \
                 got {} routes",
            routes.len()
        );
        assert!(
            stats.route_integrity.unaccounted_target_element > 0,
            "rejection must be attributed to unaccounted_target_element, \
                 got {:?}",
            stats.route_integrity
        );
    }
}
