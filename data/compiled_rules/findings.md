# Compiled rule set local gate

## Contract

`PreparedRuleSet` owns one immutable `chematic::rxn::PreparedReaction` per
directly applicable SMIRKS variant and reuses it for a search or
`CandidateProposalContext`.

- `RetroRule` and public string-based `apply_retro` remain unchanged.
- Empty-SMIRKS graph rules keep their existing Rust handlers.
- `[#N]` rules compile every validated concrete variant; unsupported variants
  fail closed.
- Conflicting template IDs reuse state only when raw SMIRKS agrees.
- Result order, precursor contents, aromaticity checks, and deduplication stay
  unchanged.
- Prepared reactions are immutable and `Send + Sync`.
- SSSR ring perception is computed once per target and shared by all prepared
  templates instead of being recomputed by each VF2 call.

Ring-context match enumeration/application and coverage-mode Stage 1/2 use the
same prepared state. Coverage frontiers, closed sets, and candidate caches
remain independent to preserve the pre-registered Stage-2 fresh-search
semantics.

## Measurement

Date: 2026-09-04. The explicit ignored corpus gate compared the old and new
paths for the first 10 USPTO-50k test targets and all 500 checked-in templates
(5,000 applications). Every ordered precursor result was identical.

Optimized local test build:

| operation | elapsed |
|---|---:|
| compile 500-template ruleset once | 0.007574 s |
| legacy template application | 0.777056 s |
| shared ring perception for 10 targets | 0.000960 s |
| prepared template application | 0.040014 s |

Template application was about **19.4x faster**. Including one-time setup and
shared ring perception, this bounded gate was about **16.0x faster**. This is a
local microbenchmark, not a full-search or competitor claim.

The feature-gated integration test separately proves that eight direct legacy
executions produce eight parses, while a multi-expansion search and coverage
Stage 1/2 each share one ruleset compilation.

## Fixed-VAL full-search remeasurement

Date: 2026-09-04. The current release build was measured on the frozen,
disjoint 200-target Phase B.2 VAL cohort. The authoritative binary SHA-256 was
`e41b7015e3e83a73710f2d8d2a0c81bb4d9e1ab3be22cb0a5cbd9f5242718f67`.
Both arms used depth 5, beam width 100, one route, shared stock, and no
reranker:

- Arm A: standard search with 500 templates and a 150 s external timeout.
- Arm C: native one-process coverage search, 500 -> 2,000 templates, a 600 s
  cooperative Stage-2 timeout, and a 750 s external safety cap.

Arm C was deliberately resumable: the first 10 rows came from the smoke run
and the remaining 190 were appended by the full run. Both manifests record the
same binary and input hashes. Consequently the aggregate's
`wall_clock_total_sweep_s` covers only the resumed 190-row invocation; the
table uses the sum of all 200 per-target wall times instead.

The historical cohort has `source_line_number: null`, which the hardened
current sample loader rejects. `val_sample_disjoint_200_compat.jsonl` changes
only that field, deriving it from each unchanged `target_id` suffix. The
ordered `canonical_smiles`, `sample_key`, `sample_rank`, and `target_id` fields
are identical for all 200 rows. The original cohort SHA-256 remains
`8725031a31e298c50eacba41152c4b3a634d591b39f219975c82ebe5a462bbfc`;
the compatible measurement copy is
`fe05925c7a442933781da1946c96bd2c4f538b18bb4920e6aa9a5d1b0a3ada31`.

### Current paired result

| metric | Arm A: 500 | Arm C: 500 -> 2,000 | C vs. A |
|---|---:|---:|---:|
| route to configured stock | 21/200 (10.5%) | 23/200 (11.5%) | +2, +1.0 pp |
| solved-target regressions | - | 0 | exact superset |
| timeout / crash / invalid | 0 / 0 / 0 | 0 / 0 / 0 | - |
| Stage-2 invocation | - | 179/200 (89.5%) | - |
| p50 wall time | 2.061 s | 7.356 s | 3.57x |
| p95 wall time | 11.440 s | 41.100 s | 3.59x |
| maximum wall time | 43.201 s | 123.566 s | 2.86x |
| cumulative per-target wall time | 731.896 s | 2,620.099 s | 3.58x |
| p50 peak RSS | 17.63 MiB | 48.69 MiB | 2.76x |
| p95 peak RSS | 24.30 MiB | 62.64 MiB | 2.58x |
| maximum peak RSS | 37.84 MiB | 86.88 MiB | 2.30x |

Arm C preserved every Arm-A solve, with the same normalized route SHA-256 for
all 21, and added two element-accounted, configured-stock routes
(`uspto50k_val#L1728` and `uspto50k_val#L214`). This is full-search evidence
that sharing prepared rules across coverage stages preserves the Stage-1
result. It is not merely a template-application microbenchmark.

Against the old Phase B.2 measurements, current Arm A is 6.93x faster at p50,
5.62x faster at p95, and 5.57x faster in cumulative wall time. Current Arm C
is 11.37x faster at p50, 8.94x faster at p95, and 9.27x faster in cumulative
wall time. The old Arm-C 750 s maximum fell to 123.57 s, and all six historical
Stage-1/Stage-2 timeout outcomes completed. Retaining compiled reactions costs
memory: current Arm-C p95 RSS is 62.64 MiB versus 25.78 MiB in the historical
merged arm.

These cross-date speedup ratios support the optimization but do not isolate
its causal effect: the current dirty-tree binary also contains later stock and
route-correctness changes, and the host load differs from the historical run.
The same-source 5,000-application gate above remains the isolated compile-path
comparison. The current Arm-A/Arm-C pair is the controlled evidence for the
product-level coverage trade-off.

### Coverage-gate consequence

The current Arm C fails the original Phase B.2 `>= +3 pp` coverage gate: it
adds only +1.0 pp while costing 3.59x p95 latency and 2.58x p95 RSS. The old
result was 50/200 versus 33/200 (+8.5 pp); the current result is 23/200 versus
21/200 (+1.0 pp). Current solves are an exact 23-target subset of the old
50-target solved set: 23 overlap, 27 historical solves disappear, and there
are no new solves relative to that old set. Of the 27 lost historical outputs,
18 had `unaccounted_target_element`, eight were accounted, and one route tree
was unparseable under the historical adapter. Because unrelated correctness
fixes are present, this route-set change must not be attributed to compile-once
without a same-source legacy-path A/B build.

Artifacts:

- `full_search_val_200_standard_{rows,aggregate,manifest}.json[l]`
- `full_search_val_200_{rows,aggregate,manifest}.json[l]`
- `full_search_val_smoke_{rows,aggregate,manifest}.json[l]`
- `val_sample_disjoint_200_compat.jsonl`

### Stage-2 beam re-evaluation

To separate Stage-2 beam crowd-out from template-count effects, the same
fixed 200-target cohort was rerun with Stage 1 unchanged at beam 100 and a
Stage-2-only beam override. The new CLI/Python option is
`--coverage-beam-width` / `coverage_beam_width`; omitted means the historical
same-width behavior and `0` means unlimited.

| arm | solved | vs. current 500 -> 2,000 | p50 | p95 | RSS p95 | timeout/crash |
|---|---:|---:|---:|---:|---:|---:|
| current coverage, beam 100 | 23/200 (11.5%) | - | 7.356 s | 41.100 s | 62.64 MiB | 0/0 |
| Stage 2 beam 200 | 25/200 (12.5%) | +2, +1.0 pp | 11.822 s | 56.604 s | 71.22 MiB | 0/0 |
| Stage 2 beam 500 | 28/200 (14.0%) | +5, +2.5 pp | 40.551 s | 197.532 s | 93.70 MiB | 0/0 |

Both wider-beam arms were exact supersets of the current 23 solves. Beam 200
added `uspto50k_val#L1887` and `uspto50k_val#L195`; beam 500 additionally
added `uspto50k_val#L2013`, `uspto50k_val#L2983`, and `uspto50k_val#L4906`.
The beam-500 arm therefore improves coverage materially but still misses the
old `+3 pp` gate by one target (needs at least 29/200 versus the 23/200
baseline). Its p95 latency is 4.81x and p95 RSS 1.50x the current coverage
arm, so it is not a formal release candidate under the existing gate.

Artifacts:

- `full_search_val_200_cb200_{rows,aggregate,manifest}.json[l]`
- `full_search_val_200_cb500_{rows,aggregate,manifest}.json[l]`
