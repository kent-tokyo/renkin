# Deterministic chemical review rubric

`renkin audit-route` can emit an optional `chemical_review` block:

```bash
renkin audit-route route.json --chemical-review --output json
```

The rubric is deliberately an evidence boundary, not a route-quality score.
It reports a stable `rubric_version`, `judge_id`, dimension, reason code,
severity, and one of `pass`, `review`, or `not_evaluable`.

The deterministic judge currently covers structural audit, configured stock,
and declared forward replay. Selectivity, experimental conditions, substrate
scope, protecting-group compatibility, and strategic route quality are
`not_evaluable` unless those facts are present in a future evidence-carrying
interchange record. The command never fabricates conditions or turns missing
evidence into a chemical failure.

The block is opt-in and absent from the default JSON report, preserving the
existing `audit-route` output contract. Human or LLM review may be layered on
later by recording its judge identity, model/version, rubric version, verdict,
and confidence separately from this deterministic result.
