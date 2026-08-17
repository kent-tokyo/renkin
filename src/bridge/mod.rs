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
//! Not yet implemented here (RENKIN Bridge PR4): the `renkin audit-route`
//! CLI subcommand and the AiZynthFinder JSON adapter. `RouteSource` already
//! has an `AiZynthFinder` variant so `AuditReport::source` has a home for
//! it, but nothing in this module constructs one yet.

pub mod audit;
pub mod route_graph;

pub use audit::{AuditFinding, AuditFindingCode, AuditReport, AuditSeverity, AuditStatus, audit};
pub use route_graph::{ParseOutcome, RouteDocument, RouteNode, RouteSource, RouteStep};
