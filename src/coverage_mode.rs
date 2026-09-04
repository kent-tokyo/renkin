//! Shared Stage-1/Stage-2 "coverage mode" orchestration (v0.24, Phase
//! 41.18B), per `docs/design/coverage-mode-v0.md`. One implementation,
//! called from both `src/main.rs` (CLI) and `src/python.rs` (PyO3) -- the
//! whole point of this module existing is that neither of those files
//! duplicates staged-escalation logic.
//!
//! Stage 1 always runs via [`crate::search::find_routes`], byte-identical
//! to standard mode. Stage 2 only runs when Stage 1 found nothing, via
//! [`crate::search::find_routes_with_control`] against a separately loaded,
//! larger template set. A Stage-1 valid route is never overwritten -- there
//! is no merge step, Stage 2 simply never runs when Stage 1 already solved
//! the target (see [`run_coverage_mode_with_configs`]'s doc for exactly
//! where that branch lives).
//!
//! Not available on `wasm32`: [`crate::search::SearchControl::with_timeout`]
//! needs a monotonic clock this crate's supported wasm targets don't have,
//! and coverage mode is a CLI/Python-only feature per the design doc.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::chem_env::{ChemEnv, PreparedRuleSet, RetroRule, default_rules};
use crate::search::{self, Route, SearchConfig, SearchControl, SearchStats, SearchTermination};

/// Which stage's routes were actually returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectedStage {
    Stage1,
    Stage2,
}

/// Result of a coverage-mode search -- `routes` plus the observability
/// fields `docs/design/coverage-mode-v0.md` §5 specifies for the CLI/Python
/// output surface.
#[derive(Debug)]
pub struct CoverageModeResult {
    pub routes: Vec<Route>,
    pub selected_stage: SelectedStage,
    /// The selected stage's own [`crate::search::SearchStats`] -- Stage 1's
    /// if it solved, Stage 2's if it ran (regardless of whether Stage 2
    /// itself found anything). Lets a caller reuse every existing
    /// standard-mode diagnostic (`nodes_expanded`, `search_diagnostics`,
    /// the no-route-found `diagnostics` block, ...) unchanged against
    /// whichever stage actually produced the returned `routes` -- exactly
    /// like `find_routes`'s own `SearchStats` return value, just sourced
    /// from a different stage depending on what happened.
    pub stats: SearchStats,
    pub stage1_solved: bool,
    pub stage2_invoked: bool,
    pub stage1_elapsed_ms: f64,
    pub stage2_elapsed_ms: Option<f64>,
    pub total_elapsed_ms: f64,
    /// Stage 1 never has an internal timeout in v0 (same as standard mode,
    /// which has never had one) -- always `false`. Kept as an explicit
    /// field rather than omitted, matching the design doc's field list and
    /// leaving room for a future Stage-1 budget without a breaking change.
    pub stage1_timeout: bool,
    pub stage2_timeout: bool,
    /// Summed across every stage that actually ran (Stage 1 alone, or
    /// Stage 1 + Stage 2) -- not just the selected/winning stage's count,
    /// unlike `stats.reranker_failures` above (which is only ever the
    /// selected stage's own count). See `docs/design/coverage-mode-v0.md`
    /// §5 for why this differs from the research harness's own
    /// selected-stage-only semantic projection.
    pub reranker_failures: u64,
}

/// Reusable Stage-1/Stage-2 assets for batch coverage searches.
///
/// Frontiers and all mutable search caches remain per-target and per-stage;
/// only immutable compiled reactions are shared. This preserves coverage
/// mode's fresh Stage-2 semantics while avoiding repeated template compilation
/// in benchmark and other long-lived callers.
pub struct CoverageSearchContext<'a> {
    env: &'a ChemEnv,
    stage1_rules: &'a [RetroRule],
    stage2_rules: &'a [RetroRule],
    prepared_rules: PreparedRuleSet,
}

impl<'a> CoverageSearchContext<'a> {
    pub fn new(
        env: &'a ChemEnv,
        stage1_rules: &'a [RetroRule],
        stage2_rules: &'a [RetroRule],
    ) -> Self {
        Self {
            env,
            stage1_rules,
            stage2_rules,
            prepared_rules: PreparedRuleSet::from_rule_sets(&[stage1_rules, stage2_rules]),
        }
    }

    pub fn run(
        &self,
        target_smiles: &str,
        config: &SearchConfig,
        stage2_timeout: Option<Duration>,
    ) -> Result<CoverageModeResult> {
        self.run_with_stage2_beam(target_smiles, config, stage2_timeout, None)
    }

    pub fn run_with_stage2_beam(
        &self,
        target_smiles: &str,
        config: &SearchConfig,
        stage2_timeout: Option<Duration>,
        stage2_beam_width: Option<usize>,
    ) -> Result<CoverageModeResult> {
        let stage2_config = stage2_beam_width.map(|beam_width| {
            let mut config = config.clone();
            config.beam_width = beam_width;
            config
        });
        self.run_with_configs(
            target_smiles,
            config,
            stage2_config.as_ref().unwrap_or(config),
            stage2_timeout,
        )
    }

    pub fn run_with_configs(
        &self,
        target_smiles: &str,
        stage1_config: &SearchConfig,
        stage2_config: &SearchConfig,
        stage2_timeout: Option<Duration>,
    ) -> Result<CoverageModeResult> {
        run_coverage_mode_with_configs_prepared(
            target_smiles,
            self.env,
            self.stage1_rules,
            stage1_config,
            self.stage2_rules,
            stage2_config,
            stage2_timeout,
            &self.prepared_rules,
        )
    }
}

/// Rejects unsupported option combinations by **flag presence alone** --
/// `bond_index`, an ONNX `--scorer`, and an active `--ring-context-policy`
/// each would need their own Stage-2-specific validation (a retrieval index
/// / scorer vocabulary / ring-context sidecar built against the *coverage*
/// template set) that does not exist yet in v0. Deliberately takes plain
/// booleans, not a built [`SearchConfig`]/`RingContextConfig` -- an active
/// ring-context policy or ONNX scorer is only reachable today by first
/// *loading* a real sidecar/model file, and this check needs to fire
/// **before** either of those loads happens (a `--search-mode coverage
/// --ring-context-policy conservative` call with a bogus/nonexistent
/// `--ring-context-sidecar` path must still be rejected for the
/// combination itself, not fail with an unrelated "sidecar not found"
/// error first). Callers (CLI, Python) compute presence from their own raw
/// flags/kwargs, before doing any of that loading. Standard mode is
/// entirely unaffected; this is only ever called on the coverage-mode
/// path.
pub fn validate_coverage_mode_flags(
    bond_index: bool,
    ring_context_policy_active: bool,
    onnx_scorer_active: bool,
) -> Result<()> {
    if bond_index {
        bail!(
            "coverage mode does not support --bond-index in v0 -- Stage 2 would need its own, \
             separately validated retrieval index against the coverage template set"
        );
    }
    if onnx_scorer_active {
        bail!(
            "coverage mode does not support an ONNX --scorer in v0 -- Stage 2 would need its \
             own, separately validated scorer vocabulary against the coverage template set"
        );
    }
    if ring_context_policy_active {
        bail!(
            "coverage mode does not support an active --ring-context-policy in v0 -- Stage 2 \
             would need its own, separately validated ring-context sidecar against the coverage \
             template set"
        );
    }
    Ok(())
}

/// Defensive backstop over [`validate_coverage_mode_flags`], checked
/// internally by [`run_coverage_mode_with_configs`] against the already-
/// built [`SearchConfig`] it was given -- catches a caller that skipped
/// the early, pre-loading flag check above. By the time a `SearchConfig`
/// exists, an active ring-context policy or ONNX scorer has *already* been
/// loaded, so this can no longer prevent that load -- it exists purely as
/// a second line of defense, not the primary mechanism (CLI/Python are
/// expected to call [`validate_coverage_mode_flags`] on their raw
/// flags/kwargs before doing any of that loading).
pub fn validate_coverage_mode_config(config: &SearchConfig) -> Result<()> {
    let ring_context_policy_active = !matches!(
        config.ring_context,
        crate::ring_context::RingContextConfig::Disabled
    );
    #[cfg(feature = "nn-scoring")]
    let onnx_scorer_active = config.nn_scorer.is_some();
    #[cfg(not(feature = "nn-scoring"))]
    let onnx_scorer_active = false;
    validate_coverage_mode_flags(
        config.bond_index,
        ring_context_policy_active,
        onnx_scorer_active,
    )
}

/// Loads and validates the Stage-2 rule set from `coverage_templates_path`,
/// fail-loud, called before any search runs (including Stage 1) -- so a
/// bad `--coverage-templates` path is reported immediately, not only after
/// Stage 1 already ran and turned out to need it.
///
/// [`crate::chem_env::load_rules_from_file`] itself is NOT fail-loud (a
/// missing/unreadable file just prints a warning and returns an empty
/// `Vec`) -- that is the right default for the pre-existing `--templates`
/// flag (an empty extra set silently falls back to hand-crafted rules
/// alone), but wrong for coverage mode, where an unnoticed empty Stage-2
/// rule set would make every escalation silently useless. This wrapper
/// checks the path exists and is a file before attempting to read it, and
/// separately rejects a file that parsed to zero usable templates (e.g.
/// every line malformed) rather than treating that the same as "no file
/// given."
pub fn load_coverage_rules(coverage_templates_path: &str) -> Result<Vec<RetroRule>> {
    crate::chem_env::validate_template_file(coverage_templates_path)?;
    let metadata = std::fs::metadata(coverage_templates_path).with_context(|| {
        format!(
            "--coverage-templates path does not exist or is not readable: \
             {coverage_templates_path}"
        )
    })?;
    if !metadata.is_file() {
        bail!("--coverage-templates path is not a file: {coverage_templates_path}");
    }
    // Explicit read-as-UTF-8 check, kept distinct from "parsed fine but
    // found zero valid template lines" below.
    // `chem_env::load_rules_from_file` itself swallows a read/decode
    // failure into a stderr warning + empty `Vec` (the right default for
    // the pre-existing `--templates` flag), which would otherwise make
    // this function misreport a permission error or non-UTF-8 file
    // content as "file contains no valid templates" -- a different,
    // misleading failure mode from what actually happened. This does mean
    // the file is read twice (here, and again inside
    // `load_rules_from_file`'s own parse pass) -- accepted: this runs once
    // per coverage-mode invocation, on a small text file, not a hot path.
    let content =
        crate::io_limits::read_bounded_text_file(coverage_templates_path, "--coverage-templates")?;
    let extra = crate::chem_env::load_rules_from_content(&content);
    if extra.is_empty() {
        bail!("--coverage-templates file contains no valid templates: {coverage_templates_path}");
    }
    let mut rules = default_rules();
    rules.extend(extra);
    Ok(rules)
}

/// Core orchestration function. Stage 1 and Stage 2 take **independent**
/// rule sets and [`SearchConfig`]s. Production callers always pass the
/// *same* config for both (see [`run_coverage_mode`] below) -- isolating
/// the coverage template count as the only variable between stages, per
/// the pre-registered Phase B.2 measurement this whole feature is built
/// on. The split exists at this layer specifically so tests can prove
/// Stage 2 is never invoked when Stage 1 already solved the target, using
/// a Stage-2-only config/reranker that would panic if actually used --
/// see `coverage_mode::tests::stage1_solved_never_invokes_stage2` in this
/// file.
///
/// **Stage 1's valid route is never overwritten by construction, not by a
/// priority rule applied after the fact**: Stage 2 only ever runs in the
/// `else` branch below, when Stage 1's `routes` came back empty. There is
/// no merge step and no "which one wins" decision to get wrong.
pub fn run_coverage_mode_with_configs(
    target_smiles: &str,
    env: &ChemEnv,
    stage1_rules: &[RetroRule],
    stage1_config: &SearchConfig,
    stage2_rules: &[RetroRule],
    stage2_config: &SearchConfig,
    stage2_timeout: Option<Duration>,
) -> Result<CoverageModeResult> {
    CoverageSearchContext::new(env, stage1_rules, stage2_rules).run_with_configs(
        target_smiles,
        stage1_config,
        stage2_config,
        stage2_timeout,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_coverage_mode_with_configs_prepared(
    target_smiles: &str,
    env: &ChemEnv,
    stage1_rules: &[RetroRule],
    stage1_config: &SearchConfig,
    stage2_rules: &[RetroRule],
    stage2_config: &SearchConfig,
    stage2_timeout: Option<Duration>,
    prepared_rules: &PreparedRuleSet,
) -> Result<CoverageModeResult> {
    validate_coverage_mode_config(stage1_config)?;

    let total_start = Instant::now();

    let stage1_start = Instant::now();
    let stage1_result = search::find_routes_with_control_prepared(
        target_smiles,
        env,
        stage1_rules,
        stage1_config,
        &SearchControl::unlimited(),
        prepared_rules,
        None,
    )?;
    let stage1_routes = stage1_result.routes;
    let stage1_stats = stage1_result.stats;
    let stage1_elapsed_ms = stage1_start.elapsed().as_secs_f64() * 1000.0;
    let stage1_solved = !stage1_routes.is_empty();

    if stage1_solved {
        let reranker_failures = stage1_stats.reranker_failures;
        return Ok(CoverageModeResult {
            routes: stage1_routes,
            selected_stage: SelectedStage::Stage1,
            stats: stage1_stats,
            stage1_solved: true,
            stage2_invoked: false,
            stage1_elapsed_ms,
            stage2_elapsed_ms: None,
            total_elapsed_ms: total_start.elapsed().as_secs_f64() * 1000.0,
            stage1_timeout: false,
            stage2_timeout: false,
            reranker_failures,
        });
    }

    // Stage 1 found nothing -- escalate. Frontier, closed-set, and candidate
    // caches remain fully independent per the pre-registered Phase B.2
    // constraint. Only immutable SMIRKS compilation state is shared.
    let stage2_start = Instant::now();
    let control = match stage2_timeout {
        Some(d) => SearchControl::with_timeout(d),
        None => SearchControl::unlimited(),
    };
    let stage2_result = search::find_routes_with_control_prepared(
        target_smiles,
        env,
        stage2_rules,
        stage2_config,
        &control,
        prepared_rules,
        None,
    )?;
    let stage2_elapsed_ms = stage2_start.elapsed().as_secs_f64() * 1000.0;

    Ok(stage2_outcome_to_result(
        stage2_result,
        stage1_stats.reranker_failures,
        stage1_elapsed_ms,
        stage2_elapsed_ms,
        total_start.elapsed().as_secs_f64() * 1000.0,
    ))
}

/// Pure conversion from Stage 2's raw [`search::SearchRunResult`] into the
/// Stage-2 half of a [`CoverageModeResult`] -- pulled out specifically so
/// the "does the orchestrator faithfully relay whatever Stage 2 found,
/// including a nonempty partial result on `DeadlineExceeded`" question can
/// be tested deterministically, by constructing a synthetic
/// `SearchRunResult` directly, instead of racing a real search against a
/// wall-clock deadline to try to land mid-timeout. The underlying
/// guarantee that a real search retains routes found before its deadline
/// -- rather than discarding them -- is `find_routes_with_control`'s own
/// responsibility, already exhaustively verified by
/// `search::cooperative_cancellation_tests::
/// valid_routes_found_before_deadline_are_not_discarded` (Phase 41.18A).
/// This function's only job is to not lose or alter anything on the way
/// through into a `CoverageModeResult` -- see
/// `coverage_mode::tests::stage2_outcome_conversion_is_a_faithful_passthrough`.
fn stage2_outcome_to_result(
    stage2_result: search::SearchRunResult,
    stage1_reranker_failures: u64,
    stage1_elapsed_ms: f64,
    stage2_elapsed_ms: f64,
    total_elapsed_ms: f64,
) -> CoverageModeResult {
    let stage2_timed_out = stage2_result.termination == SearchTermination::DeadlineExceeded;
    let reranker_failures = stage1_reranker_failures + stage2_result.stats.reranker_failures;
    CoverageModeResult {
        routes: stage2_result.routes,
        selected_stage: SelectedStage::Stage2,
        stats: stage2_result.stats,
        stage1_solved: false,
        stage2_invoked: true,
        stage1_elapsed_ms,
        stage2_elapsed_ms: Some(stage2_elapsed_ms),
        total_elapsed_ms,
        stage1_timeout: false,
        stage2_timeout: stage2_timed_out,
        reranker_failures,
    }
}

/// Production entry point: both stages share the identical `config` --
/// only `stage1_rules` vs. `coverage_rules` differs between them. This is
/// what `src/main.rs` and `src/python.rs` both call.
pub fn run_coverage_mode(
    target_smiles: &str,
    env: &ChemEnv,
    stage1_rules: &[RetroRule],
    config: &SearchConfig,
    coverage_rules: &[RetroRule],
    stage2_timeout: Option<Duration>,
) -> Result<CoverageModeResult> {
    run_coverage_mode_with_stage2_beam(
        target_smiles,
        env,
        stage1_rules,
        config,
        coverage_rules,
        stage2_timeout,
        None,
    )
}

/// Coverage entry point with an optional Stage-2-only beam width override.
/// Stage 1 remains byte-identical to `config`; when `None`, this is exactly
/// the legacy `run_coverage_mode` behavior. This lets callers address
/// Stage-2 beam crowd-out without changing the default search budget.
pub fn run_coverage_mode_with_stage2_beam(
    target_smiles: &str,
    env: &ChemEnv,
    stage1_rules: &[RetroRule],
    config: &SearchConfig,
    coverage_rules: &[RetroRule],
    stage2_timeout: Option<Duration>,
    stage2_beam_width: Option<usize>,
) -> Result<CoverageModeResult> {
    let stage2_config = stage2_beam_width.map(|beam_width| {
        let mut config = config.clone();
        config.beam_width = beam_width;
        config
    });
    let stage2_config = stage2_config.as_ref().unwrap_or(config);
    run_coverage_mode_with_configs(
        target_smiles,
        env,
        stage1_rules,
        config,
        coverage_rules,
        stage2_config,
        stage2_timeout,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::{default_rules, load_rules_from_file};

    fn env() -> ChemEnv {
        ChemEnv::load("data/building_blocks.smi").unwrap_or_else(|_| {
            ChemEnv::in_memory(&["CC(=O)O", "Oc1ccccc1C(=O)O", "c1ccccc1C(=O)O", "C", "O"])
        })
    }

    fn cfg() -> SearchConfig {
        SearchConfig {
            max_depth: 5,
            max_routes: 5,
            beam_width: 0,
            ..Default::default()
        }
    }

    /// A shallower, beam-limited config specifically for
    /// `SOLVABLE_ONLY_WITH_FIXTURE_TEMPLATES` -- that target is unsolvable
    /// by `default_rules()` alone only at these narrower settings; at
    /// `cfg()`'s depth=5/unlimited-beam, default_rules() alone eventually
    /// solves it too (deeper, unconstrained search finds more), which
    /// would make the "Stage 1 genuinely couldn't solve this" premise
    /// false. Depth/beam-width here match the exact settings used to
    /// choose this fixture (see `tests/coverage_mode_cli.rs`'s module doc).
    fn shallow_beam_limited_cfg() -> SearchConfig {
        SearchConfig {
            max_depth: 2,
            max_routes: 5,
            beam_width: 100,
            ..Default::default()
        }
    }

    const ASPIRIN: &str = "CC(=O)Oc1ccccc1C(=O)O";
    // Solved by default_rules() alone (a real, nonempty rule set -- not an
    // empty Vec) via friedel_crafts_acylation_retro at max_depth<=3. Used
    // wherever a test needs Stage 1 to genuinely solve something using its
    // *actual* rules, as opposed to trivially succeeding because it was
    // handed nothing to fail with.
    const SOLVABLE_BY_DEFAULT_RULES_ALONE: &str = "CCN(CC)C(=O)c1ccccc1F";
    // Unsolved by default_rules() alone at shallow_beam_limited_cfg()'s
    // settings (depth=2, beam_width=100 -- NOT at cfg()'s depth=5/
    // unlimited-beam, where default_rules() alone eventually solves it
    // too), solved once the fixture template
    // (tests/fixtures/coverage_mode_templates.smi's two lines, loaded here
    // directly since unit tests don't go through the CLI) is added -- see
    // that fixture file's own header comment for provenance. Used
    // wherever a test needs two *specific, genuinely different* nonempty
    // rule sets where only one can solve the target, as opposed to "empty
    // vs. nonempty" (which only proves "Stage 2 had *some* rules," not
    // "Stage 2 had *its own* rules"). Always pair with
    // `shallow_beam_limited_cfg()`, never `cfg()`.
    const SOLVABLE_ONLY_WITH_FIXTURE_TEMPLATES: &str = "O=C1CCC(=O)N1c1ccccc1";
    // Deliberately unsolvable at any reasonable depth with an empty rule
    // set and no building-block match -- used to force Stage 1 to come
    // back empty so Stage 2 actually runs.
    const UNKNOWN: &str = "c1ccc2c(c1)c1ccccc1c1ccccc21"; // pyrene, not a building block

    fn fixture_rules() -> Vec<RetroRule> {
        load_rules_from_file("tests/fixtures/coverage_mode_templates.smi")
    }

    struct PanicReranker;
    impl crate::candidate::CandidateReranker for PanicReranker {
        fn score_pool(
            &self,
            _target: &str,
            _candidates: &mut [crate::candidate::ReactionCandidate],
        ) -> anyhow::Result<()> {
            panic!("PanicReranker::score_pool was called -- Stage 2 must not have run");
        }
    }

    /// Always fails, and counts how many times it was actually invoked.
    /// Unlike a reranker that always succeeds (which can never distinguish
    /// "summed across both stages" from "just the selected stage's count,"
    /// since a healthy reranker's failure count is 0 either way), this
    /// double guarantees each stage that invokes it contributes exactly 1
    /// to that stage's own `reranker_failures` (the first failure disables
    /// the reranker for the remainder of that stage's search, per
    /// `search.rs`'s `active_reranker` degrade-once contract) -- so a
    /// two-stage run's summed total is only correct if it's genuinely 2,
    /// not 1.
    struct FailingReranker(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl crate::candidate::CandidateReranker for FailingReranker {
        fn score_pool(
            &self,
            _target: &str,
            _candidates: &mut [crate::candidate::ReactionCandidate],
        ) -> anyhow::Result<()> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            anyhow::bail!("FailingReranker: deliberate failure for aggregation test")
        }
    }

    fn synthetic_route() -> Route {
        Route {
            steps: vec![],
            depth: 0,
            score: 0.0,
            building_blocks: vec![],
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 0.0,
        }
    }

    // Requirement: Stage 1 solved => Stage 2 search not invoked, proven by
    // a Stage-2-only config carrying a reranker that panics if `score_pool`
    // is ever called -- not just checking `stage2_invoked` after the fact,
    // which could pass even if the implementation were subtly wrong about
    // *which* function actually ran. Independently mutation-verified (by
    // hand, mirroring the method PR #119's review used): temporarily
    // removed the Stage-1-solved early return in
    // `run_coverage_mode_with_configs`, reran, confirmed this test fails
    // with exactly the expected panic; restored, confirmed clean.
    #[test]
    fn stage1_solved_never_invokes_stage2() {
        let env = env();
        let stage1_rules = default_rules();
        let stage2_rules = default_rules();
        let stage1_config = cfg();
        let stage2_config = SearchConfig {
            reranker: Some(std::sync::Arc::new(PanicReranker)),
            ..cfg()
        };

        // Acetic acid is a building block -- Stage 1 solves at depth 0
        // without needing the coverage rules or Stage 2's reranker at all.
        let result = run_coverage_mode_with_configs(
            "CC(=O)O",
            &env,
            &stage1_rules,
            &stage1_config,
            &stage2_rules,
            &stage2_config,
            None,
        )
        .unwrap();

        assert_eq!(result.selected_stage, SelectedStage::Stage1);
        assert!(result.stage1_solved);
        assert!(!result.stage2_invoked);
        assert!(!result.routes.is_empty());
        assert!(result.stage2_elapsed_ms.is_none());
    }

    #[test]
    fn stage1_unsolved_invokes_stage2() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![]; // guarantees Stage 1 finds nothing
        let stage2_rules = default_rules();
        let result =
            run_coverage_mode(ASPIRIN, &env, &stage1_rules, &cfg(), &stage2_rules, None).unwrap();

        assert!(!result.stage1_solved);
        assert!(result.stage2_invoked);
        assert_eq!(result.selected_stage, SelectedStage::Stage2);
        assert!(result.stage2_elapsed_ms.is_some());
        // Aspirin is solvable with the real default rules used as coverage_rules.
        assert!(!result.routes.is_empty());
    }

    // Requirement: Stage 1's valid route is never overwritten. Uses a
    // target with a *real*, non-trivial (depth >= 1) Stage-1 route -- not
    // a depth-0 building-block target, where "the route" is just the
    // target itself and can't actually demonstrate anything about route
    // *content* being preserved -- and asserts the coverage-mode result's
    // route content is byte-identical to a fresh, direct `find_routes`
    // baseline call using the exact same Stage-1 rules. `stage2_rules`
    // here is default_rules() *plus* the fixture templates (genuinely
    // more/different than Stage 1's rules, unlike an earlier version of
    // this test which used the same rules for both stages and could not
    // have detected an overwrite even if one occurred) -- Stage 2 never
    // runs, so this is entirely about proving Stage 1's own output passes
    // through unmodified, not about Stage 2's rules being distinct (that's
    // `stage2_uses_its_own_rules_not_stage1_rules`'s job).
    #[test]
    fn stage1_valid_route_never_overwritten() {
        let env = env();
        let stage1_rules = default_rules();
        let mut stage2_rules = default_rules();
        stage2_rules.extend(fixture_rules());

        let baseline =
            search::find_routes(SOLVABLE_BY_DEFAULT_RULES_ALONE, &env, &stage1_rules, &cfg())
                .unwrap();
        assert!(
            !baseline.0.is_empty(),
            "fixture target must be Stage-1-solvable"
        );

        let result = run_coverage_mode(
            SOLVABLE_BY_DEFAULT_RULES_ALONE,
            &env,
            &stage1_rules,
            &cfg(),
            &stage2_rules,
            None,
        )
        .unwrap();

        assert_eq!(result.selected_stage, SelectedStage::Stage1);
        assert!(!result.stage2_invoked);
        assert_eq!(
            serde_json::to_string(&result.routes).unwrap(),
            serde_json::to_string(&baseline.0).unwrap(),
            "coverage mode's Stage-1 result must be byte-identical to a direct find_routes call \
             with the same Stage-1 rules -- anything else means it was altered on the way through"
        );
    }

    // Requirement: Stage 2 uses Stage 2's rules, not Stage 1's. Uses two
    // genuinely different, both-nonempty rule sets (default_rules() alone
    // vs. default_rules()+fixture) where only the larger one can solve the
    // target -- proves Stage 2 used *its own, specific* rules, not merely
    // "some nonempty rules" (which an empty-vs-nonempty version of this
    // test could not distinguish from Stage 2 accidentally reusing Stage
    // 1's rules, if Stage 1's rules had happened to be nonempty too).
    #[test]
    fn stage2_uses_its_own_rules_not_stage1_rules() {
        let env = env();
        let stage1_rules = default_rules(); // real, nonempty, cannot solve this target
        let mut stage2_rules = default_rules();
        stage2_rules.extend(fixture_rules()); // real, nonempty, CAN solve this target

        let stage1_only = search::find_routes(
            SOLVABLE_ONLY_WITH_FIXTURE_TEMPLATES,
            &env,
            &stage1_rules,
            &shallow_beam_limited_cfg(),
        )
        .unwrap();
        assert!(
            stage1_only.0.is_empty(),
            "fixture target must NOT be solvable by Stage 1's rules alone"
        );

        let result = run_coverage_mode(
            SOLVABLE_ONLY_WITH_FIXTURE_TEMPLATES,
            &env,
            &stage1_rules,
            &shallow_beam_limited_cfg(),
            &stage2_rules,
            None,
        )
        .unwrap();
        assert!(result.stage2_invoked);
        assert_eq!(result.selected_stage, SelectedStage::Stage2);
        assert!(
            !result.routes.is_empty(),
            "Stage 2 must have used its own (larger) rule set, not Stage 1's"
        );
    }

    #[test]
    fn stage2_timeout_is_surfaced() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![];
        let stage2_rules = default_rules();
        let result = run_coverage_mode(
            ASPIRIN,
            &env,
            &stage1_rules,
            &cfg(),
            &stage2_rules,
            Some(Duration::from_nanos(1)),
        )
        .unwrap();
        assert!(result.stage2_invoked);
        assert!(result.stage2_timeout);
    }

    // Requirement: partial valid routes found before a Stage-2 timeout are
    // retained. Deliberately NOT a wall-clock race (an earlier version of
    // this test sampled fractions of a measured baseline duration, which
    // review correctly flagged as exactly the flaky-test pattern this
    // program has consistently avoided elsewhere). Instead: exercises
    // `stage2_outcome_to_result` -- the pure function that converts
    // Stage 2's raw `SearchRunResult` into a `CoverageModeResult` --
    // directly, with a synthetic `DeadlineExceeded` result carrying
    // nonempty routes. The underlying "does a real search actually retain
    // routes found before its deadline" guarantee is
    // `find_routes_with_control`'s own responsibility, already
    // exhaustively verified by
    // `search::cooperative_cancellation_tests::valid_routes_found_before_deadline_are_not_discarded`
    // (Phase 41.18A) -- this test's only job is to prove the orchestrator
    // doesn't drop or alter anything on the way through, which is
    // deterministic and needs no timing at all.
    #[test]
    fn stage2_outcome_conversion_retains_partial_routes_on_timeout() {
        let synthetic = search::SearchRunResult {
            routes: vec![synthetic_route()],
            stats: SearchStats::default(),
            termination: SearchTermination::DeadlineExceeded,
        };
        let result = stage2_outcome_to_result(synthetic, 0, 10.0, 20.0, 30.0);
        assert_eq!(result.routes.len(), 1);
        assert!(result.stage2_timeout);
        assert_eq!(result.selected_stage, SelectedStage::Stage2);
        assert!(result.stage2_invoked);
        assert_eq!(result.stage1_elapsed_ms, 10.0);
        assert_eq!(result.stage2_elapsed_ms, Some(20.0));
        assert_eq!(result.total_elapsed_ms, 30.0);
    }

    #[test]
    fn stage2_outcome_conversion_preserves_completed_routes_too() {
        let synthetic = search::SearchRunResult {
            routes: vec![synthetic_route(), synthetic_route()],
            stats: SearchStats::default(),
            termination: SearchTermination::Completed,
        };
        let result = stage2_outcome_to_result(synthetic, 0, 1.0, 2.0, 3.0);
        assert_eq!(result.routes.len(), 2);
        assert!(!result.stage2_timeout);
    }

    // Requirement: reranker_failures summed across invoked stages.
    // Distinguishes "sum" from "selected-stage-only" for real: a reranker
    // that always fails contributes exactly 1 failure per stage that
    // invokes it (the first failure disables it for that stage's
    // remainder), so a correct two-stage run must report 2, while the
    // selected (Stage 2) stage's own `stats.reranker_failures` is only 1 --
    // if these two numbers were ever equal, this test would not have
    // caught the earlier version's aggregation bug the way it's designed
    // to. (Verified by hand: changing the sum to
    // `stage2_result.stats.reranker_failures` alone reproduces exactly the
    // vacuous-test failure mode review flagged -- `result.reranker_failures`
    // drops to 1 and this assertion fails.)
    #[test]
    fn reranker_failures_summed_across_invoked_stages() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![]; // Stage 1 attempts (0 candidates) -> escalates
        let stage2_rules = default_rules(); // Stage 2 does real work
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stage_config = SearchConfig {
            reranker: Some(std::sync::Arc::new(FailingReranker(call_count.clone()))),
            ..cfg()
        };
        let result = run_coverage_mode(
            ASPIRIN,
            &env,
            &stage1_rules,
            &stage_config,
            &stage2_rules,
            None,
        )
        .unwrap();
        assert!(result.stage2_invoked);
        assert_eq!(
            result.stats.reranker_failures, 1,
            "the selected (Stage 2) stage's own count must be exactly 1"
        );
        assert_eq!(
            result.reranker_failures, 2,
            "summed across both stages (1 + 1), not just the selected stage's own count"
        );
        assert!(
            call_count.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "the reranker must have actually been invoked by both stages, not just one"
        );
    }

    #[test]
    fn elapsed_and_stage_fields_are_self_consistent() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![];
        let stage2_rules = default_rules();
        let result =
            run_coverage_mode(ASPIRIN, &env, &stage1_rules, &cfg(), &stage2_rules, None).unwrap();
        assert!(result.stage1_elapsed_ms >= 0.0);
        assert!(result.stage2_elapsed_ms.unwrap() >= 0.0);
        assert!(result.total_elapsed_ms >= result.stage1_elapsed_ms);
        assert!(result.total_elapsed_ms >= result.stage2_elapsed_ms.unwrap());
        assert_eq!(result.selected_stage, SelectedStage::Stage2);
        assert!(result.stage2_invoked);
    }

    #[test]
    fn deterministic_repeated_output_with_sufficient_budget() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![];
        let stage2_rules = default_rules();
        let r1 =
            run_coverage_mode(ASPIRIN, &env, &stage1_rules, &cfg(), &stage2_rules, None).unwrap();
        let r2 =
            run_coverage_mode(ASPIRIN, &env, &stage1_rules, &cfg(), &stage2_rules, None).unwrap();
        assert_eq!(
            serde_json::to_string(&r1.routes).unwrap(),
            serde_json::to_string(&r2.routes).unwrap()
        );
        assert_eq!(r1.selected_stage, r2.selected_stage);
        assert_eq!(r1.reranker_failures, r2.reranker_failures);
    }

    #[test]
    fn validate_config_rejects_bond_index() {
        let config = SearchConfig {
            bond_index: true,
            ..Default::default()
        };
        assert!(validate_coverage_mode_config(&config).is_err());
    }

    // Requirement: unsupported combinations fail loud. These check
    // `validate_coverage_mode_flags` directly, on plain booleans -- no
    // real `RingContextConfig::Guarded`/ONNX-scorer value needs to be
    // constructed (deliberately: this is the whole point of the
    // flags-based check existing separately from `validate_coverage_mode_config`,
    // which needs an already-*loaded* config). An earlier version of this
    // ring-context test was an empty function body with a comment
    // claiming the check was covered elsewhere; it was not. Empty test
    // bodies are not used in this module.
    #[test]
    fn validate_flags_rejects_bond_index() {
        assert!(validate_coverage_mode_flags(true, false, false).is_err());
    }

    #[test]
    fn validate_flags_rejects_ring_context_active() {
        assert!(validate_coverage_mode_flags(false, true, false).is_err());
    }

    #[test]
    fn validate_flags_rejects_onnx_scorer_active() {
        assert!(validate_coverage_mode_flags(false, false, true).is_err());
    }

    #[test]
    fn validate_flags_accepts_all_inactive() {
        assert!(validate_coverage_mode_flags(false, false, false).is_ok());
    }

    #[test]
    fn load_coverage_rules_missing_path_fails_loud() {
        let result = load_coverage_rules("/nonexistent/path/does_not_exist.smi");
        assert!(result.is_err());
    }

    #[test]
    fn load_coverage_rules_directory_path_fails_loud() {
        let result = load_coverage_rules("data");
        assert!(result.is_err());
    }

    #[test]
    fn load_coverage_rules_empty_file_fails_loud() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "renkin_coverage_mode_test_empty_{}.smi",
            std::process::id()
        ));
        std::fs::write(&path, "# only a comment, no templates\n").unwrap();
        let result = load_coverage_rules(path.to_str().unwrap());
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
    }

    // Requirement: an unreadable (not just missing) coverage-templates
    // path fails loud, with an error message describing a *read* failure
    // -- not "contains no valid templates" (a different, misleading
    // failure mode). Uses invalid UTF-8 bytes rather than `chmod 000`:
    // a `chmod`-based fixture is not deterministic across environments
    // (root, and some CI runners, can still read a mode-000 file), while
    // `std::fs::read_to_string` deterministically rejects non-UTF-8
    // content regardless of who's reading it or what permissions say.
    #[test]
    fn unreadable_coverage_templates_path_reports_a_read_failure_not_missing_templates() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "renkin_coverage_mode_test_invalid_utf8_{}.smi",
            std::process::id()
        ));
        std::fs::write(&path, [0xFF, 0xFE, 0x00, 0xFF, 0xD8, 0x00]).unwrap();
        let result = load_coverage_rules(path.to_str().unwrap());
        let _ = std::fs::remove_file(&path);
        let err = result.expect_err("invalid UTF-8 content must fail loud");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("could not be read as valid UTF-8"),
            "error must describe a read failure, got: {msg}"
        );
        assert!(
            !msg.contains("contains no valid templates"),
            "must not be misreported as the empty-templates case: {msg}"
        );
    }

    #[test]
    fn unknown_target_via_coverage_mode_does_not_panic() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![];
        let stage2_rules: Vec<RetroRule> = vec![]; // Stage 2 also can't solve it
        let result = run_coverage_mode(UNKNOWN, &env, &stage1_rules, &cfg(), &stage2_rules, None);
        assert!(result.is_ok());
    }
}
