# Agent execution replay

RENKIN's agent replay kernel verifies the execution ledger around a
retro→condition→forward workflow. It does not replace chemical validation;
the canonical route audit and declared forward replay remain the source of
chemical verdicts.

Each stage carries a versioned MCP `AuditReceipt`. The verifier checks:

- receipt ID integrity against the canonical argument/result hashes;
- one task ID across the trace;
- monotonic `retro`, `condition`, `forward` phase order;
- contiguous record sequence numbers;
- failure continuation (later stages after a failed stage are classified);
- final manifest hash over the ordered receipt IDs and statuses.

Receipts and traces contain hashes and identifiers, not SMILES, route bodies,
stock rows, or secrets. The receipt timestamp is execution metadata and is not
part of the deterministic receipt ID or manifest hash.

An invalid trace is a replay failure, not a successful route. The verifier
returns machine-readable failure codes such as `receipt_tampered`,
`receipt_task_mismatch`, `phase_order_invalid`, `step_after_failure`, and
`manifest_mismatch` so an agent runner can stop or classify the run without
guessing.
