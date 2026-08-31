//! Shared validation for route-level planning constraints.

/// Validate numeric route filters before a search starts.
///
/// Keeping this contract in the library makes the CLI, Python, and MCP
/// surfaces reject the same invalid policy instead of silently returning an
/// empty route set. `max_route_cost` uses the non-negative cost domain;
/// confidence and success-probability thresholds are normalized scores.
pub fn validate_route_thresholds(
    max_route_cost: Option<f64>,
    min_confidence: Option<f64>,
    min_success_probability: Option<f64>,
) -> Result<(), String> {
    if let Some(value) = max_route_cost
        && (!value.is_finite() || value < 0.0)
    {
        return Err(format!(
            "max_route_cost must be finite and non-negative (got {value})"
        ));
    }
    for (name, value) in [
        ("min_confidence", min_confidence),
        ("min_success_probability", min_success_probability),
    ] {
        if let Some(value) = value
            && (!value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(format!("{name} must be finite and in [0,1] (got {value})"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_route_thresholds;

    #[test]
    fn accepts_boundary_values() {
        assert!(validate_route_thresholds(Some(0.0), Some(0.0), Some(1.0)).is_ok());
    }

    #[test]
    fn rejects_negative_cost() {
        let error = validate_route_thresholds(Some(-0.1), None, None).unwrap_err();
        assert!(error.contains("max_route_cost"));
    }

    #[test]
    fn rejects_non_finite_and_out_of_range_scores() {
        assert!(validate_route_thresholds(None, Some(f64::NAN), None).is_err());
        assert!(validate_route_thresholds(None, Some(1.01), None).is_err());
        assert!(validate_route_thresholds(None, None, Some(-0.01)).is_err());
    }
}
