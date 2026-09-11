# A Reproducible Route-to-Stock Benchmark of RENKIN, AiZynthFinder, Syntheseus, and SynPlanner

**Status:** methods-first draft. Numerical result fields are intentionally
empty until the preregistered run and integrity gates pass.

## Abstract

Retrosynthesis planners differ in reaction models, search procedures, stock
semantics, and route-export formats, making direct comparisons difficult to
interpret. We define and implement a reproducible benchmark of RENKIN,
AiZynthFinder, one pinned Syntheseus configuration, and SynPlanner. The
benchmark freezes target IDs and order, applies a common execution envelope,
audits rank-1 routes against one shared stock, and retains native planner
signals as a separate outcome. Every target produces one structured row per
arm, including failures and unmeasured states. Paired comparisons are
reported with deterministic bootstrap confidence intervals and exact McNemar
tests. This draft reports the protocol and implementation; claims about
relative performance will be added only after the 50-target, 200-target, and
500-target gates pass.

## 1. Research question

Under a frozen target cohort, common resource envelope, and independent route
audit, how do the four declared tool configurations differ in native route
discovery, strict completion to a shared stock, structural audit outcomes,
failure modes, and planning cost?

The comparison unit is a fully specified tool configuration. A project name
alone is not an experimental arm: the version, source revision, model or
template bundle, stock, search algorithm, parameters, wrapper, and runtime
image must be recorded in the configuration registry.

## 2. Experimental design

The preregistered protocol is [`four-tool-protocol.md`](four-tool-protocol.md),
and the declarative configuration is
[`configuration_registry.json`](../../benchmarks/four_tool/configuration_registry.json).
The primary endpoint is the rank-1
`strict_route_to_shared_stock` result produced by the common post-hoc audit.
The tool-native `route_found` signal is retained as a secondary outcome and
is never substituted for the primary endpoint.

The common conditions are:

- identical target IDs, canonical SMILES, order, and target sample;
- identical per-target deadline and termination grace period;
- identical CPU and memory budget, where enforcement is independently verified;
- identical row schema, status taxonomy, route parser, stock identity policy,
  and statistical implementation.

Reaction models, templates, search algorithms, native stock semantics, and
atom-mapping behavior remain native to each declared configuration. Making
these identical would define a different engine-only experiment.

## 3. Cohort and execution gates

The target population is a hash-addressed, deterministic test-derived
manifest. The run is staged:

1. 50 targets for adapter, export, stock, timeout, and validator smoke checks;
2. 200 targets for failure-taxonomy review;
3. 500 targets for the formal paired benchmark;
4. the full population only as an optional extension after the 500-target gate.

The first two stages are descriptive and do not support superiority claims.
An arm is not admitted to the formal table until its inputs, executable or
image, resource enforcement, output coverage, route export, stock conversion,
and hashes pass the feasibility gate.

## 4. Measurements and statistics

For each arm, the report includes raw numerators and denominators for native
route discovery and strict shared-stock completion, Wilson intervals, status
counts, audit outcomes, planning latency, and resource-measurement metadata.
All pairwise endpoint comparisons are joined by `target_id`. The canonical
reporter (`scripts/four_tool_report.py`) emits paired sample size, wins, ties,
left-minus-right difference, a fixed-seed percentile paired-bootstrap 95% CI,
and a two-sided exact McNemar p-value.

Timeouts, crashes, setup errors, parse errors, and not-measured rows remain
visible. They are not silently converted to route failures or removed from
the all-target denominator. A common audit that cannot evaluate an emitted
route is reported separately from a completed route that fails the stock
criterion.

## 5. Results (to be populated after the gates)

| Tool / arm | Rows | Native route found | Strict shared-stock pass | Planning p50 / p95 (ms) | Peak RSS (bytes; method) |
|---|---:|---:|---:|---:|---|
| RENKIN | `<N>` | `<n/N; 95% CI>` | `<n/N; 95% CI>` | `<p50 / p95>` | `<value; method>` |
| AiZynthFinder | `<N>` | `<n/N; 95% CI>` | `<n/N; 95% CI>` | `<p50 / p95>` | `<value; method>` |
| Syntheseus | `<N>` | `<n/N; 95% CI>` | `<n/N; 95% CI>` | `<p50 / p95>` | `<value; method>` |
| SynPlanner | `<N>` | `<n/N; 95% CI>` | `<n/N; 95% CI>` | `<p50 / p95>` | `<value; method>` |

Pairwise differences, discordant counts, and exact tests will be copied from
the generated `paired_comparisons` object. No result will be entered by hand
without its corresponding per-target JSONL and run manifest.

## 6. Interpretation and limitations

The result is a comparison of the declared configurations on the declared
cohort and stock. It is not a claim of experimental yield, human-chemist
quality, universal chemical correctness, or superiority outside the protocol.
Differences may reflect reaction-model coverage, template coverage, search
budgets, stock policy, mapping behavior, and export compatibility. Route
validity under the common structural audit is evidence about the implemented
audit contract, not a substitute for experimental verification.

Cold-start and warm planning costs are reported separately when available.
An all-target elapsed time is a deployment-cost measurement and is not called
cross-tool inference speed unless the latency protocol is explicitly matched.

## 7. Reproducibility bundle

The release candidate must contain the report source and rendered report,
configuration and target/stock manifests, per-target JSONL rows, aggregate
tables, route artifacts, image identities and dependency locks, reproduction
commands, and citation metadata. A formal result is publishable only after an
independent clean checkout reproduces the aggregate from the frozen artifact.
The publication gate is detailed in
[`doi-release-checklist.md`](doi-release-checklist.md).

The unified runner can generate the machine-readable and Markdown reports from
the same merged rows using `--report-output` and
`--markdown-report-output`. Popularity indicators, including repository star
counts, are outside the benchmark and are not reported.
