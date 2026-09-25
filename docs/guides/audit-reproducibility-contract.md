---
title: "RENKIN Route Audit Contract"
description: "Reproducible local route audit, policy semantics, and compatibility rules for RENKIN Bridge adapters."
---

# Route audit contract

`renkin audit-route` evaluates a supplied route without treating it as trusted.
It accepts RENKIN, AiZynthFinder, Syntheseus, and SynPlanner route shapes, then
reports topology, structure, stock, forward-basis, and evidence findings.

```bash
renkin audit-route route.json --format auto --output json
```

Use explicit `--stock`, `--private-stock`, and `--stock-policy` inputs when
those checks matter. No supplied stock is reported as missing evidence, never
as a successful stock check.

## Manifest and deterministic output

Every JSON report includes an `audit_manifest` that identifies the RENKIN
version, report schema, source format/version when declared, input hash, stock
hash when applicable, and selected policy. The same input, stock, rules, and
options produce byte-identical output. Pin all of them when a review must be
repeated.

The manifest records what was observed; it is not a laboratory record. A
structural pass does not establish a reaction's conditions, yield, safety, or
regulatory acceptability.

## Policies

All findings are retained under every policy. Only the derived route status
changes:

| Policy | `not_evaluable` only | Gating finding |
| --- | --- | --- |
| `informational` | `partial` | `partial` |
| `standard` (default) | `partial` | `fail` |
| `strict` | `fail` | `fail` |

This separation prevents a caller from hiding an adverse finding by choosing a
more permissive policy.

## Input and adapter rules

`--format auto` detects only unambiguous supported shapes. Use an explicit
format when a source is known. Malformed, oversized, unsupported, or ambiguous
inputs fail loudly rather than being normalized into a guessed route.

Bridge imports and exports carry a field-level loss report. A field is marked
`preserved`, `normalized`, `inferred`, `dropped`, or `unsupported`; importers
must not turn loss into a silent success. Source node IDs, reaction provenance,
stock provenance, and conditions remain source-scoped evidence.

An atom-mapping receipt is diagnostic only. It may report duplicate, orphan,
or parent/child producer-consumer map labels, but it never completes a map or
changes a route's status by itself.

## Local review workflow

1. Preserve the original route export and record its hash.
2. Audit with explicit format, stock, policy, and optional evidence assets.
3. Review `fail`, `partial`, and `not_evaluable` findings separately.
4. Retain the JSON report, manifest, source assets, and policy hash together.

Use [private stock policy](private-stock-policy.md) for procurement-aware leaf
decisions, [chemical review](chemical-review-rubric.md) for reason-coded
review, and [evidence-carrying interchange](evidence-carrying-interchange.md)
for cross-tool transport.
