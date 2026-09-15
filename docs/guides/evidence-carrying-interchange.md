# Evidence-carrying route interchange

Export a versioned canonical route record together with the audit result:

```bash
renkin audit-route route.json --interchange --output json
```

The optional `route_interchange` block carries the normalized route hash,
canonical step IDs, target and precursor SMILES, forward-replay status and
evidence basis, audit findings, and stock/policy provenance when supplied.

Adapters retain only source metadata that is explicit in their confirmed
input contracts: Syntheseus `source_version` and SynPlanner's top-level route
key are exported as `source_version` and `source_route_id`. SynPlanner's
`tree_node_id` is exported as `original_node_id` (with `step_id` as an
explicit compatibility fallback). Syntheseus's explicit reaction identifier
is exported as `original_node_id` as well. Other versions or original node IDs remain
`null`, never guessed. `canonical_node_id` is
deterministic and derived from the normalized route ID and step index. Where
an adapter supplied a reaction record, the schema carries that typed
`reaction_evidence`; otherwise it explicitly marks the representation as
absent. A replay status is never silently promoted into a preserved SMIRKS or
a chemical-quality claim. For AiZynthFinder, the typed evidence also retains
the source `template_hash` and `classification` when those fields are present;
they are provenance, not an independent quality verdict.

`--interchange` emits the compatibility v1 format. `--interchange-v2` emits
an explicit occurrence tree, retaining direct-purchase roots and repeated
precursor occurrences rather than reconstructing them from flat steps. The
Rust bridge exposes `reauditable_import_v2`; node IDs derive from route hash
and child-index path. Human or LLM review can remain a separate judge record.

Every export also contains a required `loss_report` with schema version `1`.
It records each canonical field as `preserved`, `normalized`, `inferred`,
`dropped`, or `unsupported`, with a reason. Missing or malformed loss reports
are rejected by strict import validation; an `unsupported` field is reported as
loss and is never silently promoted to chemical validity or route success.

The Rust bridge exposes `validate_strict_import` for envelope validation and
`reauditable_import_v1` for strict re-import followed by the ordinary audit.
The CLI exposes the same path:

```bash
renkin audit-route interchange.json --format interchange --stock stock.smi --output json
```

Version 1 is a flattened step list. Re-import accepts it only when one root,
one decomposition per molecule, an acyclic topology, canonical SMILES, and a
matching reconstructed route hash can be proven. It requires configured stock,
because v1 did not retain leaf-stock claims. Direct-purchase routes, duplicate
decompositions, cycles, disconnected components, unknown fields, and hash
mismatches are rejected rather than inferred. The re-audit recomputes the
structure, stock, element, and forward verdicts; it does not trust the stored
`audit_status`.

## Local process-mass receipts

Attach a local JSON array of `ProcessMassLedger` records by route hash:

```bash
renkin audit-route route.json --stock stock.smi \
  --route-metrics process-ledgers.json --output json
```

Each ledger must include an exact `route_id`, boundary, product mass, input
categories, method, and source hash. PMI is emitted only when every declared
required category has a finite positive mass. E-factor additionally needs an
explicit waste mass; it is not derived as `PMI - 1`. Missing inputs produce
`not_evaluable` with a reason code, never zero. Reported source values remain
separate from RENKIN's recomputation.

`--route-metrics-sidecar process-sidecar.json` is the stricter local form.
Its `source_artifact` and `ledgers` are checked by recomputing the artifact
hash and requiring every ledger's `source_sha256` to match. The public report
contains source, sidecar, and receipt hashes only—not the artifact.

## MCP execution receipts

For canonical v1 re-audit, `--receipt-bindings bindings.json` verifies local
arguments and results against their hash-only MCP receipt, then binds it to
the canonical interchange and final audit hashes. Raw tool material is never
serialized; this form requires canonical input and `--stock`.

## Image and OCR input provenance

`--input-artifact artifact.json` accepts a local `InputArtifactReceipt` for a
single audited target. It records the source-content hash, image/SVG/PDF/text
kind, optional transform lineage, OCR tool/model/version, normalization, and
review state. It does not run OCR, render input, or fetch a URL. The audit
report emits only a redacted receipt: no locator, raw prediction, normalized
SMILES, reviewer identifier, or review reason. Confidence needs declared
semantics and is never treated as proof that the drawing was recognized
correctly.

## Post-audit Pareto ranking

`--audit-ranking ranking.json` accepts route-ID-keyed objective vectors only
after normal auditing. Every vector declares its direction, unit, and basis.
Routes are compared only when all values are finite and those contracts match;
unknown values or differing currencies/process boundaries become
`incomparable`, never zero. A failed audit is always `rejected`, and a partial
audit is at most `needs_review`, regardless of supplied objective values.
`--weighted-ranking weighted.json` uses a fixed profile: every axis declares
key, direction, unit, basis, bounds, and positive weight, with all weights
totaling one. Missing, out-of-range, or incompatible values are rejected, not
reweighted. The receipt records deterministic ±10% per-axis sensitivity.

## External mechanistic evidence

`--mechanistic-evidence evidence.json` accepts local records keyed by exact
route hash and audit step index. The receipt distinguishes electronic,
enthalpic, and Gibbs activation barriers, HOMO/LUMO gaps, and Fukui indices;
the unit must match that physical quantity. Computed values require a method,
charge, multiplicity, and geometry hash. They remain provenance attached to an
audited step: RENKIN does not execute DFT, mix incompatible calculations, or
use these values to override an audit verdict or reorder search results.
`project_comparable_axis` is the Rust bridge's safe ranking projection: it
requires exactly one same-step, same-quantity, same-unit, same-origin record
per route, and identical calculation contexts for computed values. It never
averages steps or mixes methods.
