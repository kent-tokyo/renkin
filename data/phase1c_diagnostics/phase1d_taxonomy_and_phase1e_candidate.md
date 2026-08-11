# Comp-Phase1D taxonomy + Phase1E candidate pre-registration

Source data: `data/phase1c_diagnostics/beam{100,200,300}.jsonl` (the clean,
sequential rerun -- one arm at a time, no CPU contention; see
`L1541_candidate_narrative.md`'s methodology note for why the earlier
parallel run, preserved at `beam{100,200,300}_parallel_contended.jsonl`,
was discarded). Conservative policy, shared_stock, depth=5, same 100
targets as `data/comparison/sample_full_sorted.jsonl`'s first 100 by
`sample_rank`. Binary `renkin-crowdout-diag` @ `5d03554` (post Phase1A+1B,
byte-identical to the v0.21.1 base without the new flags).

## Phase1D: failure taxonomy (100 targets)

Classified by `scripts/phase1d_taxonomy.py` -> `data/phase1c_diagnostics/taxonomy.jsonl`:

| category | n | meaning |
|---|---:|---|
| `H_unknown_no_ground_truth_route` | 70 | never solved at any beam width; no ground-truth route exists to attribute the loss to a specific candidate |
| `solved_all_beams` | 16 | route found at beam 100, 200, and 300 |
| `F_timeout_partial` | 10 | at least one beam width timed out (5 at beam≥200, 5 at beam=300 only); the beam(s) that did complete were still unsolved |
| `beam_limited_monotone_gain` | 3 | unsolved at low beam, solved at higher beam(s), never reverts (L984, L1640, L4092) |
| `D_non_monotonic_crowd_out` | 1 | **L1541**: `route_found` = False/True/False across beam 100/200/300, all three runs `run_status: completed` (31.3s/41.7s/53.1s) |

`D_non_monotonic_crowd_out` matching exactly `uspto50k_test#L1541`, with
all three beams `completed` (not masked by a timeout), is the key
cross-check between this taxonomy and the independently-collected
candidate-level trace in `L1541_candidate_narrative.md` -- they agree.

Categories `A_proposal_absent` and `E_stock_missing_suspected` (see
`phase1d_taxonomy.py`'s docstring) have **zero** members in this sample --
confirmed directly: every one of the 70 `H_unknown` targets attempted a
nonzero number of rule applications and produced at least one
stock-terminal candidate at some beam width. The failure mode for all 70
is never "no stock" or "no proposals" -- it is "the pieces exist
individually but no complete tree from target to stock was ever
assembled within the depth=5 / beam / 150s budget."

### Two aggregate checks against the L1541 duplicate-state mechanism

`L1541_candidate_narrative.md` precisely characterizes L1541's failure as
duplicate-state crowd-out: the same disconnection independently re-pushed
onto the heap many times from different partial-route derivation paths,
most copies evicted, none of the survivors happening to be the one that
expands into the terminal stock-reaching step. Before picking a Phase1E
candidate around this mechanism, two aggregates test whether it
generalizes beyond the 1 target it was diagnosed on:

**Cross-template dedup collapse ratio** (`candidates_after_cross_template_dedup / candidates_generated_before_dedup`, averaged across all 3 beams per target):
- `H_unknown` (n=70): mean 0.837, median 0.838
- `solved_all_beams` (n=16): mean 0.857, median 0.847

Indistinguishable. Whatever fraction of proposals collapse into
duplicates at dedup time, it happens at essentially the same rate whether
the target ultimately solves or not.

**Evictions per beam-prune invocation** (`candidates_evicted_total / beam_prune_invocations`):
- `H_unknown`: mean 36.52, median 35.89
- `solved_all_beams`: mean 37.29, median 32.40

Also indistinguishable -- unsolved targets are not experiencing
systematically worse beam-crowding pressure per prune event than solved
ones.

**What does differ, sharply**: `rules_attempted_total` at beam=100:
- `H_unknown`: mean 84,684 / median 86,328
- `solved_all_beams`: mean 18,975 / median 15,576

Unsolved targets attempt ~4.5-5.5x more rule applications than solved
ones. This is the actual discriminating signal in this sample: the 70
`H_unknown` targets are not failing because of duplicate-state crowd-out
or elevated beam-crowding severity -- they are simply much larger search
problems (more template matches at every node, consistent with more
complex/larger target molecules), and the search exhausts its
depth/beam/time budget building a much bigger tree without ever closing
one complete route, even though every individual branch can independently
reach stock.

### Implication for scope

The L1541 duplicate-state mechanism is real, precisely evidenced, and
worth fixing -- but the aggregate evidence bounds its reach to the
non-monotonic-crowd-out + beam-limited-monotone-gain population: **at
most 4 of the 100 targets** (L1541, L984, L1640, L4092) show any
sensitivity to beam width at all. The other 70 show no such sensitivity
across the 100-300 range tested and are not distinguishable from the
solved population on either dedup-collapse or eviction-pressure -- a fix
narrowly targeting duplicate-state crowd-out cannot plausibly convert a
meaningful fraction of the 70, and should not be expected to reach
Phase1E's `route_to_configured_stock ≥ baseline+2` gate on its own even if
it perfectly fixes all 4 beam-sensitive targets (a fix that converts all 4
already only reaches +4, and 3 of those 4 already solve by beam=300, so
the realistic beam=100 gain from fixing pure crowd-out is bounded near
+1..+4, not clearly reaching +2 either way with confidence, and entirely
orthogonal to the 70-target majority).

## Phase1E: candidate pre-registration

Of the five candidates named in the program spec (same-parent
cross-template precursor dedup / diversity-aware beam /
logical-template-normalized candidate quota / adaptive beam by branching
factor / deterministic reranker-assisted ordering), the evidence above
argues against leading with the literal duplicate-dedup candidate: it
targets exactly the mechanism just shown not to generalize.

**Selected candidate: adaptive beam by branching factor.**

Rationale: the discriminating signal (unsolved targets attempt ~5x more
rules, not more duplicates or more eviction pressure per prune) points at
resource allocation across a widely varying problem-size distribution,
not at duplicate accounting. A single fixed beam width applied uniformly
regardless of a node's branching factor plausibly under-provisions
exactly the high-branching subtrees that large/complex targets need more
room in, while over-provisioning small/simple ones -- consistent with
solved targets needing an order of magnitude less search than unsolved
ones. This is the one candidate among the five whose mechanism is most
directly responsive to the observed rules_attempted_total gap, rather
than to a duplicate/dedup signal the data does not support.

This is a **pre-registration only** -- no implementation in this round.
If, once implemented and measured, adaptive-beam-by-branching-factor does
not reach the gate below, that is itself the expected, informative
outcome given how orthogonal the 70-target majority looks to every beam-
crowding signal measured here; the next-best candidates by this evidence
would be diversity-aware beam or deterministic reranker-assisted ordering
(both still plausible under a "search wastes budget on low-value
candidates" story, just not evidenced here to the same degree as the
rules_attempted_total gap points at allocation).

### Acceptance gate (fixed before implementation, per program spec)

Evaluated on the same 100-target sample, Conservative policy,
shared_stock, at **beam=100** (beam 200/300 sensitivity-only, not gating).
Baseline = this Phase1C beam100.jsonl run (binary `5d03554`):

- `route_to_configured_stock`: baseline **16/100** -> gate requires **≥ 18/100**
- invalid/unparseable routes: baseline 0 -> gate requires **= 0**
- regressions among the 16 currently-solved targets: **≤ 1** may become unsolved
- timeouts: baseline **0/100** -> gate requires **≤ 0** (no new timeouts introduced)
- p95 latency (completed runs): baseline **83.6s** -> gate requires **≤ 104.5s** (1.25x)
- deterministic repeat: identical `raw_output_sha256` per target across two runs of the candidate binary, same inputs
- beam 200/300: report route_found deltas and whether L1541's non-monotonic
  signature is resolved (informational, not gating)
