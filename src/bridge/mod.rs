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
//!
//! v0.28.0 "Audit Playground" extracted the multi-route report pipeline
//! previously inlined in `src/main.rs::run_audit_route` into the
//! `audit_route` submodule, so both the CLI and the playground's WASM
//! `audit_route` export (`src/wasm.rs`) call the identical
//! [`build_audit_route_report`] rather than maintaining two copies of
//! format-detection/parsing/manifest logic.
//!
//! v0.29.0 Audit Policy Profiles (PR1: core policy model) added
//! [`AuditPolicy`] and `_with_policy` variants of every function that
//! derives [`AuditStatus`] (`audit_with_policy`, `audit_document_with_policy`,
//! `build_audit_route_report_with_policy`) -- the pre-existing functions
//! (`audit`, `audit_document`, `build_audit_route_report`) are kept as
//! `AuditPolicy::Standard` wrappers, unchanged in signature, since all
//! three were already published (crates.io/PyPI/npm) before this policy
//! parameter existed. Policy only ever changes how the route-level
//! `status` is derived from findings/checks already collected -- never
//! which findings are detected or reported. See
//! `docs/design/audit-policy-profiles-v0.md` for the full design.
//!
//! Phase 1 PR2 (SynPlanner-surpass roadmap) adds a real SynPlanner
//! `write_routes_json` adapter (`synplanner` submodule, `--format
//! synplanner`/`auto` on the CLI): confirmed against real SynPlanner 1.6.0
//! output twice (see `docs/design/synplanner-adapter-v1.md` and
//! `tests/fixtures/synplanner/v1.6.0/`) -- once via hand-constructed
//! `chython` reactions run through the real exporter, once via a real
//! CPU-only MCTS-searched planning run through the real `synplan planning`
//! CLI end to end. Still out of scope, not just deferred: the separate
//! `--export_routes` "public contract" wrapper format, RouteCGR/clustering/
//! quality-scoring output, and any model-training/planning integration.

pub mod aizynthfinder;
pub mod audit;
pub mod audit_route;
pub mod forward;
pub mod interchange;
pub mod private_stock;
pub mod review;
pub mod route_graph;
pub mod synplanner;
pub mod syntheseus;

pub use aizynthfinder::{AzfMetadata, AzfNode, normalize_aizynthfinder_route};
pub use audit::{
    AuditFinding, AuditFindingCode, AuditPolicy, AuditReport, AuditSeverity, AuditStatus,
    AuditedStep, CheckStatus, StockNotEvaluableReason, StockValidationResult, audit,
    audit_document, audit_document_with_policy, audit_with_policy,
};
pub use audit_route::{
    AuditManifest, AuditRouteReport, AuditRouteSummary, build_audit_route_report,
    build_audit_route_report_with_options, build_audit_route_report_with_policy, parse_stock_text,
    validate_audit_json_structure, validate_audit_text_inputs,
};
pub use forward::{
    EvidenceBasis, ForwardNotEvaluableReason, ForwardValidationResult, validate_step_forward,
};
pub use interchange::{
    InterchangeStep, ROUTE_INTERCHANGE_SCHEMA_VERSION, ReactionProvenance, RouteInterchange,
    StockProvenance, from_audit_report,
};
pub use private_stock::{
    PRIVATE_STOCK_POLICY_SCHEMA_VERSION, PrivateStockDecision, PrivateStockDecisionRecord,
    PrivateStockPolicy, PrivateStockReason, PrivateStockReport, PrivateStockRouteScore,
    assess_report, assign_route_ranks,
};
pub use review::{
    CHEMICAL_REVIEW_RUBRIC_VERSION, ChemicalReview, ReviewDimension, ReviewFinding, ReviewSeverity,
    ReviewStatus, review_report,
};
pub use route_graph::{
    ParseOutcome, ReactionEvidence, RouteDocument, RouteNode, RouteSource, RouteStep,
    SynPlannerRuleProvenance, normalize_renkin_route,
};
pub use synplanner::{SynPlannerNode, normalize_synplanner_route, parse_synplanner_routes};
pub use syntheseus::{SyntheseusRouteV1, normalize_syntheseus_route};
