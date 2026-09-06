//! Opt-in staged recovery for completed, unsuccessful route searches.
//!
//! The normal search always runs first and remains authoritative whenever it
//! succeeds. Later stages use fresh frontiers and only run after explicit
//! exhaustion evidence; they never weaken completed-route integrity checks or
//! stock identity. See Issue #239.

use crate::chem_env::{ChemEnv, PreparedRuleSet, RetroRule};
use crate::coverage_mode::validate_coverage_mode_config;
use crate::search::{
    self, BeamDiversityPolicy, ElementAccountingGatePolicy, SearchConfig, SearchControl,
    SearchRunResult, SearchTermination,
};
use anyhow::{Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStage {
    Baseline,
    ElementAccounting,
    BeamDiversity,
    Depth,
    Coverage,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryAttempt {
    pub stage: RecoveryStage,
    pub trigger: &'static str,
    pub max_depth: u32,
    pub beam_width: usize,
    pub rule_count: usize,
    pub rules_sha256: String,
    pub element_accounting_policy: &'static str,
    pub beam_diversity_policy: &'static str,
    pub beam_diversity_slots: usize,
    /// One-based index for coverage-ladder attempts; absent for other stages.
    /// The rule hash remains the authoritative tier identity.
    pub coverage_tier: Option<usize>,
    pub termination: SearchTermination,
    pub routes_found: usize,
    pub nodes_expanded: u64,
    pub beam_limit_hit: bool,
    pub max_depth_reached: bool,
    pub routes_rejected: u64,
    pub unaccounted_target_element: u64,
    pub elapsed_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryAudit {
    pub selected_stage: RecoveryStage,
    pub recovered_route: bool,
    pub attempts: Vec<RecoveryAttempt>,
    pub total_elapsed_ms: f64,
}

#[derive(Debug)]
pub struct RecoveryModeResult {
    pub selected: SearchRunResult,
    pub audit: RecoveryAudit,
}

#[derive(Debug, Clone)]
pub struct RecoveryOptions {
    /// Must be greater than the baseline depth. The CLI defaults this to
    /// baseline depth + 1.
    pub recovery_depth: u32,
    /// Zero skips the diversity stage; a positive value reserves that many
    /// slots in the otherwise unchanged beam.
    pub beam_diversity_slots: usize,
    /// Optional final-stage rule-set ladder, ordered from narrower to broader.
    /// Every tier starts from a fresh frontier. This remains caller supplied
    /// so no research-only template asset becomes a package/default dependency.
    pub coverage_rule_tiers: Vec<Vec<RetroRule>>,
    pub coverage_timeout: Option<Duration>,
    pub coverage_beam_width: Option<usize>,
}

fn rules_sha256(rules: &[RetroRule]) -> String {
    let mut hasher = Sha256::new();
    for rule in rules {
        hasher.update(rule.template_id.as_bytes());
        hasher.update([0]);
        hasher.update(rule.smirks.as_bytes());
        hasher.update([0]);
        hasher.update(rule.weight.to_bits().to_le_bytes());
        hasher.update(rule.required_elements.to_le_bytes());
        hasher.update([0xff]);
    }
    format!("sha256:{}", crate::sha256_hex(hasher.finalize()))
}

fn element_policy_name(policy: ElementAccountingGatePolicy) -> &'static str {
    match policy {
        ElementAccountingGatePolicy::Off => "off",
        ElementAccountingGatePolicy::DiagnosticsOnly => "diagnostics_only",
        ElementAccountingGatePolicy::Gated => "gated",
    }
}

fn diversity_policy_name(policy: BeamDiversityPolicy) -> &'static str {
    match policy {
        BeamDiversityPolicy::Off => "off",
        BeamDiversityPolicy::DiagnosticsOnly => "diagnostics_only",
        BeamDiversityPolicy::Active => "active",
    }
}

fn recovery_attempt(
    stage: RecoveryStage,
    trigger: &'static str,
    config: &SearchConfig,
    rules: &[RetroRule],
    result: &SearchRunResult,
    elapsed_ms: f64,
    coverage_tier: Option<usize>,
) -> RecoveryAttempt {
    RecoveryAttempt {
        stage,
        trigger,
        max_depth: config.max_depth,
        beam_width: config.beam_width,
        rule_count: rules.len(),
        rules_sha256: rules_sha256(rules),
        element_accounting_policy: element_policy_name(config.element_accounting_policy),
        beam_diversity_policy: diversity_policy_name(config.beam_diversity_policy),
        beam_diversity_slots: config.beam_diversity_slots,
        coverage_tier,
        termination: result.termination,
        routes_found: result.routes.len(),
        nodes_expanded: result.stats.nodes_expanded,
        beam_limit_hit: result.stats.beam_limit_hit,
        max_depth_reached: result.stats.max_depth_reached,
        routes_rejected: result.stats.route_integrity.routes_rejected,
        unaccounted_target_element: result.stats.route_integrity.unaccounted_target_element,
        elapsed_ms,
    }
}

fn finish(
    selected: SearchRunResult,
    selected_stage: RecoveryStage,
    attempts: Vec<RecoveryAttempt>,
    total_start: Instant,
) -> RecoveryModeResult {
    RecoveryModeResult {
        audit: RecoveryAudit {
            selected_stage,
            recovered_route: selected_stage != RecoveryStage::Baseline
                && !selected.routes.is_empty(),
            attempts,
            total_elapsed_ms: total_start.elapsed().as_secs_f64() * 1000.0,
        },
        selected,
    }
}

fn run_stage(
    target_smiles: &str,
    env: &ChemEnv,
    rules: &[RetroRule],
    config: &SearchConfig,
    control: &SearchControl,
    prepared_rules: &PreparedRuleSet,
) -> Result<(SearchRunResult, f64)> {
    let start = Instant::now();
    let result = search::find_routes_with_control_prepared(
        target_smiles,
        env,
        rules,
        config,
        control,
        prepared_rules,
        None,
    )?;
    Ok((result, start.elapsed().as_secs_f64() * 1000.0))
}

/// Run a bounded, auditable recovery cascade.
///
/// Each stage starts from a fresh frontier. A success short-circuits every
/// later stage. A coverage-attempt deadline stops escalation immediately;
/// callers may additionally enforce a whole-process deadline. The baseline
/// policies are forced to `Off` so this mode has one stable meaning rather
/// than inheriting a partially active retry from its caller.
pub fn run_recovery_mode(
    target_smiles: &str,
    env: &ChemEnv,
    baseline_rules: &[RetroRule],
    config: &SearchConfig,
    options: &RecoveryOptions,
) -> Result<RecoveryModeResult> {
    if options.recovery_depth <= config.max_depth {
        bail!(
            "recovery_depth ({}) must be greater than baseline max_depth ({})",
            options.recovery_depth,
            config.max_depth
        );
    }
    if options.beam_diversity_slots > 0 && config.beam_width == 0 {
        bail!("beam diversity recovery requires a bounded baseline beam_width");
    }
    if let Some(width) = options.coverage_beam_width
        && width == 0
    {
        // An unlimited coverage beam is valid in coverage mode, but is an
        // accidental resource explosion in this bounded recovery contract.
        bail!("coverage_beam_width must be positive in recovery mode");
    }
    if options.coverage_rule_tiers.is_empty()
        && (options.coverage_timeout.is_some() || options.coverage_beam_width.is_some())
    {
        bail!("coverage timeout/beam options require at least one coverage rule tier");
    }
    if !options.coverage_rule_tiers.is_empty() {
        validate_coverage_mode_config(config)?;
    }

    let mut all_rule_sets = Vec::with_capacity(1 + options.coverage_rule_tiers.len());
    all_rule_sets.push(baseline_rules);
    all_rule_sets.extend(options.coverage_rule_tiers.iter().map(Vec::as_slice));
    let prepared_rules = PreparedRuleSet::from_rule_sets(&all_rule_sets);
    let total_start = Instant::now();
    let mut attempts = Vec::new();

    let mut baseline_config = config.clone();
    baseline_config.element_accounting_policy = ElementAccountingGatePolicy::Off;
    baseline_config.beam_diversity_policy = BeamDiversityPolicy::Off;
    baseline_config.beam_diversity_slots = 0;
    let (baseline, elapsed_ms) = run_stage(
        target_smiles,
        env,
        baseline_rules,
        &baseline_config,
        &SearchControl::unlimited(),
        &prepared_rules,
    )?;
    attempts.push(recovery_attempt(
        RecoveryStage::Baseline,
        "always",
        &baseline_config,
        baseline_rules,
        &baseline,
        elapsed_ms,
        None,
    ));
    if !baseline.routes.is_empty() || baseline.termination != SearchTermination::Completed {
        return Ok(finish(
            baseline,
            RecoveryStage::Baseline,
            attempts,
            total_start,
        ));
    }

    let integrity_triggered = baseline.stats.route_integrity.unaccounted_target_element > 0;
    let beam_triggered = baseline.stats.beam_limit_hit;
    let depth_triggered = baseline.stats.max_depth_reached;

    if integrity_triggered {
        let mut gated_config = baseline_config.clone();
        gated_config.element_accounting_policy = ElementAccountingGatePolicy::Gated;
        let (gated, elapsed_ms) = run_stage(
            target_smiles,
            env,
            baseline_rules,
            &gated_config,
            &SearchControl::unlimited(),
            &prepared_rules,
        )?;
        attempts.push(recovery_attempt(
            RecoveryStage::ElementAccounting,
            "baseline_unaccounted_route_rejection",
            &gated_config,
            baseline_rules,
            &gated,
            elapsed_ms,
            None,
        ));
        if !gated.routes.is_empty() || gated.termination != SearchTermination::Completed {
            return Ok(finish(
                gated,
                RecoveryStage::ElementAccounting,
                attempts,
                total_start,
            ));
        }
    }

    if beam_triggered && options.beam_diversity_slots > 0 {
        let mut diversity_config = baseline_config.clone();
        diversity_config.beam_diversity_policy = BeamDiversityPolicy::Active;
        diversity_config.beam_diversity_slots = options.beam_diversity_slots;
        let (diversity, elapsed_ms) = run_stage(
            target_smiles,
            env,
            baseline_rules,
            &diversity_config,
            &SearchControl::unlimited(),
            &prepared_rules,
        )?;
        attempts.push(recovery_attempt(
            RecoveryStage::BeamDiversity,
            "baseline_beam_exhaustion",
            &diversity_config,
            baseline_rules,
            &diversity,
            elapsed_ms,
            None,
        ));
        if !diversity.routes.is_empty() || diversity.termination != SearchTermination::Completed {
            return Ok(finish(
                diversity,
                RecoveryStage::BeamDiversity,
                attempts,
                total_start,
            ));
        }
    }

    if depth_triggered {
        let mut depth_config = baseline_config.clone();
        depth_config.max_depth = options.recovery_depth;
        let (depth, elapsed_ms) = run_stage(
            target_smiles,
            env,
            baseline_rules,
            &depth_config,
            &SearchControl::unlimited(),
            &prepared_rules,
        )?;
        attempts.push(recovery_attempt(
            RecoveryStage::Depth,
            "baseline_depth_exhaustion",
            &depth_config,
            baseline_rules,
            &depth,
            elapsed_ms,
            None,
        ));
        if !depth.routes.is_empty() || depth.termination != SearchTermination::Completed {
            return Ok(finish(depth, RecoveryStage::Depth, attempts, total_start));
        }
    }

    for (tier_index, coverage_rules) in options.coverage_rule_tiers.iter().enumerate() {
        let mut coverage_config = baseline_config.clone();
        if let Some(width) = options.coverage_beam_width {
            coverage_config.beam_width = width;
        }
        let control = options
            .coverage_timeout
            .map(SearchControl::with_timeout)
            .unwrap_or_else(SearchControl::unlimited);
        let (coverage, elapsed_ms) = run_stage(
            target_smiles,
            env,
            coverage_rules,
            &coverage_config,
            &control,
            &prepared_rules,
        )?;
        attempts.push(recovery_attempt(
            RecoveryStage::Coverage,
            "prior_stages_completed_without_route",
            &coverage_config,
            coverage_rules,
            &coverage,
            elapsed_ms,
            Some(tier_index + 1),
        ));
        if !coverage.routes.is_empty() || coverage.termination != SearchTermination::Completed {
            return Ok(finish(
                coverage,
                RecoveryStage::Coverage,
                attempts,
                total_start,
            ));
        }

        if coverage.stats.beam_limit_hit && options.beam_diversity_slots > 0 {
            let mut diversity_config = coverage_config.clone();
            diversity_config.beam_diversity_policy = BeamDiversityPolicy::Active;
            diversity_config.beam_diversity_slots = options.beam_diversity_slots;
            let (diversity, elapsed_ms) = run_stage(
                target_smiles,
                env,
                coverage_rules,
                &diversity_config,
                &control,
                &prepared_rules,
            )?;
            attempts.push(recovery_attempt(
                RecoveryStage::Coverage,
                "coverage_tier_beam_exhaustion",
                &diversity_config,
                coverage_rules,
                &diversity,
                elapsed_ms,
                Some(tier_index + 1),
            ));
            if !diversity.routes.is_empty() || diversity.termination != SearchTermination::Completed
            {
                return Ok(finish(
                    diversity,
                    RecoveryStage::Coverage,
                    attempts,
                    total_start,
                ));
            }

            if coverage.stats.max_depth_reached || diversity.stats.max_depth_reached {
                let mut deep_diversity_config = diversity_config.clone();
                deep_diversity_config.max_depth = options.recovery_depth;
                let (deep_diversity, elapsed_ms) = run_stage(
                    target_smiles,
                    env,
                    coverage_rules,
                    &deep_diversity_config,
                    &control,
                    &prepared_rules,
                )?;
                attempts.push(recovery_attempt(
                    RecoveryStage::Coverage,
                    "coverage_tier_depth_and_beam_exhaustion",
                    &deep_diversity_config,
                    coverage_rules,
                    &deep_diversity,
                    elapsed_ms,
                    Some(tier_index + 1),
                ));
                if !deep_diversity.routes.is_empty()
                    || deep_diversity.termination != SearchTermination::Completed
                    || tier_index + 1 == options.coverage_rule_tiers.len()
                {
                    return Ok(finish(
                        deep_diversity,
                        RecoveryStage::Coverage,
                        attempts,
                        total_start,
                    ));
                }
            } else if tier_index + 1 == options.coverage_rule_tiers.len() {
                return Ok(finish(
                    diversity,
                    RecoveryStage::Coverage,
                    attempts,
                    total_start,
                ));
            }
        } else if tier_index + 1 == options.coverage_rule_tiers.len() {
            return Ok(finish(
                coverage,
                RecoveryStage::Coverage,
                attempts,
                total_start,
            ));
        }
    }

    Ok(finish(
        baseline,
        RecoveryStage::Baseline,
        attempts,
        total_start,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::default_rules;

    fn config(depth: u32, beam_width: usize) -> SearchConfig {
        SearchConfig {
            max_depth: depth,
            max_routes: 1,
            beam_width,
            ..Default::default()
        }
    }

    #[test]
    fn baseline_success_short_circuits_every_recovery_stage() {
        let env = ChemEnv::in_memory(&["CC(=O)O"]);
        let rules = default_rules();
        let result = run_recovery_mode(
            "CC(=O)O",
            &env,
            &rules,
            &config(2, 100),
            &RecoveryOptions {
                recovery_depth: 3,
                beam_diversity_slots: 20,
                coverage_rule_tiers: vec![rules.clone()],
                coverage_timeout: Some(Duration::from_secs(1)),
                coverage_beam_width: Some(100),
            },
        )
        .unwrap();
        assert_eq!(result.audit.selected_stage, RecoveryStage::Baseline);
        assert_eq!(result.audit.attempts.len(), 1);
        assert!(!result.audit.recovered_route);
    }

    #[test]
    fn rejects_non_increasing_recovery_depth_before_search() {
        let env = ChemEnv::in_memory(&[]);
        let rules = default_rules();
        let err = run_recovery_mode(
            "CC",
            &env,
            &rules,
            &config(5, 100),
            &RecoveryOptions {
                recovery_depth: 5,
                beam_diversity_slots: 20,
                coverage_rule_tiers: Vec::new(),
                coverage_timeout: None,
                coverage_beam_width: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("must be greater"));
    }

    #[test]
    fn coverage_timeout_stops_before_the_next_tier() {
        let env = ChemEnv::in_memory(&[]);
        let rules = default_rules();
        let result = run_recovery_mode(
            "c1ccc2c(c1)c1ccccc1c1ccccc21",
            &env,
            &rules,
            &config(1, 100),
            &RecoveryOptions {
                recovery_depth: 2,
                beam_diversity_slots: 20,
                coverage_rule_tiers: vec![rules.clone(), rules.clone()],
                coverage_timeout: Some(Duration::ZERO),
                coverage_beam_width: Some(100),
            },
        )
        .unwrap();
        let coverage_attempts: Vec<_> = result
            .audit
            .attempts
            .iter()
            .filter(|attempt| attempt.stage == RecoveryStage::Coverage)
            .collect();
        assert_eq!(coverage_attempts.len(), 1);
        assert_eq!(coverage_attempts[0].coverage_tier, Some(1));
        assert_eq!(
            coverage_attempts[0].termination,
            SearchTermination::DeadlineExceeded
        );
    }

    #[test]
    fn rules_fingerprint_changes_with_rule_set() {
        let mut rules = default_rules();
        let initial = rules_sha256(&rules);
        rules.pop();
        assert_ne!(initial, rules_sha256(&rules));
    }
}
