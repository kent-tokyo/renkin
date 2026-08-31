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

The schema version is currently `1`. Human or LLM review can be added as a
separate judge record without overwriting deterministic audit findings.
