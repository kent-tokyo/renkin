# RENKIN v1.0.0 Formal Route-to-Stock Comparison

## Status

**Statistical superiority: PASS**
**Formal publication gate: HOLD**

This report covers the preregistered paired shared-stock comparison of
RENKIN v1.0.0 and AiZynthFinder 4.4.1 on all 4,903 frozen USPTO-50k test
targets.

## Primary endpoint

`route_to_shared_stock` requires `route_found`, a parseable route tree, and
all route leaves in the exact configured shared stock.

| Arm | Primary successes | Rate |
|---|---:|---:|
| RENKIN v1.0.0 | 577 / 4,903 | 11.77% |
| AiZynthFinder 4.4.1 | 200 / 4,903 | 4.08% |

The paired RENKIN-minus-AiZynthFinder difference is **+7.689 percentage
points**, with the fixed 10,000-resample paired-bootstrap 95% CI
**[+6.812, +8.566] percentage points**. The CI is strictly above zero.

Discordant paired outcomes were RENKIN-only **437** and AiZynthFinder-only
**60**.

## Integrity and validation gates

| Check | Result |
|---|---|
| Frozen target population and input hashes | PASS |
| Exact target-set coverage, 4,903 rows per arm | PASS |
| AiZynthFinder arm verification | PASS |
| RENKIN arm verification | FAIL / HOLD |

The RENKIN verification report identifies two rows where
`route_found=true` has no `normalized_route_sha256`. Both rows are also
non-parseable route trees and therefore are not primary successes:

- `uspto50k_test#L1394` — `multiple_or_zero_roots`
- `uspto50k_test#L1080` — `unparseable_smiles_in_route`

These rows remain in the raw result set and are not silently repaired or
removed. Because the preregistered formal gate requires the route-validation
and integrity checks to pass, the overall formal publication gate remains on
hold despite the positive paired confidence interval.

### Targeted rerun

Both targets were rerun separately with the frozen RENKIN v1.0.0 binary,
depth 5, beam width 100, the same shared stock and template bundle, and the
same timeout settings. Both reproduced the original behavior: raw
`route_found=true`, route tree non-parseable, and
`route_to_configured_stock=false`. This confirms the two findings are
deterministic validation failures rather than a transient output truncation.

The rerun artifacts are `rerun_targets.jsonl`,
`formal_issue_rerun.jsonl`, and `formal_issue_rerun_aggregate.json`.

### v1.0.1 candidate remediation check

After fixing zero-step route normalization and rejecting retro-generated
non-aromatic bracket atoms whose explicit-H count conflicts with surviving
graph valence, the same two targets were rerun against the v1.0.1 candidate.
Both completed without crash or timeout and produced parseable,
stock-terminated route trees (`route_tree_parseable=true`,
`all_leaves_in_configured_stock=true`). The artifacts are
`formal_issue_rerun_v102.jsonl` and
`formal_issue_rerun_v102_aggregate.json` (the filename records the local
iteration name; the rows identify the tested binary as 1.0.0 because this
targeted check preceded the release metadata bump).

This targeted remediation check does not alter the frozen v1.0.0 arm or its
HOLD verdict. A full 4,903-target rerun and complete verification are required
before publishing a formal PASS claim for the corrected binary.

## Reproduction artifacts

- `renkin_shared_stock.jsonl`
- `aizynthfinder_shared_stock.jsonl`
- `renkin_shared_stock_aggregate.json`
- `aizynthfinder_shared_stock_aggregate.json`
- `renkin_verification.json`
- `aizynthfinder_verification.json`
- `paired_stats_shared_stock.json`
- `paired_table_shared_stock.json`
- `preflight.json`
- `renkin_shared_stock_manifest.json`
- `aizynthfinder_shared_stock_manifest.json`

This result is evidence of superiority on the declared route-to-configured-
stock protocol only. It is not a claim of universal CASP superiority,
experimental yield, or replacement of a retrosynthesis planner.
