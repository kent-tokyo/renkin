#![forbid(unsafe_code)]

//! Route/step plausibility checks, shared by `renkin-bench` and `renkin-mcp`.

pub mod atom_conservation;
pub mod forward;

pub use atom_conservation::{route_balanced, step_balanced};
pub use forward::route_forward_validated;
