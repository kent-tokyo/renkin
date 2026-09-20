---
title: "Phase 55 performance receipt contract"
description: "How future supplementary performance measurements relate to the frozen Phase 55 r2 coverage result."
---

# Phase 55 performance receipt contract

The frozen 2026-09-16 r2 result establishes coverage for its registered
configuration. Its RENKIN rows do not contain peak RSS or time-to-first-route
receipts. Those values are **not measured** for r2 and must not be inferred
from total elapsed time, a later run, or a different tool arm.

Any performance supplement is a new run with a new run ID. It preserves the
r2 report and labels its relationship to r2 rather than combining values from
two runs into one row.

## Required registration before execution

- Exact target/cohort hash, stock, template/model assets, binaries/images,
  configuration, output rank semantics, host/architecture, CPU/RAM limits,
  network setting, warm/cold policy, timeout and grace policy.
- Both arms use the same definition of process boundary: the whole launched
  process/container, including model and stock loading. A child-only RSS
  measurement cannot be compared to a container-wide value.
- Peak RSS method, sampling interval and its undercount limitation; method
  name is stored on every row. Sampling does not claim a precise maximum it
  did not observe.
- An explicit first-route event emitted by the planner or adapter. Completion
  time, `rank=1` output, and a timeout are not substitutes. Unsolved or
  censored targets retain `time_to_first_route_ms: null` with their terminal
  status.

## Required receipts

Each row records total elapsed time, terminal status, output hash, applicable
resource observation, and timing method. Aggregates state denominator and
measurement count independently for total time, first-route latency, and RSS.
Warm and cold arms are separate; conditional latency over targets solved by
both tools is labelled conditional rather than a whole-cohort speed ranking.

The development smoke validates instrumentation, its resource boundary, and
nullability before a registered two-arm supplement. A failed or incomplete
arm is `not_measured` for the affected metric; it is not replaced by a value
from r2 or an unmatched earlier benchmark.
