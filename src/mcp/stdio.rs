//! stdio transport loop: newline-delimited JSON-RPC 2.0 in, one JSON-RPC
//! message per line out. See module docs on discipline requirements —
//! stdout carries protocol messages ONLY, diagnostics go to stderr, a
//! failed write is never silently swallowed, and a broken pipe (client
//! closed its end) is a normal shutdown rather than an error.

use crate::mcp::{jsonrpc, protocol};
use serde_json::Value;
use std::io::{self, BufRead, Write};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

enum BoundedLine {
    Eof,
    Complete(Vec<u8>),
    TooLarge,
}

/// Read one newline-delimited request without allowing an attacker-controlled
/// line to grow the allocation without bound. Oversized input is consumed
/// through its newline so the following frame remains usable.
fn read_bounded_line(reader: &mut impl BufRead) -> io::Result<BoundedLine> {
    let mut line = Vec::new();
    let mut too_large = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return if too_large {
                Ok(BoundedLine::TooLarge)
            } else if line.is_empty() {
                Ok(BoundedLine::Eof)
            } else {
                Ok(BoundedLine::Complete(line))
            };
        }

        let newline = buf.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(buf.len());
        if !too_large && line.len() + content_len > MAX_REQUEST_BYTES {
            too_large = true;
            line.clear();
        } else if !too_large {
            line.extend_from_slice(&buf[..content_len]);
        }

        let consumed = newline.map_or(buf.len(), |index| index + 1);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(if too_large {
                BoundedLine::TooLarge
            } else {
                BoundedLine::Complete(line)
            });
        }
    }
}

pub fn run() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut server = protocol::McpServer::new();
    let mut input = stdin.lock();

    loop {
        let line = match read_bounded_line(&mut input) {
            Ok(BoundedLine::Eof) => break,
            Ok(BoundedLine::TooLarge) => {
                let response = jsonrpc::error_response(
                    Value::Null,
                    jsonrpc::INVALID_REQUEST,
                    "resource_exhausted: request too large",
                    None,
                );
                if write_message(&mut out, &response).is_err() {
                    break;
                }
                continue;
            }
            Ok(BoundedLine::Complete(bytes)) => match String::from_utf8(bytes) {
                Ok(line) => line,
                Err(_) => {
                    let response = jsonrpc::error_response(
                        Value::Null,
                        jsonrpc::PARSE_ERROR,
                        "Parse error",
                        None,
                    );
                    if write_message(&mut out, &response).is_err() {
                        break;
                    }
                    continue;
                }
            },
            Err(e) => {
                eprintln!("renkin-mcp: error reading stdin, shutting down: {e}");
                break;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if crate::bridge::validate_json_structure(line).is_err() {
            let response = jsonrpc::error_response(
                Value::Null,
                jsonrpc::INVALID_REQUEST,
                "resource_exhausted: JSON structure exceeds server budget",
                None,
            );
            if write_message(&mut out, &response).is_err() {
                break;
            }
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
    use std::io::Cursor;

    #[test]
    fn write_message_emits_exactly_one_trailing_newline_and_no_embedded_ones() {
        let mut buf = Vec::new();
        write_message(&mut buf, &serde_json::json!({"a": "line1\nline2"})).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.matches('\n').count(), 1);
        assert!(s.ends_with('\n'));
        assert!(s.contains("line1\\nline2"));
    }

    #[test]
    fn bounded_reader_rejects_large_lines_and_preserves_following_frame() {
        let mut bytes = vec![b'x'; MAX_REQUEST_BYTES + 1];
        bytes.extend_from_slice(b"\n{}\n");
        let mut input = Cursor::new(bytes);
        assert!(matches!(
            read_bounded_line(&mut input).unwrap(),
            BoundedLine::TooLarge
        ));
        assert!(matches!(
            read_bounded_line(&mut input).unwrap(),
            BoundedLine::Complete(bytes) if bytes == b"{}"
        ));
    }

    #[test]
    fn bounded_reader_accepts_exact_limit_without_newline() {
        let mut input = Cursor::new(vec![b'x'; MAX_REQUEST_BYTES]);
        assert!(matches!(
            read_bounded_line(&mut input).unwrap(),
            BoundedLine::Complete(bytes) if bytes.len() == MAX_REQUEST_BYTES
        ));
    }
}
