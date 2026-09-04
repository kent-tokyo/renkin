# Route Feasibility Diagnostics

`synthesizability::diagnose_route_feasibility()` converts an existing
`RouteAssessment` and its matching `Route` into a transparent, versioned report.
It does not rerun search or change route ranking.

## Report contents

The report keeps the underlying facts separate:

- completion to independently verified configured stock;
- structural, directional target-element-accounting, and forward-validation
  status;
- route depth and the existing, already-disclaimed route-cost heuristic;
- evidence and evidence-backed condition coverage;
- every configured hard failure and policy validation gap;
- deterministic per-step limiting reasons; and
- explicit missing-information reason codes.

There is no aggregate numeric feasibility score. The categorical `disposition`
only summarizes whether configured checks rejected the route, whether review is
still needed, or whether the available checks support it. It is not a predicted
probability of laboratory success. A serialized report carries this
interpretation boundary in its `interpretation` field.

The function recomputes the Synthesizability Kernel route ID and rejects a
mismatched `(target, route, assessment)` tuple, preventing diagnostics from
being silently attached to the wrong route.

## Research, licence, and patent boundary

The distinction between finding a stock-terminated route and assessing its
practical executability is motivated by J. Choe et al., “Retrosynthetic
crosstalk between single-step reaction and multi-step planning,” *Journal of
Cheminformatics* 17, 130 (2025),
<https://doi.org/10.1186/s13321-025-01088-z>. The article is distributed under
CC BY-NC-ND 4.0. RENKIN does not copy or adapt its text, figures, tables,
datasets, source code, metric formula, or model; this module independently
projects fields already produced by RENKIN.

The implementation deliberately excludes fragment-frequency dictionaries,
molecular-descriptor aggregation, learned synthetic-accessibility models,
fragment-density calculations, rewards/penalties, and a combined synthetic-
accessibility score. Those exclusions keep it outside the implementation shape
described in the reviewed independent claims of WO2021229454A1 and the pending
US application US20260100252A1. This is only a preliminary technical screen,
not legal advice or an FTO clearance; claim status, family members, territory,
and commercial use still require qualified counsel when relevant.
