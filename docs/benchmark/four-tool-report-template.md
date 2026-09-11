# Four-tool retrosynthesis benchmark report

This document is a report template for the benchmark protocol in
[`four-tool-protocol.md`](four-tool-protocol.md). It is intended to be
technical evidence that can later be expanded into a paper. Empty cells are
not zeroes: a missing measurement must be reported as `not_measured` with a
reason in the run manifest.

## Research question

Under a frozen target cohort and declared resource budget, how do RENKIN,
AiZynthFinder, Syntheseus, and SynPlanner compare on (1) native route
discovery, (2) rank-1 route validity against the common stock, and (3)
wall-clock/resource cost?

The comparison does not claim that a framework's native model, templates,
search algorithm, or stock semantics are identical. Those are part of each
tool configuration and are reported explicitly. “Same conditions” means the
same target rows, order, timeout, grace period, CPU/memory budget, output
schema, post-hoc audit, and statistical procedure.

## Configuration and provenance

| Item | Value |
|---|---|
| Registry | `benchmarks/four_tool/configuration_registry.json` |
| Target manifest | `<path and SHA-256>` |
| Shared stock | `<path and SHA-256>` |
| Runner commit | `<commit>` |
| Host/container | `<OS, image digest, CPU, memory>` |
| Timeout / grace | `<seconds> / <seconds>` |
| Formal target count | `<N>` |

For every arm, record the exact package/version or image digest, source
commit, checkpoint/template hash, Python/Rust version, and effective search
parameters. A paper table should link to the immutable manifest rather than
relying on a package name alone.

## Primary outcomes

The primary common outcome is `strict_route_to_shared_stock` for the rank-1
available route. The native `route_found` signal is a separate outcome and
must not be substituted for it.

| Tool / arm | N rows | Native route found (n/N) | Rank-1 common audit pass (n/N measured) | Not measured | Median planning ms |
|---|---:|---:|---:|---:|---:|
| RENKIN | | | | | |
| AiZynthFinder | | | | | |
| Syntheseus | | | | | |
| SynPlanner | | | | | |

Report the Wilson 95% interval next to every rate. For paired comparisons,
use `target_id` as the pairing key and state the treatment of timeout,
crash, setup, and parse-error rows before looking at the result.

## Failure and coverage accounting

Include counts for `completed`, `timeout`, `crashed`, `setup_error`,
`parse_error`, and `not_measured` by arm. A route emitted by a tool but not
parseable by the common bridge is not a common-stock failure; it is a separate
coverage/measurement limitation and must remain visible.

## Interpretation boundary

Do not infer chemical superiority from a single aggregate rate. Discuss the
target distribution, route depth, template/model coverage, and stock policy.
Do not compare a native-stock result with a shared-stock result in one primary
table. Do not report repository popularity metrics such as star counts as a
result of this benchmark.

## Reproduction commands

```text
# Freeze and validate the manifest
python3 scripts/validate_four_tool_registry.py \
  benchmarks/four_tool/configuration_registry.json

# Generate the machine-readable report after all rows are collected
python3 scripts/four_tool_report.py <records.jsonl> --output <report.json>
```

The raw per-target JSONL, runner log, configuration registry, target/stock
manifests, artifact hashes, and report JSON are the reproducibility bundle.
