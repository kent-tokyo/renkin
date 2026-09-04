//! Synthesizability Kernel v0 (`docs/design/synthesizability-kernel-v0.md`):
//! takes already-produced search routes plus stock/evidence/validation
//! context and returns a decisive, auditable, machine-readable assessment
//! of how well-supported a route is -- never a claim that a target
//! *cannot* be synthesized, and never an uncalibrated score.
//!
//! This module is a stable single entry point regardless of how many files
//! live under it, per the design doc's per-agent file ownership (§7):
//! `schema.rs` (types), `signals.rs`/`element_accounting.rs` (pure signal
//! extraction), `assessment.rs`/`provenance.rs` (policy + hashing).

mod assessment;
mod diagnostics;
mod element_accounting;
mod provenance;
mod schema;
mod signals;

pub use assessment::{AssessmentContext, assess_routes};
pub use diagnostics::{
    CoverageStatus, FeasibilityDiagnosticDisposition, FeasibilityDiagnosticsError, LimitingStep,
    LimitingStepCode, MissingInformation, MissingInformationCode,
    ROUTE_FEASIBILITY_DIAGNOSTICS_SCHEMA_VERSION, RouteCompletionStatus,
    RouteFeasibilityDiagnostics, StructuralDiagnosticStatus, diagnose_route_feasibility,
};
pub(crate) use element_accounting::{
    compute_element_accounting, heavy_atom_counts, step_element_accounting,
};
pub use schema::*;
