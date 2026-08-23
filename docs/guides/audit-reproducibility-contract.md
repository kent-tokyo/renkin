---
title: "RENKIN Bridge: Audit Reproducibility and Compatibility Contract"
description: "What the audit_manifest guarantees, the informational/standard/strict audit-policy semantics, and the compatibility rules every RENKIN Bridge adapter follows."
---

# Audit Reproducibility and Compatibility Contract

This page documents two things introduced in v0.27.0 ("Reproducible Route
Audit"): what `audit_manifest` guarantees, and the compatibility rules every
`renkin audit-route` adapter (RENKIN-native, AiZynthFinder, Syntheseus,
SynPlanner, and any future one) follows. It's the general, tool-neutral
reference; adapter-specific walkthroughs like
[Audit a Real AiZynthFinder Route](aizynthfinder-audit-demo.md),
[Audit a Syntheseus Route](syntheseus-audit-demo.md), and
[Audit a Real SynPlanner Route](synplanner-audit-demo.md) link back here
rather than repeating this content.

## Audit manifest

Every `renkin audit-route --output json` report carries an `audit_manifest`
object recording what was audited and under what conditions:

```json
{
  "audit_manifest": {
    "renkin_version": "0.34.0",
    "report_schema_version": 1,
    "source_format": "aizynthfinder",
    "source_version": null,
    "input_sha256": "sha256:...",
    "stock_sha256": null,
    "policy": "standard"
  }
}
```

- `input_sha256` hashes the decompressed route-input text actually audited
  (not the raw on-disk bytes), so a gzip vs. plain copy of identical
  content hashes identically.
- `stock_sha256` hashes the canonicalized `--stock` set actually loaded
  (order-independent), and is `null` when no `--stock` was given —
  distinct from "unknown," it means stock validation genuinely did not run.
- `source_version` is `null` whenever the source tool doesn't self-report a
  version anywhere in its route output (true for AiZynthFinder JSON today)
  — never a guess.
- `report_schema_version`/`source_format` intentionally duplicate the
  report's own pre-existing flat `schema_version`/`source_format` fields
  (kept for backward compatibility, not removed) — see the report-schema
  rule in [Compatibility rules](#compatibility-rules) below for why both
  exist.

**Determinism**: auditing the same input twice, with the same flags,
produces byte-identical output. This is a tested property
(`auditing_the_same_input_twice_is_byte_identical` in
`tests/audit_route_cli.rs`), not just a design intent.

## Audit policy

**All three policies are implemented as of v0.29.0** (Audit Policy
Profiles) — `informational`/`standard`/`strict` are all selectable via
`--policy` on the CLI, `policy=` on `renkin.audit_route()` in Python, and
the 4th argument to the WASM
[`audit_route_v2`](../api/wasm.md#audit_route_v2) export (also the
playground's Audit tab policy selector). `standard` remains the default
everywhere — omitting `--policy`/`policy`/passing `"standard"` explicitly
reproduces exactly the same verdict computation this project has always
had, unchanged.

The rule that constrains all three: **policy never hides a finding.**
Every individual finding (`AuditFinding`, per-step
`forward_validation`/`stock_validation` results) is always reported in
full, at every policy level. Only the *derived* `AuditStatus`
(`pass`/`fail`/`partial`) computation changes:

| Policy | A route with only `not_evaluable` checks (nothing outright fails) | A route with a gating finding present |
|---|---|---|
| `informational` | `partial` | `partial` (never `fail`) |
| `standard` (the default) | `partial` | `fail` |
| `strict` | `fail` (not_evaluable is not good enough) | `fail` |

`informational` is for exploratory triage where a hard stop isn't wanted;
`strict` is for pipelines that should treat "we couldn't fully verify this"
the same as "this is wrong."

## Compatibility rules

These apply to every adapter (RENKIN-native, AiZynthFinder, Syntheseus, and
any future one), not just one:

1. **"Verified against" is not "supported."** Documentation states an
   adapter is confirmed against one specific real captured tool version
   (see each adapter's own `PROVENANCE.md` under `tests/fixtures/`) — never
   phrased as broad version support inferred rather than observed.
2. **Unknown/future input fields are tolerated, never rejected.** Every
   adapter's input struct (`AzfNode` for AiZynthFinder,
   `AuditRouteInput`/`AuditRouteEntry`/`AuditRouteStepInput` for
   RENKIN-native, `SyntheseusRouteV1` for Syntheseus) derives `Deserialize`
   without `deny_unknown_fields` on purpose, so a field from a future tool
   version — or a caller's own extra metadata — is silently ignored rather
   than a parse error.
   (`unknown_extra_fields_in_renkin_input_are_tolerated_not_rejected` in
   `tests/audit_route_cli.rs` tests this directly.)
3. **A corrupted/malformed tree shape fails loud, never silently coerced.**
   Self-loops, cycles, unparseable SMILES, a non-leaf node with no
   children, an ambiguous leaf (neither a declared building block nor
   another step's target), and a handful of other structural defects each
   map to one of `AuditFindingCode`'s closed set
   (`RawOutputNotDecodable`, `MultipleOrZeroRoots`, `CycleDetected`,
   `DegenerateSelfReferentialStep`, `ChildlessNonLeaf`,
   `AmbiguousLeafStatus`, `UnparseableSmilesInRoute`, ...) — every one of
   these sets `route_tree_parseable: false` and `status: fail`. There is no
   "best-effort partial parse" path.
4. **Report schema changes.**
   `schema_version` (top-level on `AuditRouteReport`) and
   `audit_manifest.report_schema_version` both version the *report
   envelope* — the shape of `{schema_version, source_format,
   audit_manifest, summary, routes}` itself, not any one adapter's input
   format. A purely additive change (a new optional top-level or
   per-route field) does not bump this version; a breaking change to an
   existing field's meaning or an existing field's removal does. Report
   consumers should tolerate unknown keys in the response (the same
   forward-compatibility rule as rule 2, applied to output instead of
   input) but treat an unexpected `schema_version` as unsupported rather
   than guessing at the new shape.
5. **Source-tool stock claims and RENKIN's own stock verification are
   separate signals, never merged.** An AiZynthFinder route's own
   `in_stock` claim is read only as structural input (whether a node is a
   leaf); `stock_validation`'s `pass`/`fail`/`not_evaluable` verdict comes
   entirely from checking each leaf's canonical SMILES against RENKIN's
   own `--stock` file (or reporting `not_evaluable`/`stock_not_provided`
   if none was given). A leaf AiZynthFinder calls purchasable can still
   fail RENKIN's own stock check, and that disagreement is reported as
   real signal, not resolved in either direction.
6. **Adapter fixture addition runbook.** New adapters and new edge-case
   fixtures for existing adapters both follow the same pattern established
   in `tests/fixtures/aizynthfinder/v4.4.1/`:
   - Prefer a real captured output from an actual run of the real tool.
     Record exact capture command, tool version, and input/model/stock
     file SHA-256s in a sibling `PROVENANCE.md` — see that file for the
     level of detail expected.
   - If a specific edge case (e.g. a field the real tool never actually
     omits under normal operation) can't be produced from real output,
     a minimal, explicitly-labeled mutation of a real fixture is
     acceptable — but the `PROVENANCE.md` entry must say so plainly and
     state it must never be cited as evidence of real tool output. Never
     synthesize a fixture from imagined/guessed schema.
   - Trim large real captures rather than hand-authoring — keep only the
     routes/fields needed for the test, document exactly what was
     removed, and never alter a field's value within a kept route.
