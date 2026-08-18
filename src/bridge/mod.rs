//! RENKIN Bridge P0: a tool-neutral route audit model.
//!
//! Promotes `scripts/compare_route_graph.py` (tool-agnostic route DAG
//! normalization) and `scripts/compare_validation.py` (post-hoc route
//! validation) -- the Python research harness built for the Issue #66
//! open-source planner comparison -- into first-class Rust product types,
//! so the same audit RENKIN already applies to its own completed routes
//! (see `search::route_integrity_defects`, RENKIN Bridge PR1) can also be
//! run against a competitor's exported route.
//!
//! Scope note: this module ports the *finding taxonomy* (the defect/warning
//! codes each Python module emits) and its *normalization/validation
//! logic*, not `scripts/compare_schema.py`'s `PlannerComparisonRow` --
//! that's a flattened research-measurement row (timing, RSS, sample rank)
//! for a specific historical comparison study, not an audit report shape.
//! The Python modules remain an independent fixture-parity oracle: this
//! module's tests assert the same defect/warning code *sets* on the same
//! fixture inputs as `scripts/tests/test_compare_route_graph.py`, not
//! byte-identical serialized output (RDKit vs. chematic canonicalization
//! differ, so `normalized_route_sha256` values are never compared
//! cross-language -- only within-Rust stability/uniqueness properties are
//! asserted, mirroring what `test_compare_route_graph.py` itself checks
//! within Python).
//!
//! RENKIN Bridge PR4 added declared-reaction-replay forward validation
//! (`forward` submodule): per-step `pass`/`fail`/`not_evaluable` verdicts
//! folded into the route-level `AuditStatus` (now `Pass`/`Fail`/`Partial`,
//! `Partial` covering any not-evaluable check with no outright failure).
//!
//! RENKIN Bridge PR5 added the `renkin audit-route <PATH>` CLI subcommand
//! (`src/main.rs::run_audit_route`), RENKIN-native JSON input only.
//!
//! RENKIN Bridge PR6 adds a real AiZynthFinder JSON adapter
//! (`aizynthfinder` submodule, `--format aizynthfinder`/`auto` on the CLI):
//! confirmed against real `aizynthcli 4.4.1` output (see
//! `tests/fixtures/aizynthfinder/v4.4.1/PROVENANCE.md`), not guessed. Still
//! out of scope, not just deferred: HTML output, DOI/condition/yield
//! reporting, and alternative-disconnection suggestions.

pub mod aizynthfinder;
pub mod audit;
pub mod forward;
pub mod route_graph;

pub use aizynthfinder::{AzfMetadata, AzfNode, normalize_aizynthfinder_route};
pub use audit::{
    AuditFinding, AuditFindingCode, AuditReport, AuditSeverity, AuditStatus, AuditedStep,
    CheckStatus, StockNotEvaluableReason, StockValidationResult, audit,
};
pub use forward::{ForwardNotEvaluableReason, ForwardValidationResult, validate_step_forward};
pub use route_graph::{
    ParseOutcome, ReactionEvidence, RouteDocument, RouteNode, RouteSource, RouteStep,
    normalize_renkin_route,
};
