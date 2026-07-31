//! Type definitions for the Synthesizability Kernel (`docs/design/
//! synthesizability-kernel-v0.md`). This file is types only: no
//! `search::Route` inspection, no hashing, no assessment logic. The two
//! `SynthesizabilityConfig` constructors below (`conservative()`/
//! `diagnostic()`) are field-literal initializers, not policy logic --
//! see their doc comments for exactly what they do and don't decide.
//!
//! Everything here must compile standalone from this file (plus `mod.rs`'s
//! re-export) alone: two more agents build `signals.rs`/
//! `element_accounting.rs` and `assessment.rs`/`provenance.rs` against
//! these types without touching this file.

use serde::Serialize;

/// Schema version of [`SynthesizabilityAssessment`]. Bump whenever a field
/// is added, removed, or its meaning changes, so downstream JSON consumers
/// can detect incompatible changes instead of silently misreading an
/// assessment. Wholly independent of any other report's schema version in
/// this workspace (e.g. `renkin_forward`'s `FORWARD_REPORT_SCHEMA_VERSION`)
/// -- bumping one never implies anything about another.
pub const SYNTHESIZABILITY_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------
// 4.1 AssessmentStatus
// ---------------------------------------------------------------------

/// Overall verdict for a target's synthesizability assessment. Never a
/// score -- a discrete status chosen by a first-match decision order (see
/// design doc §4.1): `InvalidTarget` > `EvaluationError` >
/// `NoRouteFoundWithinBudget` > `Indeterminate` > `RoutesFoundButRejected`
/// > `RouteSupportedWithValidationGaps` > `RouteSupported`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentStatus {
    /// The target SMILES does not parse. Checked first, before anything
    /// else about routes, stock, or config is considered.
    InvalidTarget,
    /// The assessment itself could not complete -- e.g. a stock entry's
    /// SMILES is unparseable and the configured policy does not tolerate
    /// that, or an internal invariant was violated. A kernel integrity
    /// failure, never a chemistry judgment.
    EvaluationError,
    /// Zero routes were supplied. With today's `find_routes` (design doc
    /// §3), this is always the correct classification for a zero-route
    /// result: the search config has no time/node budget, so an empty
    /// search frontier is the only way to get zero routes back -- depth
    /// and beam width *are* the budget.
    NoRouteFoundWithinBudget,
    /// Reserved for a future state: zero routes were returned but the
    /// reason the search stopped short is unknown (e.g. a search cut off
    /// by a wall-clock or node-count budget before its frontier was
    /// exhausted). Unreachable with today's `find_routes`, which has no
    /// such budget -- see design doc §3. Kept in the schema now, unused,
    /// so a future PR that adds a real search budget doesn't need a schema
    /// break to wire it up. Do not "fix" this into reachability without
    /// also adding that budget.
    Indeterminate,
    /// One or more routes were supplied, and every one of them has at
    /// least one hard failure.
    RoutesFoundButRejected,
    /// At least one route has zero hard failures, but none of them clears
    /// every configured required check (i.e. at least one route has zero
    /// hard failures and one or more validation gaps).
    RouteSupportedWithValidationGaps,
    /// At least one route has zero hard failures and clears every
    /// configured required check. This means the route is supported under
    /// the configured search, stock, templates, and validation policy. It
    /// is not a guarantee of experimental synthesis success.
    RouteSupported,
}

// ---------------------------------------------------------------------
// 4.2 StockTerminationStatus
// ---------------------------------------------------------------------

/// Whether every leaf (terminal precursor) in a route was independently
/// verified against the configured stock set. Per design doc §4.2, the
/// kernel performs its own stock-identity check here and deliberately does
/// not call `ChemEnv::is_building_block`: that path is the confirmed-buggy
/// VF2 subgraph fallback (issue #71) on `master` today, and even once
/// fixed its correctness would depend on branch-specific `chem_env.rs`
/// wiring the kernel shouldn't have to track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StockTerminationStatus {
    /// Every leaf's kernel-canonicalized SMILES was found in the
    /// kernel-canonicalized configured stock set.
    AllLeavesVerifiedInConfiguredStock,
    /// At least one leaf's kernel-canonicalized SMILES was not found in
    /// the configured stock set.
    OneOrMoreLeavesNotInStock,
    /// No stock set was supplied to the assessment at all. Never silently
    /// treated as `StockCheckNotPerformed` or, worse, "assume it's fine"
    /// -- a distinct, explicit status.
    StockNotSupplied,
    /// A specific leaf's SMILES failed to parse or standardize under the
    /// kernel's own pipeline. A fact about the *input*, not the check
    /// logic -- distinct from `StockCheckError`. Do not conflate the two:
    /// an unparseable leaf is not a kernel bug.
    StockIdentityUnavailable,
    /// The stock check was not performed because `SynthesizabilityConfig`
    /// disabled it (`require_verified_stock_terminal: false`).
    StockCheckNotPerformed,
    /// The stock-identity check logic itself broke (an internal error),
    /// as opposed to a genuine stock mismatch. A kernel bug, never a
    /// chemistry judgment -- distinct from `StockIdentityUnavailable`,
    /// which is a fact about one leaf's input, not the check machinery.
    StockCheckError,
}

// ---------------------------------------------------------------------
// 4.3 ForwardValidationStatus
// ---------------------------------------------------------------------

/// Route-level rollup of per-step forward-validation results. Per design
/// doc §4.3, forward validation is taken as an *input* to `assess_routes`
/// (an optional per-step validation-status list), never (re)computed by
/// this module -- picking one of the two existing, behaviorally different
/// forward-validation engines would couple the kernel to that engine's
/// bugs, and writing a third would duplicate effort for no clear gain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardValidationStatus {
    /// Every step was evaluated and every one is `Valid`.
    AllEvaluatedStepsValid,
    /// At least one step was evaluated as `Invalid`.
    OneOrMoreStepsInvalid,
    /// Some steps were evaluated (a mix of `Valid`/`NotEvaluable`, with no
    /// `Invalid`) -- i.e. neither every step is confirmed `Valid` nor is
    /// any step `Invalid`.
    PartiallyEvaluated,
    /// No per-step validation input was supplied for this route at all.
    /// `None` supplied -> this status.
    NotEvaluated,
    /// The supplied per-step validation input was structurally
    /// inconsistent with the route (e.g. wrong step count). A kernel-side
    /// integrity error, never a chemistry judgment.
    ValidatorError,
}

// ---------------------------------------------------------------------
// 4.5 ElementAccountingStatus
// ---------------------------------------------------------------------

/// Whether a route's per-step target-element accounting holds. This is a
/// **directional, one-way inequality only**: for each element present in
/// the target, the target's heavy-atom count (hydrogen excluded) must not
/// exceed the sum of that element's heavy-atom count over the step's
/// precursors. This is **not exact mass conservation** and must never be
/// read as such -- precursors are allowed to contribute atoms the target
/// doesn't need (a leaving group, a protecting group, a reagent); the
/// check only flags the target *needing more of an element than the
/// precursors supply*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementAccountingStatus {
    /// Every step's per-element inequality holds for every element present
    /// in the target.
    Accounted,
    /// At least one step has at least one element where the target's
    /// heavy-atom count exceeds the sum over precursors.
    UnaccountedTargetElement,
    /// The check could not be evaluated (e.g. a step's SMILES failed to
    /// parse under the kernel's pipeline).
    NotEvaluable,
}

// ---------------------------------------------------------------------
// 4.4 EvidenceCoverage
// ---------------------------------------------------------------------

/// Route-level rollup of per-step evidence signals, computed directly from
/// each `ReactionStep.evidence: Option<StepEvidence>` and its
/// `examples[].match_kind` (design doc §4.4) -- no new evidence logic,
/// purely a tally over already-existing data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct EvidenceCoverage {
    /// Steps with at least one exact-substrate-matched example
    /// (`ExampleMatch::ExactSubstrate`).
    pub exact_substrate_evidence_steps: usize,
    /// Steps with at least one template-level example
    /// (`ExampleMatch::TemplateOnly`) and no exact-substrate example.
    pub template_level_evidence_steps: usize,
    /// Steps with no evidence attached at all.
    pub steps_without_evidence: usize,
    /// Steps with at least one reported yield. Contract for whoever
    /// implements the extraction logic (`signals.rs`): this field must
    /// only be incremented for a reported yield whose source is *not*
    /// `MetadataSource::ModelPrediction`. "Yield is never a prediction" is
    /// true today only because nothing constructs that variant yet, and
    /// this field must not bake in an assumption that stops holding the
    /// moment it is. This type only documents the contract; it does not
    /// enforce it -- there is no extraction logic in this file.
    pub steps_with_reported_yield: usize,
    /// Steps with at least one condition candidate attached.
    pub steps_with_conditions: usize,
    /// Steps with at least one reaction warning attached.
    pub steps_with_warnings: usize,
}

// ---------------------------------------------------------------------
// 4.6 HardFailure / ValidationGap
// ---------------------------------------------------------------------

/// A hard failure recorded against one route: a reason the route is
/// rejected outright (design doc §4.6), regardless of everything else
/// about it. Distinct from [`ValidationGap`], which records a known,
/// accepted limitation that does not by itself reject the route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HardFailure {
    /// The route's structure could not be parsed by the kernel at all.
    RouteStructureUnparseable,
    /// A leaf claimed as a stock terminal by the route does not verify
    /// against the configured stock set under the kernel's own check (see
    /// [`StockTerminationStatus`]).
    StockTerminalMismatch { leaf: String },
    /// A step's target-element accounting failed (see
    /// [`ElementAccountingStatus::UnaccountedTargetElement`]) and the
    /// step's template is not on `reagent_omission_template_allowlist`.
    /// Design doc §4.5 names three failure classes that can surface this
    /// way, only two of which land here: a genuine template-extraction
    /// defect (#72-class), or a `split_fragments` fragment-filter artifact
    /// -- both indistinguishable to the kernel, both a real reason to
    /// reject the route. The third class (an intentionally-unmodeled
    /// reagent, e.g. Boc/Cbz deprotection) is carved out via the allowlist
    /// and recorded instead as
    /// [`ValidationGap::ReagentOmissionAccountingGap`].
    UnaccountedTargetElement { step_index: usize },
    /// A step's forward validation was `Invalid`, and
    /// `ForwardValidationPolicy` configured this as required.
    ForwardValidationFailed { step_index: usize },
    /// The route graph itself is internally inconsistent (e.g. a
    /// precursor/step mismatch), independent of any single step's
    /// chemistry.
    RouteGraphInconsistent,
}

/// A known, accepted limitation or missing check recorded against a route
/// that does *not* by itself reject it (design doc §4.6). Distinct from
/// [`HardFailure`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationGap {
    /// No forward-validation input was supplied for this route at all.
    ForwardValidationNotRun,
    /// No evidence at all is attached to at least one step.
    NoEvidence,
    /// At least one step has evidence, but none of it is an
    /// exact-substrate match.
    NoExactSubstrateEvidence,
    /// A step's forward-validation status is `NotEvaluable`.
    StepNotEvaluable { step_index: usize },
    /// The configured stock set has no provenance hash attached, so its
    /// contents cannot be independently reproduced/audited.
    StockProvenanceHashMissing,
    /// A step's target-element accounting failed, but its template *is*
    /// on `reagent_omission_template_allowlist`: design doc §4.5's third
    /// failure class, an intentionally-unmodeled reagent (e.g.
    /// `rule:boc_deprotection_retro`, `rule:cbz_deprotection_retro`) --
    /// an accepted search-rule limitation, not a defect. This bucket is
    /// deliberately kept separate from the genuine-extraction-defect and
    /// fragment-filter-artifact classes that land in
    /// [`HardFailure::UnaccountedTargetElement`] instead. See issue #73
    /// (unresolved as of this design) for why `rule:aryl_amine_retro` is
    /// *not* on the default allowlist, and therefore a matching failure on
    /// that template does not land here.
    ReagentOmissionAccountingGap {
        step_index: usize,
        template_id: String,
    },
    /// Reserved for when a real search timeout/node budget exists (see
    /// [`AssessmentStatus::Indeterminate`]) and a route is known to be
    /// only a best-effort result of a cut-off search, not a search of the
    /// full space within budget.
    BestEffortRouteOnly,
}

// ---------------------------------------------------------------------
// 4.6 RouteAssessment
// ---------------------------------------------------------------------

/// One route's full assessment: every signal, hard failure, and
/// validation gap computed for it (design doc §4.6).
/// [`SynthesizabilityAssessment::route_assessments`] holds one of these
/// per supplied route, in deterministic order (design doc §6).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouteAssessment {
    /// Deterministic route identifier (design doc §6): `sha256:<hex>` over
    /// a fixed domain separator, the canonical target, and the sorted,
    /// canonicalized `(template_id, target, precursors)` tuple for every
    /// step. Independent of route-discovery order.
    pub route_id: String,
    pub route_depth: u32,
    /// Reused verbatim from `search::Route::route_cost`. An existing,
    /// already-disclaimed cost heuristic -- used only as a
    /// route-selection tie-break (design doc §4.8), never promoted to a
    /// primary signal here.
    pub route_cost: f64,
    pub stock_termination_status: StockTerminationStatus,
    pub target_element_accounting_status: ElementAccountingStatus,
    pub forward_validation_status: ForwardValidationStatus,
    pub evidence_coverage: EvidenceCoverage,
    pub hard_failures: Vec<HardFailure>,
    pub validation_gaps: Vec<ValidationGap>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------
// 4.7 SynthesizabilityConfig
// ---------------------------------------------------------------------

/// How strictly `assess_routes()` treats forward-validation results as a
/// requirement for `RouteSupported` (design doc §4.7). Never a hard
/// requirement by default -- neither this validator's coverage nor its
/// false-positive/negative rate has been measured yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardValidationPolicy {
    /// Forward-validation results, if supplied, are recorded but never
    /// turned into a hard failure or a required check.
    Ignore,
    /// Every step must be evaluated and `Valid` for the route to clear
    /// this check.
    RequireAllValid,
    /// No step may be `Invalid`; steps that are `NotEvaluable` or simply
    /// not evaluated are tolerated.
    RequireNoInvalid,
}

/// How strictly `assess_routes()` treats evidence coverage as a
/// requirement for `RouteSupported` (design doc §4.7). Never a hard
/// requirement by default -- absence of evidence is never, by itself, a
/// hard failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidencePolicy {
    /// Evidence coverage, if present, is recorded but never turned into a
    /// hard failure or a required check.
    Ignore,
    /// Every step must have at least some evidence (exact-substrate or
    /// template-level) for the route to clear this check.
    RequireAnyEvidence,
    /// Every step must have at least one exact-substrate evidence match
    /// for the route to clear this check.
    RequireExactSubstrate,
}

/// Configuration for `assess_routes()`: which checks are required for
/// `RouteSupported`, and how the target-element-accounting
/// reagent-omission allowlist is applied (design doc §4.5, §4.7). Not
/// itself serialized -- see [`SynthesizabilityConfigSummary`] for the
/// owned, serializable echo of this config that ships inside a
/// [`SynthesizabilityAssessment`] for audit purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynthesizabilityConfig {
    pub require_verified_stock_terminal: bool,
    pub require_target_element_accounting: bool,
    pub forward_validation_policy: ForwardValidationPolicy,
    pub evidence_policy: EvidencePolicy,
    /// Template ids (`RetroRule::template_id`) whose only accounting
    /// problem is `UnaccountedTargetElement` are recorded as a
    /// [`ValidationGap::ReagentOmissionAccountingGap`], not a hard
    /// failure -- because the omission is by construction, not a
    /// search-quality question (design doc §4.5). Default:
    /// `["rule:boc_deprotection_retro", "rule:cbz_deprotection_retro"]`.
    /// Deliberately does **not** include `rule:aryl_amine_retro` by
    /// default -- see issue #73 (unresolved): unlike Boc/Cbz, it has no
    /// existing graph-rule exact-formula carve-out and is not assumed to
    /// be the same class.
    pub reagent_omission_template_allowlist: Vec<String>,
    pub max_routes_to_assess: usize,
    pub include_all_route_diagnostics: bool,
}

/// Default value of [`SynthesizabilityConfig::max_routes_to_assess`] for
/// both `conservative()` and `diagnostic()`. Not specified numerically by
/// the design doc; chosen as a reasonable, documented default rather than
/// left as an undocumented magic number at each call site.
const DEFAULT_MAX_ROUTES_TO_ASSESS: usize = 10;

impl SynthesizabilityConfig {
    /// The default allowlist for `reagent_omission_template_allowlist`:
    /// Boc/Cbz deprotection only (design doc §4.5). Shared verbatim by
    /// `conservative()` and `diagnostic()`.
    fn default_reagent_omission_allowlist() -> Vec<String> {
        vec![
            "rule:boc_deprotection_retro".to_string(),
            "rule:cbz_deprotection_retro".to_string(),
        ]
    }

    /// The strict default: both stock-terminal verification and
    /// target-element accounting are required; forward validation and
    /// evidence are not (design doc §4.7 -- neither validator's
    /// reliability has been measured yet, so neither is a hard
    /// requirement out of the box). See [`Self::diagnostic`]'s doc comment
    /// for what is -- and, importantly, currently is *not* -- different
    /// about that constructor.
    pub fn conservative() -> Self {
        Self {
            require_verified_stock_terminal: true,
            require_target_element_accounting: true,
            forward_validation_policy: ForwardValidationPolicy::Ignore,
            evidence_policy: EvidencePolicy::Ignore,
            reagent_omission_template_allowlist: Self::default_reagent_omission_allowlist(),
            max_routes_to_assess: DEFAULT_MAX_ROUTES_TO_ASSESS,
            include_all_route_diagnostics: false,
        }
    }

    /// For exploratory use. Every field value here is identical to
    /// [`Self::conservative`]'s -- per design doc §4.5's last paragraph,
    /// the two constructors are meant to differ in how a
    /// non-allowlisted `UnaccountedTargetElement` is classified
    /// (`HardFailure` under `conservative()`, `ValidationGap` under
    /// `diagnostic()`), but that distinction is *behavioral*: it is
    /// implemented downstream in `assessment.rs`'s branching logic, not
    /// encoded as a field on this struct.
    ///
    /// **Known open point, not resolved by this file -- blocks Agent C**:
    /// because every field is identical, a `SynthesizabilityConfig` value
    /// by itself does not let `assessment.rs` tell a `conservative()`-built
    /// config apart from a `diagnostic()`-built one at runtime -- there is
    /// no field to branch on. Agent C cannot implement §4.5's
    /// conservative-vs-diagnostic branching from this value alone, and
    /// cannot add a field to this struct (schema types are Agent A's job,
    /// per the agent split). Resolving this needs either (a)
    /// `assess_routes` taking an explicit mode parameter alongside the
    /// config, or (b) a field added to this struct on the integration
    /// branch before Agent C starts. The orchestrator must decide which,
    /// before Agent C's worktree branches off.
    pub fn diagnostic() -> Self {
        Self::conservative()
    }
}

/// Owned, serializable echo of every [`SynthesizabilityConfig`] field,
/// carried on [`SynthesizabilityAssessment::config_used`] so the
/// assessment output is self-describing and auditable without needing the
/// original config value. Plain owned types throughout (no references).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SynthesizabilityConfigSummary {
    pub require_verified_stock_terminal: bool,
    pub require_target_element_accounting: bool,
    pub forward_validation_policy: ForwardValidationPolicy,
    pub evidence_policy: EvidencePolicy,
    pub reagent_omission_template_allowlist: Vec<String>,
    pub max_routes_to_assess: usize,
    pub include_all_route_diagnostics: bool,
}

// ---------------------------------------------------------------------
// 5. AssessmentProvenance
// ---------------------------------------------------------------------

/// Everything needed to independently reproduce and audit one
/// [`SynthesizabilityAssessment`] (design doc §5): what rule set, stock
/// set, and config produced it, and hashes over each so a caller can
/// verify nothing silently changed between two runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssessmentProvenance {
    /// `env!("CARGO_PKG_VERSION")` of the RENKIN crate that produced this
    /// assessment.
    pub renkin_version: String,
    pub assessment_schema_version: u32,
    pub canonical_target: String,
    pub rules_count: usize,
    /// sha256 over sorted template ids + SMIRKS, same style as
    /// `ChemEnv::content_sha256`.
    pub rules_hash: String,
    pub stock_count: usize,
    /// sha256 over sorted, kernel-canonicalized stock entries.
    pub stock_hash: String,
    /// Caller-supplied label for where the stock set came from (a file
    /// path, `"embedded"`, etc.) -- never a silent default; `None` means
    /// the caller did not say.
    pub stock_source: Option<String>,
    pub template_metadata_hash: Option<String>,
    /// Caller-supplied echo of the `SearchConfig` used to produce the
    /// routes being assessed; this module does not re-run or infer it.
    pub search_config_summary: Option<String>,
    pub assessment_config_hash: String,
    /// Caller-supplied only -- **never computed in this crate**. Compiling
    /// `src/synthesizability/` for wasm32 rules out `std::process::Command`
    /// and filesystem probing of the build environment (design doc §5.1),
    /// so a git commit hash must come from the caller's own build/deploy
    /// context if it wants one recorded at all.
    pub git_commit: Option<String>,
    /// Caller-supplied only -- **never computed in this crate**, for the
    /// same wasm32 reason as `git_commit` (design doc §5.1): whether an
    /// embedded rule/stock fallback was used is a fact about the caller's
    /// loading path, not something this module can probe or infer.
    pub embedded_fallback_used: Option<bool>,
    /// Combines `rules_hash`, `stock_hash`, `assessment_config_hash`,
    /// `canonical_target`, and every route's `route_id` plus its
    /// `status`/`hard_failures`/`validation_gaps` (design doc §6). Two
    /// identical inputs must produce the same value here.
    pub reproducibility_hash: String,
    /// Documents what was deliberately left out of `reproducibility_hash`
    /// (e.g. `["timing_ms", "wall_clock_timestamp"]`) -- not just what was
    /// included. Nothing time-based is computed inside this module, so
    /// this is expected to enumerate only caller-stamped fields, if any.
    pub reproducibility_exclusions: Vec<String>,
}

// ---------------------------------------------------------------------
// 4.6 SynthesizabilityAssessment (top level)
// ---------------------------------------------------------------------

/// The full, machine-readable output of an assessment run (design doc
/// §4.6): a decisive, auditable verdict for one target under one
/// route/stock/evidence/config context -- never a claim that a molecule
/// *cannot* be synthesized, and never an uncalibrated score.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SynthesizabilityAssessment {
    pub schema_version: u32,
    /// The target SMILES exactly as supplied by the caller.
    pub target: String,
    /// The target's canonical SMILES. `None` only when `status` is
    /// `InvalidTarget` (the target didn't parse, so there is nothing to
    /// canonicalize).
    pub canonical_target: Option<String>,
    pub status: AssessmentStatus,
    /// The route selected by the lexicographic classification in design
    /// doc §4.8, if any route qualifies (`status` in
    /// `{RouteSupported, RouteSupportedWithValidationGaps}`). `None` if no
    /// route qualifies, even when `route_assessments` is non-empty.
    pub selected_route: Option<RouteAssessment>,
    /// Every assessed route, in deterministic order (design doc §6).
    pub route_assessments: Vec<RouteAssessment>,
    pub provenance: AssessmentProvenance,
    /// Echo of the `SynthesizabilityConfig` used to produce this
    /// assessment, for self-describing audit output.
    pub config_used: SynthesizabilityConfigSummary,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conservative_config_has_documented_defaults() {
        let cfg = SynthesizabilityConfig::conservative();
        assert!(cfg.require_verified_stock_terminal);
        assert!(cfg.require_target_element_accounting);
        assert_eq!(
            cfg.forward_validation_policy,
            ForwardValidationPolicy::Ignore
        );
        assert_eq!(cfg.evidence_policy, EvidencePolicy::Ignore);
        assert_eq!(
            cfg.reagent_omission_template_allowlist,
            vec![
                "rule:boc_deprotection_retro".to_string(),
                "rule:cbz_deprotection_retro".to_string(),
            ]
        );
        assert!(
            !cfg.reagent_omission_template_allowlist
                .iter()
                .any(|t| t == "rule:aryl_amine_retro"),
            "aryl_amine_retro must not be on the default allowlist -- see issue #73"
        );
    }

    #[test]
    fn diagnostic_config_is_field_identical_to_conservative() {
        // Per design doc §4.5's last paragraph: these two constructors are
        // meant to differ in downstream policy interpretation (assessment.rs),
        // not in any SynthesizabilityConfig field value. This test pins that
        // down -- if a future edit makes them diverge, it must be deliberate
        // and update this test, not an accidental drift.
        assert_eq!(
            SynthesizabilityConfig::conservative(),
            SynthesizabilityConfig::diagnostic()
        );
    }

    #[test]
    fn statuses_implement_debug_and_partial_eq() {
        assert_eq!(
            AssessmentStatus::RouteSupported,
            AssessmentStatus::RouteSupported
        );
        assert_ne!(
            AssessmentStatus::RouteSupported,
            AssessmentStatus::InvalidTarget
        );
        assert_eq!(
            StockTerminationStatus::AllLeavesVerifiedInConfiguredStock,
            StockTerminationStatus::AllLeavesVerifiedInConfiguredStock
        );
        assert_eq!(
            ForwardValidationStatus::NotEvaluated,
            ForwardValidationStatus::NotEvaluated
        );
        assert_eq!(
            ElementAccountingStatus::Accounted,
            ElementAccountingStatus::Accounted
        );
        assert_eq!(
            HardFailure::RouteGraphInconsistent,
            HardFailure::RouteGraphInconsistent
        );
        assert_ne!(
            HardFailure::UnaccountedTargetElement { step_index: 0 },
            HardFailure::UnaccountedTargetElement { step_index: 1 }
        );
        assert_eq!(
            ValidationGap::BestEffortRouteOnly,
            ValidationGap::BestEffortRouteOnly
        );
        // Debug is exercised implicitly by every `assert_eq!`/`assert_ne!`
        // failure message above; format!() here proves it compiles for a
        // struct-carrying variant too.
        let _ = format!(
            "{:?}",
            HardFailure::StockTerminalMismatch {
                leaf: "CCO".to_string()
            }
        );
    }

    fn sample_route_assessment() -> RouteAssessment {
        RouteAssessment {
            route_id: "sha256:deadbeef".to_string(),
            route_depth: 2,
            route_cost: 1.5,
            stock_termination_status: StockTerminationStatus::AllLeavesVerifiedInConfiguredStock,
            target_element_accounting_status: ElementAccountingStatus::Accounted,
            forward_validation_status: ForwardValidationStatus::NotEvaluated,
            evidence_coverage: EvidenceCoverage::default(),
            hard_failures: vec![],
            validation_gaps: vec![ValidationGap::ForwardValidationNotRun],
            warnings: vec![],
        }
    }

    #[test]
    fn synthesizability_assessment_smoke_test_serializes() {
        let route = sample_route_assessment();
        let assessment = SynthesizabilityAssessment {
            schema_version: SYNTHESIZABILITY_SCHEMA_VERSION,
            target: "c1ccccc1".to_string(),
            canonical_target: Some("c1ccccc1".to_string()),
            status: AssessmentStatus::RouteSupportedWithValidationGaps,
            selected_route: Some(route.clone()),
            route_assessments: vec![route],
            provenance: AssessmentProvenance {
                renkin_version: "0.0.0-test".to_string(),
                assessment_schema_version: SYNTHESIZABILITY_SCHEMA_VERSION,
                canonical_target: "c1ccccc1".to_string(),
                rules_count: 1,
                rules_hash: "sha256:rules".to_string(),
                stock_count: 1,
                stock_hash: "sha256:stock".to_string(),
                stock_source: None,
                template_metadata_hash: None,
                search_config_summary: None,
                assessment_config_hash: "sha256:config".to_string(),
                git_commit: None,
                embedded_fallback_used: None,
                reproducibility_hash: "sha256:repro".to_string(),
                reproducibility_exclusions: vec!["wall_clock_timestamp".to_string()],
            },
            config_used: SynthesizabilityConfigSummary {
                require_verified_stock_terminal: true,
                require_target_element_accounting: true,
                forward_validation_policy: ForwardValidationPolicy::Ignore,
                evidence_policy: EvidencePolicy::Ignore,
                reagent_omission_template_allowlist: vec![
                    "rule:boc_deprotection_retro".to_string(),
                ],
                max_routes_to_assess: 10,
                include_all_route_diagnostics: false,
            },
            warnings: vec![],
        };

        let json = serde_json::to_string(&assessment).expect("must serialize without panicking");
        assert!(json.contains("\"route_supported_with_validation_gaps\""));
        // Explicit nulls, not omitted keys, for audit Option fields.
        assert!(json.contains("\"stock_source\":null"));
        assert!(json.contains("\"git_commit\":null"));
    }
}
