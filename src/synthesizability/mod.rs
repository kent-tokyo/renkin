//! Synthesizability Kernel v0 (`docs/design/synthesizability-kernel-v0.md`):
//! takes already-produced search routes plus stock/evidence/validation
//! context and returns a decisive, auditable, machine-readable assessment
//! of how well-supported a route is -- never a claim that a target
//! *cannot* be synthesized, and never an uncalibrated score.
//!
//! This module is a stable single entry point regardless of how many files
//! get added under it (`schema.rs` today; `signals.rs`,
//! `element_accounting.rs`, `assessment.rs`, `provenance.rs` land here in
//! follow-up PRs per the design doc's per-agent file ownership, §7).

mod schema;

pub use schema::*;
