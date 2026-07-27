//! stdio transport loop: newline-delimited JSON-RPC 2.0 in, one JSON-RPC
//! message per line out. See module docs on discipline requirements —
//! stdout carries protocol messages ONLY, diagnostics go to stderr, a
//! failed write is never silently swallowed, and a broken pipe (client
//! closed its end) is a normal shutdown rather than an error.

use crate::mcp::{jsonrpc, protocol};
use serde_json::Value;
use std::io::{self, BufRead, Write};

pub fn run() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut server = protocol::McpServer::new();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("renkin-mcp: error reading stdin, shutting down: {e}");
                break;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let response = match jsonrpc::parse(line) {
            Ok(msg) => server.handle(&msg),
            Err(error_response) => Some(error_response),
        };
        let Some(response) = response else {
            continue; // notification: no response permitted
        };

        if let Err(e) = write_message(&mut out, &response) {
            if e.kind() == io::ErrorKind::BrokenPipe {
                break; // client closed its stdin — normal shutdown
            }
            eprintln!("renkin-mcp: failed to write response, shutting down: {e}");
            break;
        }
    }
}

/// Writes exactly one JSON-RPC message as exactly one line. `serde_json`
/// escapes newline bytes inside string values (`\n`, not a literal LF), so
/// the one-message-per-line invariant holds as long as this is the only
/// place that writes to stdout and `to_string_pretty` is never used here.
fn write_message(out: &mut impl Write, msg: &Value) -> io::Result<()> {
    let line = serde_json::to_string(msg).expect("MCP responses are always valid JSON");
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_message_emits_exactly_one_trailing_newline_and_no_embedded_ones() {
        let mut buf = Vec::new();
        write_message(&mut buf, &serde_json::json!({"a": "line1\nline2"})).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.matches('\n').count(), 1);
        assert!(s.ends_with('\n'));
        assert!(s.contains("line1\\nline2"));
    }
}
