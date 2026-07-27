//! Typed JSON-RPC 2.0 message parsing and error construction.
//!
//! Transport- and RENKIN-tool-agnostic: this module knows nothing about
//! stdio framing or MCP protocol eras.

use serde_json::{Value, json};

// Standard JSON-RPC 2.0 error codes.
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

// MCP-specific error codes (schema/draft/schema.ts, 2026-07-28 revision).
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// A successfully-parsed JSON-RPC 2.0 request or notification.
///
/// `id: None` means no `"id"` key was present in the message at all, which
/// per the JSON-RPC 2.0 spec makes it a notification: it MUST NOT receive a
/// response. `id: Some(Value::Null)` is a request whose id is JSON `null`
/// (unusual but legal) and DOES get a response.
#[derive(Debug)]
pub struct Message {
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

pub fn is_notification(msg: &Message) -> bool {
    msg.id.is_none()
}

/// Parses one line of input. On success returns the typed `Message`; on any
/// framing problem returns an already-built JSON-RPC error response value
/// (never panics, never silently drops the line).
pub fn parse(line: &str) -> Result<Message, Value> {
    let v: Value = serde_json::from_str(line).map_err(|e| {
        error_response(Value::Null, PARSE_ERROR, &format!("Parse error: {e}"), None)
    })?;

    if v.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        let id = v.get("id").cloned().unwrap_or(Value::Null);
        return Err(error_response(
            id,
            INVALID_REQUEST,
            "Invalid Request: missing or unsupported \"jsonrpc\" version",
            None,
        ));
    }
    let method = match v.get("method").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => {
            let id = v.get("id").cloned().unwrap_or(Value::Null);
            return Err(error_response(
                id,
                INVALID_REQUEST,
                "Invalid Request: missing \"method\"",
                None,
            ));
        }
    };
    let id = v.get("id").cloned();
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    Ok(Message { id, method, params })
}

pub fn result_response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub fn error_response(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let Some(d) = data {
        error["data"] = d;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_uses_null_id() {
        let err = parse("not json").unwrap_err();
        assert_eq!(err["error"]["code"], PARSE_ERROR);
        assert_eq!(err["id"], Value::Null);
    }

    #[test]
    fn missing_id_key_is_a_notification() {
        let msg = parse(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
        assert!(is_notification(&msg));
    }

    #[test]
    fn explicit_null_id_is_not_a_notification() {
        let msg = parse(r#"{"jsonrpc":"2.0","id":null,"method":"tools/list"}"#).unwrap();
        assert!(!is_notification(&msg));
    }

    #[test]
    fn wrong_jsonrpc_version_is_invalid_request() {
        let err = parse(r#"{"jsonrpc":"1.0","id":1,"method":"tools/list"}"#).unwrap_err();
        assert_eq!(err["error"]["code"], INVALID_REQUEST);
    }
}
