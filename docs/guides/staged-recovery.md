# Staged recovery mode

Issues: [#239](https://github.com/kent-tokyo/renkin/issues/239),
[#240](https://github.com/kent-tokyo/renkin/issues/240)

## Purpose

`--search-mode recovery` is an opt-in native search mode for a completed,
unsuccessful search. It preserves the normal result when baseline search
succeeds and starts every fallback from a fresh frontier. It does not weaken
stock identity, completed-route structural integrity, aromaticity checks, or
directional element accounting.

The mode is intended for hard targets where spending more local compute is an
acceptable tradeoff. It is not the default search path and does not change
standard-mode output. The initial surface is the native CLI, Rust API, and the
formal comparison adapter; Python, MCP, and WASM retain their existing
standard/coverage contracts.

## Escalation order

Given baseline depth `D`, beam `B`, and recovery depth `R > D`:

1. baseline: depth `D`, beam `B`, score-only selection;
2. candidate-time element gate, only after a concrete
   `unaccounted_target_element` completed-route rejection;
3. diversity-reserved beam, only after baseline hits the beam limit and the
   caller supplies non-zero reserved slots;
4. depth `R`, only after baseline reaches its depth limit;
5. native recovery, after a depth-only retry fails, tries `R` with the wider
   recovery beam in one combined pass. This preserves the depth-only success
   path while recovering branches that are both deeper and crowded out;
6. each caller-supplied coverage tier, in narrow-to-broad order:
   - depth `D`, score-only beam;
   - depth `D`, diversity beam, only after that tier hits the beam limit;
   - depth `R`, diversity beam, only after that tier also reaches the depth
     limit.

`--recovery-coverage-tier <path>` may be repeated for intermediate tiers.
`--coverage-templates <path>` remains the final tier. A success or a
non-completed termination short-circuits later attempts. The optional
`--coverage-timeout-secs` deadline is created once per coverage tier and is
shared by that tier's attempts; an external caller may additionally impose a
whole-process deadline.

Beam 200 is deliberately absent. On the fixed discordant cohort, depth 5 / beam
200 added no net-new valid route and depth 6 / beam 200 added none on the
residual set.

## Audit contract

JSON output adds `search_mode: "recovery"` and a `recovery` object. Every
attempt records:

- stage and trigger;
- depth, beam width, and diversity policy/slots;
- one-based coverage tier where applicable;
- exact rule count and deterministic rule-set SHA-256;
- termination, routes found, nodes expanded, and beam/depth exhaustion;
- completed-route rejection and unaccounted-element counts;
- elapsed milliseconds.

The selected route still passes through the ordinary output adapter and the
tool-neutral route-tree, stock, and directional element-accounting validator.

## Safety and provenance boundary

Coverage files remain caller-supplied research assets and are not packaged.
The implementation does not import or translate AiZynthFinder templates,
models, or benchmark route trees. Broader corpora or template-abstraction
methods require a separate provenance, license, and leakage review.

## Fixed-cohort evidence

On the 58 v1.0.1 shared-stock targets solved by AiZynthFinder but not by the
frozen RENKIN arm, the first measured ladder used 500, 5,000, and 9,974 usable
TRAIN-derived templates, depth 5 to 6, beam 100, and 20 diversity slots.

- valid route-to-stock: 36/58 (62.1%);
- route tree, stock termination, and directional element accounting: 36/36;
- timeout, crash, or invalid output: 0;
- wall-clock sweep: 997.84 s; p50 11.85 s, p95 54.94 s, max 99.95 s;
- peak RSS: p50 267.1 MB, p95 286.3 MB, max 331.8 MB.

This is a targeted follow-up on a known discordant cohort, not a new 4,903-
target formal result or evidence of universal CASP superiority.

## Radius-zero template-abstraction follow-up

Issue #240 added a provenance-bounded, research-only way to construct an
optional final coverage tier. It re-extracts from the same fixed USPTO-50k
TRAIN reactions using rdchiral reactant radius zero, then unions only
TRAIN-frequency-filtered rules with the existing radius-one corpus.

Radius one remains the default. Radius zero and the deterministic filtered
union are explicit:

```bash
python3 scripts/extract_templates.py \
  --reactions /path/to/fixed-train-reactions.smi \
  --top 50000 --reactant-radius 0 \
  --output /tmp/radius0.smi \
  --manifest /tmp/radius0.manifest.json

python3 scripts/build_template_union.py \
  --base /path/to/radius1.smi \
  --additional /tmp/radius0.smi \
  --additional-min-count 25 \
  --output /tmp/radius1-plus-radius0-ge25.smi \
  --manifest /tmp/radius1-plus-radius0-ge25.manifest.json
```

The extraction wrapper changes only reactant-side fragment radius, delegates
all other behavior to rdchiral, and restores its temporary helper override. It
seeds both Python's standard RNG and the `numpy.random.shuffle` actually used by
rdchiral's tetrahedral-map traversal, then restores both caller states. Two
independent complete 40,008-reaction runs are byte-identical across all 1,359
emitted templates. The union keeps source paths and hashes in its manifest
rather than output comments, so identical selected rows receive the same output
hash.

The 22 residual targets were frozen as a holdout. Selection used a separate
VAL-200 cohort and preregistered TRAIN support thresholds 2, 5, 10, and 25. A
tier had to preserve every baseline positive, add at least one, produce no
malformed group, and keep candidate growth at or below 2.0x. The full union
added seven positives but grew candidates 4.39x and was rejected. `count >= 25`
added 182 rules, preserved all 167 baseline positives, added one, and grew
candidates from 17,667 to 25,404 (1.438x), so it advanced.

As the final recovery tier it recovered `L1206`, `L2879`, `L2778`, and `L3129`.
All four routes were parseable, stock-terminal, and directionally
element-accounted; post-arm verification passed for all 22 rows. The cumulative
targeted result is now 40/58, with 18 unresolved. The generated corpus is not
packaged or enabled by default, and this targeted result does not update the
frozen formal comparison.

Detailed metrics and hashes are in
[`template_abstraction_followup.md`](../../data/comparison/formal_v1.0.1/template_abstraction_followup.md).

Three narrower explicit-H abstraction prototypes generated zero rules and were
removed rather than retained as dead production tooling. Their negative results
remain recorded in Issue #240.

A later sparse-selection experiment used only TRAIN-derived rules and the
disjoint VAL-200 labels. Four radius-zero rules raised VAL positive-present
groups from 165 to 170 with 1.0026x candidate growth, but the frozen tier
recovered 0/18 targets on the already-residual holdout. It is therefore rejected
and must not be tuned again on those consumed 18 targets. See the gate and
result JSON files in `data/comparison/formal_v1.0.1/`.
