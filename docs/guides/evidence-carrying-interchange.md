# Evidence-carrying route interchange

Export a versioned canonical route record together with the audit result:

```bash
renkin audit-route route.json --interchange --output json
```

The optional `route_interchange` block carries the normalized route hash,
canonical step IDs, target and precursor SMILES, forward-replay status and
evidence basis, audit findings, and stock/policy provenance when supplied.

Current adapters do not retain source-tool versions or original node IDs in
their confirmed input contracts. Those fields are therefore `null`, never
guessed. `canonical_node_id` is deterministic and derived from the normalized
route ID and step index. The schema explicitly records whether the original
reaction representation was retained; a replay status is not silently
promoted into a preserved SMIRKS or a chemical-quality claim.

The schema version is currently `1`. Human or LLM review can be added as a
separate judge record without overwriting deterministic audit findings.
