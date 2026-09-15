//! Local verification that binds MCP receipts to a canonical route re-audit.
//!
//! An [`crate::mcp::audit_receipt::AuditReceipt`] deliberately stores hashes,
//! rather than potentially confidential arguments and results.  A caller that
//! still holds those bodies can put them in this *local-only* sidecar, verify
//! them, and retain only the resulting binding receipt.  The binding receipt
//! is therefore useful in a manifest without publishing structures, vendor
//! data, or tool output.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bridge::audit::AuditPolicy;
use crate::bridge::interchange::{InterchangeReaudit, reauditable_import_v1};
use crate::chem_env::RetroRule;
use crate::mcp::audit_receipt::sha256_value;

pub const RECEIPT_BINDING_SCHEMA_VERSION: u32 = 1;

/// Local material required to verify one hash-only MCP receipt.  This type is
/// intentionally accepted as input only; it is never embedded in
/// [`ReceiptBindingReceipt`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptBindingInput {
    pub route_id: String,
    #[serde(default)]
    pub canonical_node_id: Option<String>,
    pub receipt: ReceiptMaterial,
    pub arguments: Value,
    pub result: Value,
}

/// Deserializable form of the stable fields emitted by `AuditReceipt::to_value`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptMaterial {
    pub schema_version: u32,
    pub receipt_id: String,
    pub task_id: String,
    #[serde(default)]
    pub parent_task_id: Option<String>,
    pub tool: String,
    pub server_version: String,
    #[serde(default)]
    pub model: Option<String>,
    pub arguments_sha256: String,
    pub result_sha256: String,
    pub status: String,
    #[serde(default)]
    pub failure_code: Option<String>,
}

/// Public, non-sensitive proof that a receipt was checked against locally held
/// material and a particular re-audited route.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReceiptBindingReceipt {
    pub schema_version: u32,
    pub route_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_node_id: Option<String>,
    pub receipt_id: String,
    pub task_id: String,
    pub tool: String,
    pub arguments_sha256: String,
    pub result_sha256: String,
    pub final_audit_sha256: String,
    pub canonical_interchange_sha256: String,
}

/// A verified, evidence-carrying re-audit.  It retains hashes and identifiers
/// only: arguments and results never escape the local verifier.
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceChainVerification {
    pub schema_version: u32,
    pub reaudited: InterchangeReaudit,
    pub final_audit_sha256: String,
    pub canonical_interchange_sha256: String,
    pub receipt_bindings: Vec<ReceiptBindingReceipt>,
}

/// Re-audit a v1 canonical interchange and bind local MCP receipt material to
/// its exact canonical bytes and final audit result.
///
/// This is deliberately fail-closed: every receipt must reference the route,
/// match the original arguments/result bodies, have the current deterministic
/// receipt identifier, and (when supplied) name a canonical v1 step ID.
pub fn verify_evidence_chain_v1(
    interchange: &Value,
    configured_stock: &HashSet<String>,
    rules: &[RetroRule],
    policy: AuditPolicy,
    bindings: &[ReceiptBindingInput],
) -> anyhow::Result<EvidenceChainVerification> {
    let reaudited = reauditable_import_v1(interchange, configured_stock, rules, policy)?;
    let final_audit_sha256 = sha256_value(&serde_json::to_value(&reaudited.audit)?);
    let canonical_interchange_sha256 = sha256_value(interchange);
    let valid_node_ids = interchange
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("canonical interchange steps are required"))?
        .iter()
        .filter_map(|step| step.get("canonical_node_id").and_then(Value::as_str))
        .collect::<HashSet<_>>();

    let receipt_bindings = bindings
        .iter()
        .map(|binding| {
            verify_binding(
                binding,
                &reaudited.recomputed_route_id,
                &valid_node_ids,
                &final_audit_sha256,
                &canonical_interchange_sha256,
            )
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(EvidenceChainVerification {
        schema_version: RECEIPT_BINDING_SCHEMA_VERSION,
        reaudited,
        final_audit_sha256,
        canonical_interchange_sha256,
        receipt_bindings,
    })
}

fn verify_binding(
    binding: &ReceiptBindingInput,
    route_id: &str,
    valid_node_ids: &HashSet<&str>,
    final_audit_sha256: &str,
    canonical_interchange_sha256: &str,
) -> anyhow::Result<ReceiptBindingReceipt> {
    if binding.route_id != route_id {
        anyhow::bail!("receipt binding route_id does not match re-audited route");
    }
    if binding
        .canonical_node_id
        .as_deref()
        .is_some_and(|node| !valid_node_ids.contains(node))
    {
        anyhow::bail!("receipt binding canonical_node_id is absent from interchange");
    }
    let receipt = &binding.receipt;
    if receipt.schema_version != crate::mcp::audit_receipt::SCHEMA_VERSION {
        anyhow::bail!("unsupported audit receipt schema_version");
    }
    if !matches!(receipt.status.as_str(), "success" | "failure") {
        anyhow::bail!("audit receipt status must be success or failure");
    }
    if receipt.arguments_sha256 != sha256_value(&binding.arguments) {
        anyhow::bail!("receipt arguments_sha256 does not match local arguments");
    }
    if receipt.result_sha256 != sha256_value(&binding.result) {
        anyhow::bail!("receipt result_sha256 does not match local result");
    }
    if receipt.receipt_id != deterministic_receipt_id(receipt) {
        anyhow::bail!("receipt_id does not match receipt material");
    }
    Ok(ReceiptBindingReceipt {
        schema_version: RECEIPT_BINDING_SCHEMA_VERSION,
        route_id: route_id.to_owned(),
        canonical_node_id: binding.canonical_node_id.clone(),
        receipt_id: receipt.receipt_id.clone(),
        task_id: receipt.task_id.clone(),
        tool: receipt.tool.clone(),
        arguments_sha256: receipt.arguments_sha256.clone(),
        result_sha256: receipt.result_sha256.clone(),
        final_audit_sha256: final_audit_sha256.to_owned(),
        canonical_interchange_sha256: canonical_interchange_sha256.to_owned(),
    })
}

fn deterministic_receipt_id(receipt: &ReceiptMaterial) -> String {
    // Keep the original v1 wire convention exactly, including its historical
    // `sha256:sha256:<hex>` prefix.  Changing it would invalidate receipts
    // already emitted by the MCP server.
    let material = serde_json::json!({
        "schema_version": receipt.schema_version,
        "task_id": receipt.task_id,
        "parent_task_id": receipt.parent_task_id,
        "tool": receipt.tool,
        "server_version": receipt.server_version,
        "model": receipt.model,
        "arguments_sha256": receipt.arguments_sha256,
        "result_sha256": receipt.result_sha256,
        "status": receipt.status,
        "failure_code": receipt.failure_code,
    });
    format!("sha256:{}", sha256_value(&material))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn material(arguments: &Value, result: &Value) -> ReceiptMaterial {
        let arguments_sha256 = sha256_value(arguments);
        let result_sha256 = sha256_value(result);
        let mut receipt = ReceiptMaterial {
            schema_version: 1,
            receipt_id: String::new(),
            task_id: "task-7".into(),
            parent_task_id: None,
            tool: "find_routes".into(),
            server_version: "1.0.7".into(),
            model: None,
            arguments_sha256,
            result_sha256,
            status: "success".into(),
            failure_code: None,
        };
        receipt.receipt_id = deterministic_receipt_id(&receipt);
        receipt
    }

    #[test]
    fn binding_checks_body_hash_and_does_not_emit_body() {
        let arguments = json!({"smiles": "CCO"});
        let result = json!({"private_result": "supplier-only"});
        let binding = ReceiptBindingInput {
            route_id: "route".into(),
            canonical_node_id: Some("node".into()),
            receipt: material(&arguments, &result),
            arguments,
            result,
        };
        let nodes = HashSet::from(["node"]);
        let verified =
            verify_binding(&binding, "route", &nodes, "sha256:audit", "sha256:route").unwrap();
        let output = serde_json::to_string(&verified).unwrap();
        assert!(!output.contains("CCO"));
        assert!(!output.contains("supplier-only"));
    }

    #[test]
    fn binding_rejects_tampered_local_body() {
        let arguments = json!({"smiles": "CCO"});
        let result = json!({"routes": []});
        let binding = ReceiptBindingInput {
            route_id: "route".into(),
            canonical_node_id: None,
            receipt: material(&arguments, &result),
            arguments: json!({"smiles": "CCN"}),
            result,
        };
        assert!(verify_binding(&binding, "route", &HashSet::new(), "audit", "route").is_err());
    }
}
