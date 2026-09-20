---
title: "Trusted route operations"
description: "The v1.0.9 contract for locating route-audit findings and declaring safe public execution boundaries."
---

# Trusted route operations

This is a follow-up to the evidence chain shipped in v1.0.8. It does not add
a planner, an OCR model, a second chemistry parser, or a new route schema. It
makes two existing boundaries explicit: where an audit finding occurred, and
what a caller may safely ask a public surface to do.

## Binding inventory

| Surface | Route audit | Input and resource boundary | Cancellation / network |
|---|---|---|---|
| CLI | `renkin audit-route` reads a local plain or gzip JSON file | bounded audit input reader; optional local stock and O7 sidecars | process owner may interrupt; audit initiates no network access |
| Python | `renkin.audit_route(content, ...)` accepts decoded in-memory text | shared text, stock-line, nesting, and token limits | synchronous call; no cooperative cancellation; no network |
| WASM | `audit_route_v2(content, format, stock, policy)` | same shared audit limits; plain in-memory JSON only | no core cooperative cancellation; playground terminates and respawns its Worker; no network |
| MCP | generated-route tools only; it does **not** expose arbitrary external route import/audit | each public tool validates its declared input schema | stdio request has no cancellation contract; no network contract is inferred from MCP itself |

An audit verdict is shared by CLI, Python, and WASM only for the common
in-memory content/format/policy contract. Filesystem gzip support, O7
sidecars, process cancellation, and MCP tool availability are not silently
claimed to be cross-binding features.

## Finding location v1

`AuditFinding` keeps the existing `code`, `severity`, and optional `node`.
It also carries optional location fields:

```json
{
  "code": "forward_reaction_not_reproduced",
  "severity": "gating",
  "node": "CCO",
  "occurrence_path": [1, 0],
  "step_index": 2
}
```

- `occurrence_path` is the zero-based child-index path from the normalized
  route root. It identifies one occurrence even when the same canonical
  SMILES appears multiple times.
- `step_index` is the preorder index in `AuditReport.steps` for a
  decomposing node. It is absent for leaf-only findings.
- Both are absent for a route-wide finding or a parse failure with no trusted
  normalized tree. A missing location is therefore not guessed from an
  external tool's node ID.

The path identifies a normalized occurrence, not a source atom-map or an
experimental reaction identity. Atom-map labels from external planner exports
can be local to one reaction. `target_element_accounting_status` remains a
directional heavy-element check, not a mass/charge balance or proof of
reaction feasibility.

## External planner fixtures

The committed `tests/fixtures/synplanner/v1.6.0/` captures real
`write_routes_json` output with provenance, source artifact metadata, and
hashes. It is the supported SynPlanner fixture baseline. The adapter preserves
confirmed source route/tree identifiers where available and records loss in
canonical interchange; it does not invent mapping, missing species, or
conditions.

SynPlanner v1.7.0 route-tree JSON compatibility is verified against upstream
route 58 from the release's own 18-route test file. Its intake identity is
fixed: release `v1.7.0` (2026-08-25), annotated tag
`c581c67143a09c0dbffd3c5d019412ac54766e75`, commit
`c46c84e7b3688a0175d87e7ba7e0302d139171a9`, and MIT license file
`c9ce472807ae4d1e009d4e8ab2399802f32eb1ff`. The fixture preserves the upstream
blob and extracted-route hashes plus the MIT text. Both reaction steps parse,
normalize, and forward-replay successfully. The release has no attached model
export artifact, so this does not establish compatibility with every model,
route, or the separate CLI wrapper/bundle format. The still-open
protecting-group revision PR is experimental upstream work and is not a
RENKIN supported input contract.

## Browser operation contract

The playground has one WASM Worker. A search and an audit never run
concurrently. For either operation, Cancel or an elapsed browser budget rejects
the pending request, terminates that Worker, loads a fresh module, and only
then re-enables the next request. This is termination-based cancellation, not
cooperative WASM cancellation. A reply from the terminated worker cannot be
applied to a later request.

The release-candidate browser check cancels a 100,000-route audit, waits for
the fresh Worker, then completes the normal failing example. The cancelled
reply did not replace the later result, and the browser console contained no
route-input log entry. This verifies the playground orchestration; it does not
turn the synchronous WASM audit function into a cooperatively cancellable API.

The exported WASM `capabilities()` JSON is the machine-readable source for
the WASM module's accepted formats, policies, limits, network stance, and
cancellation limitation. It describes the module, not the native CLI/Python
or MCP surfaces. The CI Node/WASM quickstart derives over-limit calls from
that payload, so a changed number cannot silently drift from the export that
enforces it.

The installed Python extension exposes its own `renkin.capabilities()` payload.
It uses the native search limits rather than the tighter browser limits and
also describes forward prediction, caller-selected local-path access, and the
coverage Stage-2-only timeout. It does not imply cooperative cancellation for
ordinary synchronous Python search or audit calls.

The root CLI exposes the corresponding contract with:

```bash
renkin capabilities
```

Its payload includes CLI-only gzip audit input and `interchange` support. It
does not claim that every CLI subcommand is read-only: commands which create
stock or other artifacts remain explicitly command-specific. The separate MCP
server advertises callable tools through the standard `tools/list` discovery;
it does not accept arbitrary external route import as an MCP tool.
