# RENKIN 1.0.3 formal native benchmark

Date: 2026-09-08  
Protocol: Issue #66 shared-stock comparison, frozen 200-target validation cohort  
Comparator: AiZynthFinder 4.4.1 result set from the same cohort

## Result

| Metric | RENKIN | AiZynthFinder | Difference |
|---|---:|---:|---:|
| Native `route_found` | 134/200 (67.0%) | 134/200 (67.0%) | 0.0pp |
| Strict validated route to configured stock | 134/200 (67.0%) | 123/200 (61.5%) | +5.5pp |

The native route-found result is a tie on this fixed cohort and configuration;
it is not evidence of universal superiority. The strict metric is a paired,
post-hoc validation result under the configured shared stock. Its discordant
pairs were AiZynthFinder-only 20 and RENKIN-only 31, with an exact McNemar
p-value of 0.1608 and bootstrap 95% CI of [-1.5pp, +12.5pp]. The native
route-found discordant pairs were 25 versus 25, exact McNemar p=1.0, with
95% CI [-7.0pp, +7.0pp].

## Execution contract

- Cohort: `data/phase_b1_frontier/val_sample_disjoint_200.jsonl`
- Cohort SHA-256: `8725031a31e298c50eacba41152c4b3a634d591b39f219975c82ebe5a462bbfc`
- RENKIN stock: compiled `.rstock` union, exact canonical identity
- Stock SHA-256: `83db40b1d4463c1300ed005adf91989252c341133707a5525347d4d35bb73aff`
- RENKIN templates: `data/templates_extracted_5000.smi`
- Templates SHA-256: `7e0d105d736ffc264c1e286ff4fc7497ad913b61b4fa15555afc465dca05231e`
- Search: depth 5, beam 100, max routes 1
- Native recovery: depth 6, beam 200, shared 30-second budget
- Recovery policy: `native`, with depth-only retry before combined depth+beam retry
- RENKIN Cargo.lock SHA-256: `0821dd7205ff06b0d190421937f07b265ae4fc2be16d7daba61122fae06f1443`

The RENKIN run completed all 200 targets with zero timeout and zero crash
rows. Total RENKIN wall-clock sweep time was 3,145.4 seconds; per-target
total elapsed p50 was 1.60 seconds and p95 was 30.23 seconds. These latency
figures are reported for reproducibility, not as a cross-tool speed claim:
the two planners use different search-budget semantics and execution
environments.

## Reproducibility artifacts

- `renkin.jsonl` — one result row per target
- `renkin_aggregate.json` — aggregate metrics
- `renkin_manifest.json` — command, hashes, binary, host, and resource metadata
- `paired_stats_shared_stock.json` — paired statistics
- `paired_table_shared_stock.json` — per-target paired outcomes

The AiZynthFinder rows and aggregate used for pairing are retained at
`../formal_200_20260908/aizynthfinder.jsonl` and
`../formal_200_20260908/aizynthfinder_aggregate.json`.

## Claim boundary

This is a formal, reproducible benchmark result for the stated cohort,
stock, templates, search budgets, and tool versions. It does not establish
that RENKIN is universally more accurate, faster, or chemically superior to
AiZynthFinder. The strict result is also not a human-chemist route-quality
assessment.
