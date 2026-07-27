//! renkin-mcp — MCP server for retrosynthesis via the Model Context Protocol.
//!
//! Transport: JSON-RPC 2.0 over stdio (one JSON object per line). Supports
//! both the legacy 2024-11-05 handshake and the modern 2026-07-28
//! `server/discover` / per-request `_meta` negotiation — see
//! `docs/guides/mcp.md` for the full protocol support matrix.
//!
//! Register in Claude Desktop's `claude_desktop_config.json`:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "renkin": { "command": "/path/to/renkin-mcp" }
//!   }
//! }
//! ```
#![forbid(unsafe_code)]

fn main() {
    renkin::mcp::stdio::run();
}
