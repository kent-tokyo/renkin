# Four-tool retrosynthesis benchmark protocol

Issue: [#241](https://github.com/kent-tokyo/renkin/issues/241)

This document is the pre-registration boundary for a technical report comparing
RENKIN, AiZynthFinder, one explicitly pinned Syntheseus configuration, and
SynPlanner. It defines what may be measured and reported before the formal
target results are inspected.

## Research question

Under the same frozen target cohort, execution environment, resource budget,
and independent route-audit policy, how do the four publicly reproducible
retrosynthesis configurations differ in route generation, route-to-stock
completion, structural audit outcomes, failures, and deployment cost?

The unit of comparison is a named tool configuration, not a project name. A
configuration includes the exact source revision, model or template bundle,
stock, search algorithm, search parameters, wrapper, and container image.

## What “same conditions” means

The following are common across measured arms:

- frozen target IDs, canonical target SMILES, and target order;
- the same per-target wall-clock deadline and termination grace period;
- the same CPU and memory ceiling, host, container policy, and measurement method;
- the same output-row schema and status taxonomy;
- the same post-hoc route parser, stock identity procedure, and common audit;
- the same paired-statistics implementation and confidence-interval seed.

The following remain native to each configuration and are reported explicitly:

- reaction model or template source;
- search algorithm and search hyperparameters;
- native stock representation and lookup semantics;
- atom-mapping and route-export behavior;
- random seed behavior and determinism guarantees.

An arm is not admitted to the formal table merely because its process accepts
the same numeric limits. The runner must enforce the CPU and memory ceiling
and record the enforcement method. The current Syntheseus and SynPlanner
host-Python paths are feasibility implementations; the formal path is their
Linux bounded container recipe. Until that image is built, digest-pinned, and
smoke-tested, their resource-sensitive results remain
`candidate`/`not_measured` rather than evidence for a same-resource claim.

Giving every tool the same reaction model or template list is not required for
this end-to-end comparison and would not represent how the tools are actually
used. An engine-only comparison is a separate experiment.

## Arms

### Arm N: native public configuration

Each project is run with a documented, publicly obtainable configuration that
is fixed before the formal run. This arm answers what a user can reproduce
from the project's public distribution. It is not an engine-only comparison.

### Arm S: shared-stock configuration

Where the tool supports a controlled stock, each arm receives the same frozen
compound identity set. The stock is converted using the tool's documented
runtime representation, and the conversion is independently checked against
the source identity set. If a tool cannot consume the common stock without
changing its planner semantics, its Arm S cell is `not measured` with the
reason recorded in the manifest.

The primary common endpoint is not a tool's native `solved` flag. It is the
independently verified `strict_route_to_shared_stock` outcome: a complete,
parseable route whose leaves satisfy the declared shared-stock identity policy
and whose route structure passes the common structural checks. Tool-native
success is retained as a separate secondary field.

### Arm L: latency and deployment cost

Cold-start and warm-worker measurements are separate arms. A cold-start
measurement includes process or container initialization and model/stock load.
A warm measurement must report initialization separately from planning time.

If one tool uses a combinatorial search budget and another uses a temporal
budget, all-target elapsed time is reported as deployment cost only; it is not
narrated as a cross-tool inference-speed claim without a matched latency
protocol.

## Target population

The formal population is a frozen, hash-addressed test corpus. The current
candidate is the repository's USPTO-50K test-derived population, but the exact
source revision, row count, filtering, canonicalization, and target-list hash
must be recorded in the frozen manifest before formal execution.

The run proceeds in gates:

1. 50 targets: adapter, export, stock, timeout, and validator smoke test;
2. 200 targets: pilot and failure-taxonomy review;
3. 500 targets: formal paired benchmark;
4. full population: optional extension only after the 500-target gate passes.

The 50- and 200-target gates are not formal superiority tests. Results are
descriptive until the protocol declares otherwise.

## Configuration feasibility gate

Before a tool is admitted to a formal arm, the following must be available:

- exact version and source commit;
- license and provenance for model, template, stock, and container assets;
- a scripted batch invocation for every target;
- complete route output or an explicit failure record;
- a wrapper-enforced timeout and resource limit;
- a stable route export that the common adapter can parse;
- a reproducible stock conversion or a documented reason for exclusion;
- a manifest containing hashes for every input and executable artifact.

Syntheseus is a framework that combines search algorithms and reaction models,
so “Syntheseus” alone is not a valid experimental arm. The report must name
the exact Syntheseus version, search algorithm, reaction model, model/data
bundle, stock, and parameters. SynPlanner must likewise name the exact export
surface and version. A missing or incomparable condition is recorded as
`not_measured`; published numbers from another protocol are never substituted.

## Per-target output contract

Every arm writes exactly one row per target ID. A row contains at least:

- target ID, canonical SMILES, sample rank;
- tool configuration ID and artifact hashes;
- `run_status`: `completed`, `timeout`, `crash`, `setup_error`, or `parse_error`;
- native `route_found` and native route metadata;
- common parseability and `strict_route_to_shared_stock`;
- structural-audit statuses and directional element-accounting status;
- route hash where a route is available;
- cold-start, planning, and total elapsed milliseconds where measurable;
- peak RSS and the measurement method where measurable;
- failure reason and diagnostic details.

The native RENKIN path records peak RSS from `/usr/bin/time -l`. Docker-backed
arms sample the container's cgroup memory usage with `docker stats` and record
the largest observed value as `docker_stats_sampled`. If the runtime cannot
provide a measurement, the field remains `not_measured`; it is never reported
as zero.

Missing output rows, duplicate target IDs, malformed JSONL, unexplained
timeouts, and input-hash changes fail the arm integrity gate.

## Statistics and reporting

All tool comparisons use the same target IDs and are paired by target ID.
Report both raw numerators and denominators, paired discordant counts, absolute
differences, 95% paired bootstrap confidence intervals, and exact McNemar
results where applicable. `scripts/four_tool_report.py` is the canonical
implementation: its JSON output contains `paired_comparisons`, and its
Markdown output contains one row for every arm pair. The bootstrap uses a
fixed seed and percentile interval; the exact McNemar test is two-sided.
Do not calculate each tool's confidence interval independently and subtract it.

The report must show native and strict common-audit outcomes in separate
tables. It must also show setup errors, timeouts, crashes, and not-evaluable
routes rather than silently removing them from the denominator.

No metric in this protocol establishes experimental yield, chemical correctness
without human review, or universal superiority of any planner.

## Technical report structure

1. Motivation and evaluation question
2. Tool configurations and provenance
3. Dataset and target-freezing procedure
4. Common execution and audit protocol
5. Statistical analysis plan
6. Results by arm and tool configuration
7. Failure and disagreement analysis
8. Reproduction instructions and artifact hashes
9. Limitations and non-claims
10. Conclusions restricted to the measured protocol

The release artifact must contain the report source, rendered report, frozen
manifests, per-target JSONL, aggregate tables, reproduction scripts, container
or environment specification, image identities, and a citation file. The
integrated runner copies the verified image identities and their hash into its
run manifest. The formal report is not
ready for publication until an independent clean checkout reproduces the
aggregate results from the tagged artifact.
