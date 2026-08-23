//! Ad hoc analysis tool for the Phase 31 corrected-baseline investigation:
//! reconstructs each target's best route (same config as the benchmark run)
//! and prints per-STEP validation status by rule, so Invalid verdicts can be
//! classified as real chemistry errors vs. validator false-negatives instead
//! of only ever seeing the route-level rollup. Not part of any measured
//! binary — reads targets from stdin, one SMILES per line.
//!
//! Optional `INSPECT_VALIDATION_TIMEOUT_SECS` env var (unset = unlimited,
//! the original/default behavior): per-target cooperative-cancellation
//! deadline (`SearchControl::with_timeout`), added for Finding #4's
//! rule-stratified sample after Issue #128's root cause (chematic's
//! canonical_smiles combinatorial cost on locally-symmetric molecules --
//! Boc/tBu/pivaloyl groups, rings, cages) confirmed that per-target latency
//! in a real USPTO-50k-shaped sample is not predictable up front: a small,
//! not-reliably-identifiable-in-advance minority of targets can run for
//! several minutes. A hard OS-level `timeout` wrapper was already found
//! unreliable against this exact codebase (didn't actually kill the process
//! at the requested mark on one prior attempt) -- this native, cooperative
//! deadline is checked at the search loop's own existing checkpoints
//! instead, same mechanism `find_routes_with_control`'s own doc comment
//! documents as a *soft* bound (worst-case overshoot is bounded by the
//! slowest single stretch of synchronous work between two checkpoints, not
//! a hard real-time guarantee) -- adequate here since the goal is bounding
//! a *batch's* total wall-clock, not any one target's exactly. Set to a
//! non-positive or non-numeric value, this fails loud rather than silently
//! falling back to unlimited -- a harness run of this scale is expensive
//! enough that a silently-ignored typo (e.g. "90s" instead of "90") is
//! worth a hard error, not a quiet, much-slower-than-intended re-run.
use renkin::chem_env::{ChemEnv, default_rules, load_rules_from_file};
use renkin::search::{SearchConfig, SearchControl, SearchTermination, find_routes_with_control};
use renkin::validation::atom_conservation::step_balanced;
use renkin::validation::validate_route_steps;
use std::io::Read;
use std::time::Duration;

const TIMEOUT_ENV_VAR: &str = "INSPECT_VALIDATION_TIMEOUT_SECS";

/// Parses [`TIMEOUT_ENV_VAR`]'s raw `std::env::var` result. `Ok(None)` means
/// "unset -- unlimited" (the original default behavior); any other outcome
/// (present-but-non-numeric, present-but-zero, or a non-UTF8 value) is a
/// hard error rather than a silent fallback to unlimited -- see this file's
/// own module doc for why. Takes the `Result` directly (rather than reading
/// `std::env::var` itself) so this parsing logic is testable without
/// mutating real process-global environment state.
fn parse_timeout_env(raw: Result<String, std::env::VarError>) -> Result<Option<u64>, String> {
    match raw {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(format!("{TIMEOUT_ENV_VAR} is set but not valid UTF-8"))
        }
        Ok(s) => {
            let secs: u64 = s.parse().map_err(|_| {
                format!("{TIMEOUT_ENV_VAR} must be a positive integer number of seconds, got {s:?}")
            })?;
            if secs == 0 {
                return Err(format!(
                    "{TIMEOUT_ENV_VAR} must be >= 1 (0 would time out every target \
                     immediately); unset the variable entirely for unlimited"
                ));
            }
            Ok(Some(secs))
        }
    }
}

/// Fresh [`SearchControl`] for one target, from an already-parsed timeout.
/// Deliberately takes `Option<u64>` (not reading the env var itself) and is
/// meant to be called once per target, inside the per-target loop --
/// [`SearchControl::with_timeout`] bakes in an *absolute* `Instant` deadline
/// at the moment it's constructed, so building it once outside the loop
/// would silently turn "N seconds per target" into "N seconds for the whole
/// batch": once cumulative wall-clock since that one construction exceeds
/// the timeout, every remaining target's very first cooperative-cancellation
/// checkpoint sees the shared deadline already passed and reports an
/// instant, meaningless timeout. Caught exactly this way on this harness's
/// first real run against Finding #4's n=300 sample (299/300 reported
/// TIMEOUT in 96s total wall-clock -- impossible if each target had
/// genuinely spent up to the configured 90s). See
/// `fresh_control_per_call_does_not_inherit_prior_elapsed_time` below for
/// the regression test.
fn build_control(timeout_secs: Option<u64>) -> SearchControl {
    timeout_secs
        .map(|secs| SearchControl::with_timeout(Duration::from_secs(secs)))
        .unwrap_or_else(SearchControl::unlimited)
}

/// Status line for a target whose search found no route at all (distinct
/// from the per-step `Invalid`/`Valid` validation status, which only
/// applies once a route exists).
fn no_route_status(termination: SearchTermination) -> &'static str {
    match termination {
        SearchTermination::Completed => "UNSOLVED",
        SearchTermination::DeadlineExceeded => "TIMEOUT",
    }
}

/// `termination=` field value for the `ROUTE` line -- printed regardless of
/// whether a route was found, since `SearchTermination::DeadlineExceeded`'s
/// own contract is "whatever valid routes were found before the deadline
/// are still returned, never discarded": a route found under
/// `DeadlineExceeded` is still a real, complete route, not a partial one,
/// and the harness output shouldn't silently drop that provenance just
/// because a route happened to exist.
fn termination_label(termination: SearchTermination) -> &'static str {
    match termination {
        SearchTermination::Completed => "completed",
        SearchTermination::DeadlineExceeded => "deadline_exceeded",
    }
}

fn format_route_line(
    smiles: &str,
    route_status: renkin::validation::RouteValidationStatus,
    depth: u32,
    termination: SearchTermination,
) -> String {
    format!(
        "{smiles}\tROUTE\t{route_status:?}\tdepth={depth}\ttermination={}",
        termination_label(termination)
    )
}

fn main() {
    let env = ChemEnv::load("data/building_blocks.smi").expect("load building blocks");
    let mut rules = default_rules();
    rules.extend(load_rules_from_file("data/templates_extracted_5000.smi"));

    let config = SearchConfig {
        max_depth: 5,
        max_routes: 1,
        beam_width: 100,
        ..Default::default()
    };

    let timeout_secs =
        parse_timeout_env(std::env::var(TIMEOUT_ENV_VAR)).unwrap_or_else(|e| panic!("{e}"));

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();

    for line in input.lines() {
        let smiles = line.trim();
        if smiles.is_empty() || smiles.starts_with('#') {
            continue;
        }
        let control = build_control(timeout_secs);
        let Ok(result) = find_routes_with_control(smiles, &env, &rules, &config, &control) else {
            println!("{smiles}\tERROR");
            continue;
        };
        let Some(route) = result.routes.first() else {
            println!("{smiles}\t{}", no_route_status(result.termination));
            continue;
        };
        let (statuses, route_status) = validate_route_steps(&route.steps, &rules);
        println!(
            "{}",
            format_route_line(smiles, route_status, route.depth, result.termination)
        );
        for (step, status) in route.steps.iter().zip(statuses.iter()) {
            let balanced = step_balanced(&step.target, &step.precursors);
            println!(
                "{smiles}\tSTEP\t{status:?}\tbalanced={balanced}\trule={}\ttarget={}\tprecursors={}",
                step.rule,
                step.target,
                step.precursors.join(".")
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_timeout_env ────────────────────────────────────────────────
    #[test]
    fn timeout_env_unset_is_unlimited() {
        assert_eq!(
            parse_timeout_env(Err(std::env::VarError::NotPresent)),
            Ok(None)
        );
    }

    #[test]
    fn timeout_env_valid_integer() {
        assert_eq!(parse_timeout_env(Ok("90".to_string())), Ok(Some(90)));
    }

    #[test]
    fn timeout_env_invalid_string_is_hard_error() {
        assert!(parse_timeout_env(Ok("90s".to_string())).is_err());
    }

    #[test]
    fn timeout_env_zero_is_hard_error() {
        // 0 would time out every target immediately -- almost certainly a
        // mistake (e.g. meant to be unset), not a real request.
        assert!(parse_timeout_env(Ok("0".to_string())).is_err());
    }

    #[test]
    fn timeout_env_empty_string_is_hard_error() {
        assert!(parse_timeout_env(Ok(String::new())).is_err());
    }

    // ── build_control / fresh-per-call regression ──────────────────────
    #[test]
    fn fresh_control_per_call_does_not_inherit_prior_elapsed_time() {
        // Regression for the harness's first real run (see build_control's
        // own doc comment). Simulates wall-clock already having elapsed
        // (as earlier targets in a real batch would consume) BEFORE
        // building this call's control -- a control built once outside the
        // loop would have had its deadline computed before that elapsed
        // time, so by now it would already be expired even for a 1s
        // timeout. A freshly-built control must not be.
        std::thread::sleep(Duration::from_millis(200));
        let control = build_control(Some(1));
        let env = ChemEnv::in_memory(&["C"]);
        let rules = default_rules();
        let config = SearchConfig {
            max_depth: 1,
            max_routes: 1,
            beam_width: 0,
            ..Default::default()
        };
        // "C" (methane) is itself a stock hit -- resolves essentially
        // instantly, so this only fails if the control was already expired
        // at construction time, not from genuinely running out the clock.
        let result = find_routes_with_control("C", &env, &rules, &config, &control)
            .expect("trivial in-stock target must not error");
        assert_eq!(
            result.termination,
            SearchTermination::Completed,
            "a freshly-built 1s control must not already be expired \
             immediately after construction, even after 200ms of prior \
             (simulated-batch) elapsed time"
        );
    }

    // ── output formatting ───────────────────────────────────────────────
    #[test]
    fn no_route_status_maps_completed_to_unsolved() {
        assert_eq!(no_route_status(SearchTermination::Completed), "UNSOLVED");
    }

    #[test]
    fn no_route_status_maps_deadline_exceeded_to_timeout() {
        assert_eq!(
            no_route_status(SearchTermination::DeadlineExceeded),
            "TIMEOUT"
        );
    }

    #[test]
    fn route_line_carries_termination_field_when_completed() {
        let line = format_route_line(
            "CCO",
            renkin::validation::RouteValidationStatus::Validated,
            2,
            SearchTermination::Completed,
        );
        assert!(
            line.contains("termination=completed"),
            "route line must carry a termination field even when a route \
             was found (not only on the no-route TIMEOUT/UNSOLVED path): {line}"
        );
    }

    #[test]
    fn route_line_carries_termination_field_when_deadline_exceeded() {
        // A route found before the deadline is still real and complete
        // (SearchTermination::DeadlineExceeded's own contract never
        // discards routes already found) -- the output must say so, not
        // silently look identical to a normally-completed search.
        let line = format_route_line(
            "CCO",
            renkin::validation::RouteValidationStatus::Validated,
            2,
            SearchTermination::DeadlineExceeded,
        );
        assert!(
            line.contains("termination=deadline_exceeded"),
            "route line must reflect DeadlineExceeded even when a route \
             was still found before the deadline: {line}"
        );
    }
}
