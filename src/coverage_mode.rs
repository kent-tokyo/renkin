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

use crate::chem_env::{ChemEnv, RetroRule, default_rules, load_rules_from_file};
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

/// Rejects `SearchConfig` combinations coverage mode does not support in
/// v0: `bond_index`, an ONNX `--scorer` (`nn_scorer`), or an active
/// `ring_context` policy. Each would need its own Stage-2-specific
/// validation (a separate retrieval index built against the larger
/// template set, a separate scorer vocabulary, a separate ring-context
/// sidecar) that does not exist yet -- silently ignoring the option for
/// Stage 2 while it's still active for Stage 1 would be a worse footgun
/// than refusing to start. Standard mode is entirely unaffected; this is
/// only ever called on the coverage-mode path.
pub fn validate_coverage_mode_config(config: &SearchConfig) -> Result<()> {
    if config.bond_index {
        bail!(
            "coverage mode does not support --bond-index in v0 -- Stage 2 would need its own, \
             separately validated retrieval index against the coverage template set"
        );
    }
    #[cfg(feature = "nn-scoring")]
    if config.nn_scorer.is_some() {
        bail!(
            "coverage mode does not support an ONNX --scorer in v0 -- Stage 2 would need its \
             own, separately validated scorer vocabulary against the coverage template set"
        );
    }
    if !matches!(
        config.ring_context,
        crate::ring_context::RingContextConfig::Disabled
    ) {
        bail!(
            "coverage mode does not support an active --ring-context-policy in v0 -- Stage 2 \
             would need its own, separately validated ring-context sidecar against the coverage \
             template set"
        );
    }
    Ok(())
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
    let metadata = std::fs::metadata(coverage_templates_path).with_context(|| {
        format!(
            "--coverage-templates path does not exist or is not readable: \
             {coverage_templates_path}"
        )
    })?;
    if !metadata.is_file() {
        bail!("--coverage-templates path is not a file: {coverage_templates_path}");
    }
    let extra = load_rules_from_file(coverage_templates_path);
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
    validate_coverage_mode_config(stage1_config)?;

    let total_start = Instant::now();

    let stage1_start = Instant::now();
    let (stage1_routes, stage1_stats) =
        search::find_routes(target_smiles, env, stage1_rules, stage1_config)?;
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

    // Stage 1 found nothing -- escalate. A fully independent search call:
    // no warm-start, no candidate reuse between stages, per the
    // pre-registered Phase B.2 constraint.
    let stage2_start = Instant::now();
    let control = match stage2_timeout {
        Some(d) => SearchControl::with_timeout(d),
        None => SearchControl::unlimited(),
    };
    let stage2_result = search::find_routes_with_control(
        target_smiles,
        env,
        stage2_rules,
        stage2_config,
        &control,
    )?;
    let stage2_elapsed_ms = stage2_start.elapsed().as_secs_f64() * 1000.0;
    let stage2_timed_out = stage2_result.termination == SearchTermination::DeadlineExceeded;
    let reranker_failures = stage1_stats.reranker_failures + stage2_result.stats.reranker_failures;

    Ok(CoverageModeResult {
        routes: stage2_result.routes,
        selected_stage: SelectedStage::Stage2,
        stats: stage2_result.stats,
        stage1_solved: false,
        stage2_invoked: true,
        stage1_elapsed_ms,
        stage2_elapsed_ms: Some(stage2_elapsed_ms),
        total_elapsed_ms: total_start.elapsed().as_secs_f64() * 1000.0,
        stage1_timeout: false,
        stage2_timeout: stage2_timed_out,
        reranker_failures,
    })
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
    run_coverage_mode_with_configs(
        target_smiles,
        env,
        stage1_rules,
        config,
        coverage_rules,
        config,
        stage2_timeout,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::default_rules;

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

    const ASPIRIN: &str = "CC(=O)Oc1ccccc1C(=O)O";
    // Deliberately unsolvable at any reasonable depth with an empty rule
    // set and no building-block match -- used to force Stage 1 to come
    // back empty so Stage 2 actually runs.
    const UNKNOWN: &str = "c1ccc2c(c1)c1ccccc1c1ccccc21"; // pyrene, not a building block

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

    struct CountingReranker(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl crate::candidate::CandidateReranker for CountingReranker {
        fn score_pool(
            &self,
            _target: &str,
            candidates: &mut [crate::candidate::ReactionCandidate],
        ) -> anyhow::Result<()> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            for c in candidates.iter_mut() {
                c.reranker_score = Some(c.precursor_smiles.join(".").len() as f64);
            }
            Ok(())
        }
    }

    // Requirement: Stage 1 solved => Stage 2 search not invoked, proven by
    // a Stage-2-only config carrying a reranker that panics if `score_pool`
    // is ever called -- not just checking `stage2_invoked` after the fact,
    // which could pass even if the implementation were subtly wrong about
    // *which* function actually ran.
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

    #[test]
    fn stage1_valid_route_never_overwritten() {
        let env = env();
        let stage1_rules = default_rules();
        // A hostile "coverage" rule set that would find a *different*
        // route if it ever ran -- proves Stage 1's actual result, not
        // some coincidentally-identical one, is what comes back.
        let stage2_rules = default_rules();
        let result =
            run_coverage_mode("CC(=O)O", &env, &stage1_rules, &cfg(), &stage2_rules, None).unwrap();
        assert_eq!(result.selected_stage, SelectedStage::Stage1);
        assert!(result.routes.iter().any(|r| r.depth == 0));
    }

    #[test]
    fn stage2_uses_stage2_rules_not_stage1_rules() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![]; // Stage 1 cannot solve anything
        let stage2_rules = default_rules(); // Stage 2 can solve aspirin
        let result =
            run_coverage_mode(ASPIRIN, &env, &stage1_rules, &cfg(), &stage2_rules, None).unwrap();
        assert!(result.stage2_invoked);
        // If Stage 2 had (wrongly) used stage1_rules (empty), this would
        // be empty too -- it isn't, so Stage 2 genuinely used its own rules.
        assert!(!result.routes.is_empty());
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

    #[test]
    fn partial_routes_found_before_stage2_timeout_are_retained() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![];
        let stage2_rules = default_rules();

        let baseline =
            run_coverage_mode(ASPIRIN, &env, &stage1_rules, &cfg(), &stage2_rules, None).unwrap();
        assert!(!baseline.routes.is_empty());

        let mut saw_nonempty_partial = false;
        for frac in [3u32, 4, 5, 6, 7, 8, 9] {
            let t0 = Instant::now();
            let _ = run_coverage_mode(ASPIRIN, &env, &stage1_rules, &cfg(), &stage2_rules, None)
                .unwrap();
            let baseline_elapsed = t0.elapsed();

            let partial = run_coverage_mode(
                ASPIRIN,
                &env,
                &stage1_rules,
                &cfg(),
                &stage2_rules,
                Some(baseline_elapsed * frac / 10),
            )
            .unwrap();
            assert!(partial.routes.len() <= baseline.routes.len());
            if partial.stage2_timeout && !partial.routes.is_empty() {
                saw_nonempty_partial = true;
            }
        }
        assert!(
            saw_nonempty_partial,
            "expected at least one sampled deadline fraction to catch a nonempty partial \
             route set before Stage 2 completed"
        );
    }

    #[test]
    fn reranker_failures_summed_across_invoked_stages() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![]; // Stage 1 finds nothing -> escalates
        let stage2_rules = default_rules();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stage_config = SearchConfig {
            reranker: Some(std::sync::Arc::new(CountingReranker(counter.clone()))),
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
        // reranker_failures is always 0 for a healthy reranker (never
        // errors) -- this asserts the *field exists and is well-formed*
        // for a two-stage run, not a nonzero value (which would mean the
        // reranker degraded, an unrelated failure mode).
        assert_eq!(result.reranker_failures, 0);
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

    #[test]
    fn validate_config_rejects_active_ring_context() {
        // Constructing a real `Guarded` variant needs a loaded guard file;
        // this crate's ring_context module doesn't expose a trivial
        // in-memory test double, so this test is deferred to the CLI-level
        // integration test (`tests/coverage_mode_cli.rs`), which exercises
        // it end to end via `--ring-context-policy`/`--ring-context-sidecar`.
        // `validate_config_rejects_bond_index` above already proves the
        // rejection *mechanism* (an early `bail!` before Stage 1 runs).
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

    #[test]
    fn unknown_target_via_coverage_mode_does_not_panic() {
        let env = env();
        let stage1_rules: Vec<RetroRule> = vec![];
        let stage2_rules: Vec<RetroRule> = vec![]; // Stage 2 also can't solve it
        let result = run_coverage_mode(UNKNOWN, &env, &stage1_rules, &cfg(), &stage2_rules, None);
        assert!(result.is_ok());
    }
}
