//! Versioned, content-addressed audit receipts for MCP tool executions.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION: u32 = 1;
const RECEIPT_META_KEY: &str = "io.renkin/auditReceipt";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditReceipt {
    pub schema_version: u32,
    pub receipt_id: String,
    pub task_id: String,
    pub parent_task_id: Option<String>,
    pub tool: String,
    pub server_version: String,
    pub model: Option<String>,
    pub arguments_sha256: String,
    pub result_sha256: String,
    pub status: &'static str,
    pub failure_code: Option<&'static str>,
    pub timestamp_unix_ms: Option<u64>,
}

impl AuditReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: String,
        parent_task_id: Option<String>,
        tool: &str,
        server_version: &str,
        model: Option<String>,
        arguments: &Value,
        result: &Value,
        failed: bool,
    ) -> Self {
        let status = if failed { "failure" } else { "success" };
        let failure_code = failed.then_some("tool_error");
        let arguments_sha256 = sha256_value(arguments);
        let result_sha256 = sha256_value(result);
        let receipt_id = Self::deterministic_id(
            &task_id,
            &parent_task_id,
            tool,
            server_version,
            &model,
            &arguments_sha256,
            &result_sha256,
            status,
            failure_code,
        );
        Self {
            schema_version: SCHEMA_VERSION,
            receipt_id,
            task_id,
            parent_task_id,
            tool: tool.to_owned(),
            server_version: server_version.to_owned(),
            model,
            arguments_sha256,
            result_sha256,
            status,
            failure_code,
            timestamp_unix_ms: timestamp_unix_ms(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn deterministic_id(
        task_id: &str,
        parent_task_id: &Option<String>,
        tool: &str,
        server_version: &str,
        model: &Option<String>,
        arguments_sha256: &str,
        result_sha256: &str,
        status: &str,
        failure_code: Option<&str>,
    ) -> String {
        let material = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "task_id": task_id,
            "parent_task_id": parent_task_id,
            "tool": tool,
            "server_version": server_version,
            "model": model,
            "arguments_sha256": arguments_sha256,
            "result_sha256": result_sha256,
            "status": status,
            "failure_code": failure_code,
        });
        format!("sha256:{}", sha256_value(&material))
    }

    pub fn verify_integrity(&self) -> bool {
        self.schema_version == SCHEMA_VERSION
            && self.receipt_id
                == Self::deterministic_id(
                    &self.task_id,
                    &self.parent_task_id,
                    &self.tool,
                    &self.server_version,
                    &self.model,
                    &self.arguments_sha256,
                    &self.result_sha256,
                    self.status,
                    self.failure_code,
                )
    }

    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "schemaVersion": self.schema_version,
            "receiptId": self.receipt_id,
            "taskId": self.task_id,
            "parentTaskId": self.parent_task_id,
            "tool": self.tool,
            "serverVersion": self.server_version,
            "model": self.model,
            "argumentsSha256": self.arguments_sha256,
            "resultSha256": self.result_sha256,
            "status": self.status,
            "failureCode": self.failure_code,
            "timestampUnixMs": self.timestamp_unix_ms,
        })
    }

    pub const fn meta_key() -> &'static str {
        RECEIPT_META_KEY
    }
}

pub fn sha256_value(value: &Value) -> String {
    let mut bytes = Vec::new();
    write_canonical(value, &mut bytes);
    format!("sha256:{}", crate::sha256_hex(Sha256::digest(bytes)))
}

fn write_canonical(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(v) => out.extend_from_slice(if *v { b"true" } else { b"false" }),
        Value::Number(v) => out.extend_from_slice(v.to_string().as_bytes()),
        Value::String(v) => out.extend_from_slice(serde_json::to_string(v).unwrap().as_bytes()),
        Value::Array(values) => {
            out.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                write_canonical(value, out);
            }
            out.push(b']');
        }
        Value::Object(values) => {
            out.push(b'{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                write_canonical(&Value::String(key.clone()), out);
                out.push(b':');
                write_canonical(value, out);
            }
            out.push(b'}');
        }
    }
}

fn timestamp_unix_ms() -> Option<u64> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

pub fn string_meta(meta: &Value, key: &str) -> Option<String> {
    meta.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hash_ignores_object_key_order() {
        assert_eq!(
            sha256_value(&serde_json::json!({"b": 2, "a": 1})),
            sha256_value(&serde_json::json!({"a": 1, "b": 2}))
        );
    }

    #[test]
    fn receipt_does_not_include_raw_arguments() {
        let receipt = AuditReceipt::new(
            "7".into(),
            None,
            "find_routes",
            "1.0.4",
            None,
            &serde_json::json!({"smiles": "secret"}),
            &serde_json::json!({"content": []}),
            false,
        );
        assert!(!receipt.to_value().to_string().contains("secret"));
        assert_eq!(receipt.schema_version, SCHEMA_VERSION);
    }
}
