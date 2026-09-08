//! Deterministic replay validation for multi-stage agent executions.
//!
//! This module verifies the execution ledger around an agent, not the agent's
//! chemistry. Each stage must carry an independently verifiable MCP receipt;
//! chemical validity remains the canonical audit/forward-replay pipeline's
//! responsibility.

use serde::Serialize;

use crate::mcp::audit_receipt::{AuditReceipt, sha256_value};

pub const AGENT_TRACE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    Retro,
    Condition,
    Forward,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplayFailureCode {
    ReceiptTampered,
    ReceiptTaskMismatch,
    PhaseOrderInvalid,
    PhaseMissing,
    StepAfterFailure,
    ManifestMismatch,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentExecutionRecord {
    pub sequence: u32,
    pub phase: AgentPhase,
    pub receipt: AuditReceipt,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentExecutionTrace {
    pub schema_version: u32,
    pub task_id: String,
    pub records: Vec<AgentExecutionRecord>,
    pub final_audit_manifest_sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReplayFailure {
    pub sequence: Option<u32>,
    pub code: ReplayFailureCode,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayVerification {
    pub passed: bool,
    pub failures: Vec<ReplayFailure>,
    pub verified_records: usize,
}

impl AgentExecutionTrace {
    pub fn new(task_id: String, records: Vec<AgentExecutionRecord>) -> Self {
        let final_audit_manifest_sha256 = manifest_hash(&records);
        Self {
            schema_version: AGENT_TRACE_SCHEMA_VERSION,
            task_id,
            records,
            final_audit_manifest_sha256,
        }
    }

    pub fn verify(&self) -> ReplayVerification {
        let mut failures = Vec::new();
        let mut previous_phase = None;
        let mut failed = false;
        let mut seen_phases = [false; 3];
        for (index, record) in self.records.iter().enumerate() {
            seen_phases[record.phase as usize] = true;
            if record.sequence != index as u32 {
                failures.push(ReplayFailure {
                    sequence: Some(record.sequence),
                    code: ReplayFailureCode::PhaseOrderInvalid,
                    detail: "record sequence is not contiguous".into(),
                });
            }
            if record.receipt.task_id != self.task_id {
                failures.push(ReplayFailure {
                    sequence: Some(record.sequence),
                    code: ReplayFailureCode::ReceiptTaskMismatch,
                    detail: "receipt task_id differs from trace task_id".into(),
                });
            }
            if !record.receipt.verify_integrity() {
                failures.push(ReplayFailure {
                    sequence: Some(record.sequence),
                    code: ReplayFailureCode::ReceiptTampered,
                    detail: "receipt_id does not match its hashed content".into(),
                });
            }
            if previous_phase.is_some_and(|phase| record.phase < phase) {
                failures.push(ReplayFailure {
                    sequence: Some(record.sequence),
                    code: ReplayFailureCode::PhaseOrderInvalid,
                    detail: "phase order must be retro, condition, forward".into(),
                });
            }
            if failed {
                failures.push(ReplayFailure {
                    sequence: Some(record.sequence),
                    code: ReplayFailureCode::StepAfterFailure,
                    detail: "execution continued after a failed receipt".into(),
                });
            }
            failed |= record.receipt.status == "failure";
            previous_phase = Some(record.phase);
        }
        for (index, phase) in [
            AgentPhase::Retro,
            AgentPhase::Condition,
            AgentPhase::Forward,
        ]
        .into_iter()
        .enumerate()
        {
            if !seen_phases[index] {
                failures.push(ReplayFailure {
                    sequence: None,
                    code: ReplayFailureCode::PhaseMissing,
                    detail: format!("required phase {phase:?} is missing"),
                });
            }
        }
        if manifest_hash(&self.records) != self.final_audit_manifest_sha256 {
            failures.push(ReplayFailure {
                sequence: None,
                code: ReplayFailureCode::ManifestMismatch,
                detail: "final audit manifest hash does not match records".into(),
            });
        }
        ReplayVerification {
            passed: failures.is_empty(),
            failures,
            verified_records: self.records.len(),
        }
    }
}

fn manifest_hash(records: &[AgentExecutionRecord]) -> String {
    let material: Vec<_> = records
        .iter()
        .map(|record| {
            serde_json::json!({
                "sequence": record.sequence,
                "phase": record.phase,
                "receipt_id": record.receipt.receipt_id,
                "status": record.receipt.status,
                "failure_code": record.receipt.failure_code,
            })
        })
        .collect();
    sha256_value(&serde_json::Value::Array(material))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn receipt(task_id: &str, failed: bool) -> AuditReceipt {
        AuditReceipt::new(
            task_id.into(),
            None,
            "agent_step",
            "1.0.4",
            Some("test-model".into()),
            &json!({"secret_smiles": "CCO"}),
            &json!({"status": if failed { "failure" } else { "ok" }}),
            failed,
        )
    }

    #[test]
    fn ordered_trace_replays_and_does_not_store_secret_body() {
        let trace = AgentExecutionTrace::new(
            "task-1".into(),
            vec![
                AgentExecutionRecord {
                    sequence: 0,
                    phase: AgentPhase::Retro,
                    receipt: receipt("task-1", false),
                },
                AgentExecutionRecord {
                    sequence: 1,
                    phase: AgentPhase::Condition,
                    receipt: receipt("task-1", false),
                },
                AgentExecutionRecord {
                    sequence: 2,
                    phase: AgentPhase::Forward,
                    receipt: receipt("task-1", false),
                },
            ],
        );
        assert!(trace.verify().passed);
        assert!(!serde_json::to_string(&trace).unwrap().contains("CCO"));
    }

    #[test]
    fn tampered_receipt_and_continuation_after_failure_are_classified() {
        let mut bad = receipt("task-1", false);
        bad.receipt_id = "sha256:tampered".into();
        let trace = AgentExecutionTrace::new(
            "task-1".into(),
            vec![
                AgentExecutionRecord {
                    sequence: 0,
                    phase: AgentPhase::Retro,
                    receipt: receipt("task-1", true),
                },
                AgentExecutionRecord {
                    sequence: 1,
                    phase: AgentPhase::Forward,
                    receipt: bad,
                },
            ],
        );
        let result = trace.verify();
        assert!(!result.passed);
        assert!(
            result
                .failures
                .iter()
                .any(|f| f.code == ReplayFailureCode::ReceiptTampered)
        );
        assert!(
            result
                .failures
                .iter()
                .any(|f| f.code == ReplayFailureCode::StepAfterFailure)
        );
    }
}
