//! Dual-era MCP protocol handling: legacy 2024-11-05 (`initialize` handshake)
//! and modern 2026-07-28 (`server/discover`, per-request `_meta`, no
//! handshake). Wire shapes for the modern era are grounded in the official
//! draft schema vendored at `tests/fixtures/mcp/2026-07-28-rc/` — see that
//! directory's README for exact provenance (commit, SHA-256, license).

use crate::mcp::{jsonrpc, tools};
use serde_json::{Value, json};

pub const LEGACY_PROTOCOL_VERSION: &str = "2024-11-05";
pub const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which era a stdio connection has been pinned to, decided by the first
/// non-notification request it sends. See `McpServer::handle`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Era {
    Unpinned,
    Legacy,
    Modern,
}

pub struct McpServer {
    pub era: Era,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
    pub fn new() -> Self {
        McpServer { era: Era::Unpinned }
    }

    fn server_info(&self) -> Value {
        json!({"name": "renkin", "version": VERSION})
    }

    /// Handles one parsed request/notification. Returns `None` for
    /// notifications (which the JSON-RPC 2.0 spec forbids responding to) and
    /// for anything else that must not pin an `Unpinned` connection.
    pub fn handle(&mut self, msg: &jsonrpc::Message) -> Option<Value> {
        if jsonrpc::is_notification(msg) {
            return None;
        }
        let id = msg.id.clone().expect("checked by is_notification above");
        let method = msg.method.as_str();
        let params = &msg.params;

        match self.era {
            Era::Unpinned => self.handle_unpinned(id, method, params),
            Era::Legacy => Some(self.handle_legacy(id, method, params)),
            Era::Modern => Some(self.handle_modern_pinned(id, method, params)),
        }
    }

    fn handle_unpinned(&mut self, id: Value, method: &str, params: &Value) -> Option<Value> {
        match method {
            "initialize" => {
                self.era = Era::Legacy;
                Some(jsonrpc::result_response(
                    id,
                    legacy_initialize_result(&self.server_info()),
                ))
            }
            "server/discover" | "tools/list" | "tools/call" => {
                match validate_modern_meta(params) {
                    Ok(()) => {
                        self.era = Era::Modern;
                        Some(self.dispatch_modern_method(id, method, params))
                    }
                    Err((code, message, data)) if method == "server/discover" => {
                        // A malformed `server/discover` call is a genuine
                        // validation failure on an otherwise-modern-shaped
                        // request (its params are REQUIRED to carry `_meta`
                        // per the spec) — surface it directly, still Unpinned.
                        Some(jsonrpc::error_response(id, code, &message, data))
                    }
                    Err(_) => {
                        // tools/list or tools/call with no (or invalid)
                        // modern _meta as the OPENING request is ambiguous:
                        // it could be a legacy client that skipped
                        // `initialize`. Don't guess — stay Unpinned so the
                        // next message (a valid `initialize` or a valid
                        // modern request) can still pin correctly.
                        Some(jsonrpc::error_response(
                            id,
                            jsonrpc::INVALID_REQUEST,
                            "cannot determine protocol era: send \"initialize\" for 2024-11-05 clients, or include valid _meta (io.modelcontextprotocol/protocolVersion + clientCapabilities) for 2026-07-28 clients",
                            None,
                        ))
                    }
                }
            }
            _ => Some(jsonrpc::error_response(
                id,
                jsonrpc::METHOD_NOT_FOUND,
                "Method not found",
                None,
            )),
        }
    }

    /// Legacy dispatch — intentionally preserves the pre-refactor binary's
    /// behavior exactly, including its pre-existing quirks (see
    /// `tools::dispatch_legacy`). Regression-checked against
    /// `tests/fixtures/mcp/2024-11-05/legacy_transcript_output.jsonl`.
    fn handle_legacy(&self, id: Value, method: &str, params: &Value) -> Value {
        match method {
            "initialize" => {
                jsonrpc::result_response(id, legacy_initialize_result(&self.server_info()))
            }
            "tools/list" => jsonrpc::result_response(id, legacy_tools_list_result()),
            "tools/call" => {
                jsonrpc::result_response(id, tools::dispatch_legacy(params).to_legacy_value())
            }
            _ => jsonrpc::error_response(id, jsonrpc::METHOD_NOT_FOUND, "Method not found", None),
        }
    }

    /// Modern dispatch once a connection is already pinned to 2026-07-28.
    /// `initialize` has no modern definition at all (it does not appear in
    /// the 2026-07-28 schema — clients negotiate via `_meta` instead), so it
    /// naturally falls through to `METHOD_NOT_FOUND`, which satisfies "a
    /// modern connection must reject `initialize` as a protocol error"
    /// without any special-casing.
    fn handle_modern_pinned(&mut self, id: Value, method: &str, params: &Value) -> Value {
        match method {
            "server/discover" | "tools/list" | "tools/call" => match validate_modern_meta(params) {
                Ok(()) => self.dispatch_modern_method(id, method, params),
                Err((code, message, data)) => jsonrpc::error_response(id, code, &message, data),
            },
            _ => jsonrpc::error_response(id, jsonrpc::METHOD_NOT_FOUND, "Method not found", None),
        }
    }

    fn dispatch_modern_method(&self, id: Value, method: &str, params: &Value) -> Value {
        match method {
            "server/discover" => jsonrpc::result_response(id, self.wrap_modern(discover_result())),
            "tools/list" => {
                jsonrpc::result_response(id, self.wrap_modern(modern_tools_list_result()))
            }
            "tools/call" => match modern_tools_call(params) {
                Ok(outcome) => {
                    jsonrpc::result_response(id, self.wrap_modern(outcome.to_modern_value()))
                }
                Err((code, message, data)) => jsonrpc::error_response(id, code, &message, data),
            },
            _ => unreachable!("dispatch_modern_method only called for the three modern methods"),
        }
    }

    /// Adds the modern envelope fields (`resultType`, `_meta.serverInfo`) to
    /// a handler-produced result object without disturbing any other key the
    /// handler set. Never applied to error responses — the 2026-07-28 schema
    /// gives `JSONRPCErrorResponse` no `_meta` field at all.
    fn wrap_modern(&self, mut result: Value) -> Value {
        let obj = result
            .as_object_mut()
            .expect("modern result builders always return a JSON object");
        obj.entry("resultType".to_string())
            .or_insert_with(|| json!("complete"));
        let meta = obj.entry("_meta".to_string()).or_insert_with(|| json!({}));
        if !meta.is_object() {
            *meta = json!({});
        }
        meta["io.modelcontextprotocol/serverInfo"] = self.server_info();
        result
    }
}

fn legacy_initialize_result(server_info: &Value) -> Value {
    json!({
        "protocolVersion": LEGACY_PROTOCOL_VERSION,
        "capabilities": {"tools": {}},
        "serverInfo": server_info,
    })
}

fn legacy_tools_list_result() -> Value {
    let list: Vec<Value> = tools::TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.legacy_input_schema)(),
            })
        })
        .collect();
    json!({"tools": list})
}

fn discover_result() -> Value {
    json!({
        "supportedVersions": [MODERN_PROTOCOL_VERSION],
        "capabilities": {"tools": {}},
        "instructions": "RENKIN provides retrosynthetic route search and route-analysis tools.",
        "ttlMs": 3_600_000,
        "cacheScope": "public",
    })
}

fn modern_tools_list_result() -> Value {
    let list: Vec<Value> = tools::TOOLS
        .iter()
        .map(|t| {
            let mut tool = json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": (t.modern_input_schema)(),
            });
            if let Some(output_schema) = t.modern_output_schema {
                tool["outputSchema"] = output_schema();
            }
            tool
        })
        .collect();
    json!({
        "tools": list,
        "ttlMs": 3_600_000,
        "cacheScope": "public",
    })
}

type ProtocolError = (i64, String, Option<Value>);

/// Validates the per-request `_meta` object required on every modern
/// request (`RequestParams`/`RequestMetaObject` in the vendored schema).
/// `io.modelcontextprotocol/protocolVersion` and `.../clientCapabilities`
/// are required; `.../clientInfo` is optional but must be well-formed if
/// present. The client's identity is validated but never used to change
/// behavior (no authz/feature branching on clientInfo).
fn validate_modern_meta(params: &Value) -> Result<(), ProtocolError> {
    let Some(meta) = params.get("_meta").filter(|m| m.is_object()) else {
        return Err((
            jsonrpc::INVALID_PARAMS,
            "missing or malformed \"_meta\" object in params".to_string(),
            None,
        ));
    };

    let protocol_version = match meta
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
    {
        Some(v) => v,
        None => {
            return Err((
                jsonrpc::INVALID_PARAMS,
                "missing required _meta field: io.modelcontextprotocol/protocolVersion".to_string(),
                None,
            ));
        }
    };
    if protocol_version != MODERN_PROTOCOL_VERSION {
        return Err((
            jsonrpc::UNSUPPORTED_PROTOCOL_VERSION,
            "Unsupported protocol version".to_string(),
            Some(json!({
                "supported": [MODERN_PROTOCOL_VERSION],
                "requested": protocol_version,
            })),
        ));
    }

    match meta.get("io.modelcontextprotocol/clientCapabilities") {
        Some(c) if c.is_object() => {}
        _ => {
            return Err((
                jsonrpc::INVALID_PARAMS,
                "missing or malformed _meta field: io.modelcontextprotocol/clientCapabilities (must be an object)".to_string(),
                None,
            ));
        }
    }

    if let Some(client_info) = meta.get("io.modelcontextprotocol/clientInfo") {
        let well_formed = client_info.is_object()
            && client_info.get("name").is_some_and(Value::is_string)
            && client_info.get("version").is_some_and(Value::is_string);
        if !well_formed {
            return Err((
                jsonrpc::INVALID_PARAMS,
                "malformed _meta field: io.modelcontextprotocol/clientInfo requires string \"name\" and \"version\"".to_string(),
                None,
            ));
        }
    }

    Ok(())
}

/// Modern `tools/call`: schema-violations (unknown tool, missing/malformed
/// required arguments) are protocol-level `Invalid Params` errors, not
/// `isError: true` tool results — this follows the official schema's own
/// split (`InvalidParamsError` doc: "Tools: Unknown tool name or invalid
/// tool arguments") rather than this task's illustrative example, which
/// showed a missing-argument case as a tool-level error. That example
/// predates checking the schema against blog/SDK guesses; the legacy era
/// keeps the old (tool-level-error) behavior unchanged via
/// `tools::dispatch_legacy`, since fixing it there would be a legacy
/// behavior change and is out of scope.
fn modern_tools_call(params: &Value) -> Result<tools::ToolOutcome, ProtocolError> {
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| {
        (
            jsonrpc::INVALID_PARAMS,
            "missing required param: name".to_string(),
            None,
        )
    })?;
    let Some(def) = tools::find(name) else {
        return Err((
            jsonrpc::INVALID_PARAMS,
            format!("Unknown tool: {name}"),
            None,
        ));
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let schema = (def.modern_input_schema)();
    if let Err(message) = tools::validate_modern_args(&schema, &args) {
        return Err((jsonrpc::INVALID_PARAMS, message, None));
    }
    let smiles = args["smiles"]
        .as_str()
        .expect("validate_modern_args guarantees a required string \"smiles\"");
    Ok((def.handler)(smiles, &args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: i64, method: &str, params: Value) -> jsonrpc::Message {
        jsonrpc::Message {
            id: Some(json!(id)),
            method: method.to_string(),
            params,
        }
    }

    fn notification(method: &str) -> jsonrpc::Message {
        jsonrpc::Message {
            id: None,
            method: method.to_string(),
            params: Value::Null,
        }
    }

    fn modern_meta() -> Value {
        json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {},
            }
        })
    }

    #[test]
    fn initialize_pins_legacy() {
        let mut s = McpServer::new();
        let resp = s
            .handle(&req(
                1,
                "initialize",
                json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "c", "version": "1"}}),
            ))
            .unwrap();
        assert_eq!(s.era, Era::Legacy);
        assert_eq!(resp["result"]["protocolVersion"], LEGACY_PROTOCOL_VERSION);
        assert!(resp["result"].get("resultType").is_none());
        assert!(resp["result"]["_meta"].is_null());
    }

    #[test]
    fn discover_first_pins_modern() {
        let mut s = McpServer::new();
        let resp = s.handle(&req(1, "server/discover", modern_meta())).unwrap();
        assert_eq!(s.era, Era::Modern);
        assert_eq!(resp["result"]["resultType"], "complete");
        assert_eq!(
            resp["result"]["supportedVersions"],
            json!([MODERN_PROTOCOL_VERSION])
        );
        assert_eq!(resp["result"]["capabilities"], json!({"tools": {}}));
        assert_eq!(
            resp["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "renkin"
        );
        assert!(resp["result"]["ttlMs"].as_u64().unwrap() > 0);
        assert_eq!(resp["result"]["cacheScope"], "public");
        assert!(resp["result"]["capabilities"].get("prompts").is_none());
        assert!(resp["result"]["capabilities"].get("resources").is_none());
    }

    #[test]
    fn inline_modern_tools_list_pins_modern_without_discover() {
        let mut s = McpServer::new();
        let resp = s.handle(&req(1, "tools/list", modern_meta())).unwrap();
        assert_eq!(s.era, Era::Modern);
        assert_eq!(resp["result"]["resultType"], "complete");
        assert!(resp["result"]["ttlMs"].as_i64().unwrap() >= 0);
        assert_eq!(resp["result"]["cacheScope"], "public");
    }

    #[test]
    fn inline_modern_tools_call_pins_modern_without_discover() {
        let mut s = McpServer::new();
        let mut params = modern_meta();
        params["name"] = json!("find_routes");
        params["arguments"] = json!({"smiles": "CCO", "max_routes": 1, "depth": 2});
        let resp = s.handle(&req(1, "tools/call", params)).unwrap();
        assert_eq!(s.era, Era::Modern);
        assert_eq!(resp["result"]["resultType"], "complete");
    }

    #[test]
    fn ambiguous_tools_list_without_meta_does_not_pin_and_next_request_still_can() {
        let mut s = McpServer::new();
        let resp1 = s.handle(&req(1, "tools/list", Value::Null)).unwrap();
        assert_eq!(s.era, Era::Unpinned);
        assert_eq!(resp1["error"]["code"], jsonrpc::INVALID_REQUEST);

        // A subsequent valid modern request must still be able to pin.
        let resp2 = s.handle(&req(2, "server/discover", modern_meta())).unwrap();
        assert_eq!(s.era, Era::Modern);
        assert_eq!(resp2["result"]["resultType"], "complete");
    }

    #[test]
    fn malformed_discover_stays_unpinned_and_reports_the_real_error() {
        let mut s = McpServer::new();
        let resp = s
            .handle(&req(1, "server/discover", json!({"_meta": {}})))
            .unwrap();
        assert_eq!(s.era, Era::Unpinned);
        assert_eq!(resp["error"]["code"], jsonrpc::INVALID_PARAMS);
    }

    #[test]
    fn notification_never_pins_and_never_responds() {
        let mut s = McpServer::new();
        assert!(
            s.handle(&notification("notifications/initialized"))
                .is_none()
        );
        assert_eq!(s.era, Era::Unpinned);
    }

    #[test]
    fn legacy_then_modern_request_is_rejected() {
        let mut s = McpServer::new();
        s.handle(&req(1, "initialize", json!({}))).unwrap();
        assert_eq!(s.era, Era::Legacy);
        let resp = s.handle(&req(2, "server/discover", modern_meta())).unwrap();
        assert_eq!(resp["error"]["code"], jsonrpc::METHOD_NOT_FOUND);
        assert_eq!(s.era, Era::Legacy);
    }

    #[test]
    fn modern_then_initialize_is_rejected() {
        let mut s = McpServer::new();
        s.handle(&req(1, "server/discover", modern_meta())).unwrap();
        assert_eq!(s.era, Era::Modern);
        let resp = s.handle(&req(2, "initialize", json!({}))).unwrap();
        assert_eq!(resp["error"]["code"], jsonrpc::METHOD_NOT_FOUND);
        assert_eq!(s.era, Era::Modern);
    }

    #[test]
    fn modern_request_without_protocol_version_is_invalid_params() {
        let mut s = McpServer::new();
        let resp = s
            .handle(&req(
                1,
                "server/discover",
                json!({"_meta": {"io.modelcontextprotocol/clientCapabilities": {}}}),
            ))
            .unwrap();
        assert_eq!(resp["error"]["code"], jsonrpc::INVALID_PARAMS);
    }

    #[test]
    fn modern_request_with_wrong_protocol_version_is_unsupported_protocol_version() {
        let mut s = McpServer::new();
        let mut meta = modern_meta();
        meta["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2025-06-18");
        let resp = s.handle(&req(1, "server/discover", meta)).unwrap();
        assert_eq!(resp["error"]["code"], jsonrpc::UNSUPPORTED_PROTOCOL_VERSION);
        assert_eq!(resp["error"]["data"]["requested"], "2025-06-18");
        assert_eq!(
            resp["error"]["data"]["supported"],
            json!([MODERN_PROTOCOL_VERSION])
        );
    }

    #[test]
    fn modern_request_missing_client_capabilities_is_invalid_params() {
        let mut s = McpServer::new();
        let resp = s
            .handle(&req(
                1,
                "server/discover",
                json!({"_meta": {"io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION}}),
            ))
            .unwrap();
        assert_eq!(resp["error"]["code"], jsonrpc::INVALID_PARAMS);
    }

    #[test]
    fn modern_request_absent_client_info_is_fine() {
        let mut s = McpServer::new();
        let resp = s.handle(&req(1, "server/discover", modern_meta())).unwrap();
        assert!(resp.get("error").is_none());
    }

    #[test]
    fn modern_request_malformed_client_info_is_rejected() {
        let mut s = McpServer::new();
        let mut meta = modern_meta();
        meta["_meta"]["io.modelcontextprotocol/clientInfo"] = json!({"name": "only-name"});
        let resp = s.handle(&req(1, "server/discover", meta)).unwrap();
        assert_eq!(resp["error"]["code"], jsonrpc::INVALID_PARAMS);
    }

    #[test]
    fn modern_unknown_tool_is_invalid_params_not_fallback() {
        let mut s = McpServer::new();
        let mut params = modern_meta();
        params["name"] = json!("nonexistent_tool");
        params["arguments"] = json!({"smiles": "CCO"});
        let resp = s.handle(&req(1, "tools/call", params)).unwrap();
        assert_eq!(resp["error"]["code"], jsonrpc::INVALID_PARAMS);
        assert_eq!(resp["error"]["message"], "Unknown tool: nonexistent_tool");
    }

    #[test]
    fn modern_missing_required_argument_is_invalid_params() {
        let mut s = McpServer::new();
        let mut params = modern_meta();
        params["name"] = json!("find_routes");
        params["arguments"] = json!({});
        let resp = s.handle(&req(1, "tools/call", params)).unwrap();
        assert_eq!(resp["error"]["code"], jsonrpc::INVALID_PARAMS);
    }

    #[test]
    fn modern_tool_failure_is_is_error_true_not_protocol_error() {
        let mut s = McpServer::new();
        let mut params = modern_meta();
        params["name"] = json!("find_routes");
        // Not a schema violation (smiles is a well-typed string, present as
        // required) — it fails inside the handler, at `mol_from_smiles`,
        // which is exactly the "business-logic failure" bucket per §10:
        // reported as a tool result with isError, never a protocol error.
        params["arguments"] = json!({"smiles": "not a valid smiles at all((("});
        let resp = s.handle(&req(1, "tools/call", params)).unwrap();
        assert!(resp.get("error").is_none(), "expected a result, got {resp}");
        assert_eq!(resp["result"]["isError"], true);
        assert_eq!(resp["result"]["resultType"], "complete");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.starts_with("search error:"),
            "expected the mol_from_smiles parse failure to surface as a search error, got: {text}"
        );
    }

    #[test]
    fn two_identical_modern_tools_list_calls_are_content_identical() {
        let mut s1 = McpServer::new();
        let mut s2 = McpServer::new();
        let r1 = s1.handle(&req(1, "tools/list", modern_meta())).unwrap();
        let r2 = s2.handle(&req(2, "tools/list", modern_meta())).unwrap();
        assert_eq!(r1["result"], r2["result"]);
    }

    #[test]
    fn error_response_has_no_meta_field() {
        let mut s = McpServer::new();
        let resp = s
            .handle(&req(1, "server/discover", json!({"_meta": {}})))
            .unwrap();
        assert!(resp.get("_meta").is_none());
        assert!(resp["error"].get("_meta").is_none());
    }
}
