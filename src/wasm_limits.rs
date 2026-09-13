//! Input limits for the public WASM search surface.
//!
//! Native search already validates its core budget in `search.rs`.  WASM
//! callers can invoke exported functions directly, however, so the browser
//! boundary also needs to reject oversized filter strings and policy-slot
//! values before allocating the chemistry environment or entering search.

use anyhow::{Result, bail};

/// Browser-facing limits are intentionally tighter than the generous native
/// API limits: a single tab must not be able to reserve an unbounded amount of
/// memory or CPU through a direct WASM call.
pub const MAX_WASM_SEARCH_DEPTH: u32 = 16;
pub const MAX_WASM_ROUTES: usize = 100;
pub const MAX_WASM_BEAM_WIDTH: usize = 10_000;
pub const MAX_WASM_CANDIDATE_TRACE: usize = 50_000;

/// Validate the common argument set used by the versioned WASM search exports.
/// The limits mirror the shared search limits except for filter text, which is
/// not represented in `SearchConfig` and therefore cannot be checked there.
#[allow(clippy::too_many_arguments)]
pub fn validate_search_inputs(
    target: &str,
    depth: u32,
    max_routes: usize,
    beam_width: usize,
    avoid_elements: &str,
    require_elements: &str,
    diversity_slots: usize,
    candidate_trace_limit: Option<usize>,
) -> Result<()> {
    if target.trim().is_empty() {
        bail!("invalid_input: target SMILES must not be empty");
    }
    if target.len() > crate::search::MAX_TARGET_SMILES_BYTES {
        bail!(
            "resource_exhausted: target SMILES exceeds {} bytes",
            crate::search::MAX_TARGET_SMILES_BYTES
        );
    }
    if depth > MAX_WASM_SEARCH_DEPTH {
        bail!(
            "resource_exhausted: WASM max_depth exceeds {}",
            MAX_WASM_SEARCH_DEPTH
        );
    }
    if max_routes > MAX_WASM_ROUTES {
        bail!(
            "resource_exhausted: WASM max_routes exceeds {}",
            MAX_WASM_ROUTES
        );
    }
    if beam_width > MAX_WASM_BEAM_WIDTH {
        bail!(
            "resource_exhausted: WASM beam_width exceeds {}",
            MAX_WASM_BEAM_WIDTH
        );
    }
    if diversity_slots > MAX_WASM_BEAM_WIDTH {
        bail!(
            "resource_exhausted: WASM beam_diversity_slots exceeds {}",
            MAX_WASM_BEAM_WIDTH
        );
    }
    if candidate_trace_limit.is_some_and(|limit| limit > MAX_WASM_CANDIDATE_TRACE) {
        bail!(
            "resource_exhausted: WASM candidate_trace_limit exceeds {}",
            MAX_WASM_CANDIDATE_TRACE
        );
    }
    for (name, value) in [
        ("avoid_elements", avoid_elements),
        ("require_elements", require_elements),
    ] {
        crate::chem_env::validate_element_symbols(name, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Result<()> {
        validate_search_inputs("CCO", 5, 5, 100, "Br,I", "B", 0, None)
    }

    #[test]
    fn accepts_normal_playground_request() {
        assert!(valid().is_ok());
    }

    #[test]
    fn rejects_oversized_element_filter_before_search() {
        let filter = "C".repeat(crate::chem_env::MAX_ELEMENT_FILTER_BYTES + 1);
        let error = validate_search_inputs("CCO", 5, 5, 100, &filter, "", 0, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("resource_exhausted"));
        assert!(error.contains("avoid_elements"));
    }

    #[test]
    fn rejects_unknown_or_empty_element_filter_tokens() {
        for filter in ["Xx", "C,,N"] {
            let error = validate_search_inputs("CCO", 5, 5, 100, filter, "", 0, None)
                .unwrap_err()
                .to_string();
            assert!(error.contains("invalid_input"));
            assert!(error.contains("avoid_elements"));
        }
    }

    #[test]
    fn rejects_empty_target_at_the_boundary() {
        let error = validate_search_inputs("  ", 5, 5, 100, "", "", 0, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("target SMILES must not be empty"));
    }

    #[test]
    fn mirrors_shared_search_budget_limits() {
        let error = validate_search_inputs(
            "CCO",
            crate::search::MAX_SEARCH_DEPTH + 1,
            5,
            100,
            "",
            "",
            0,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("max_depth"));
    }

    #[test]
    fn rejects_native_sized_budget_on_browser_surface() {
        let error =
            validate_search_inputs("CCO", MAX_WASM_SEARCH_DEPTH + 1, 5, 100, "", "", 0, None)
                .unwrap_err()
                .to_string();
        assert!(error.contains("WASM max_depth"));
    }
}
