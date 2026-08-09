//! MCP (Model Context Protocol) server support for RENKIN.
//!
//! Dual-era stdio transport: legacy 2024-11-05 (`initialize` handshake) and
//! modern 2026-07-28 (`server/discover`, per-request `_meta`, no
//! handshake). See `docs/guides/mcp.md` for the protocol support matrix and
//! `tests/fixtures/mcp/` for the schema/transcript provenance this
//! implementation is checked against.
//!
//! - [`jsonrpc`] — transport-agnostic JSON-RPC 2.0 message parsing/errors.
//! - [`protocol`] — era pinning, per-request `_meta` validation, response
//!   envelopes.
//! - [`tools`] — RENKIN tool definitions and business logic, era-agnostic.
//! - [`stdio`] — the newline-delimited stdio transport loop.

pub mod jsonrpc;
pub mod protocol;
pub mod stdio;
pub mod tools;
