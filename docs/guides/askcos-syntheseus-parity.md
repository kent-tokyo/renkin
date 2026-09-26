# ASKCOS and Syntheseus feature parity

This page maps common [ASKCOS](https://askcos.mit.edu/) tree-builder settings
and [Syntheseus](https://github.com/microsoft/syntheseus) search and evaluation
settings to their RENKIN equivalents. A matching name does not mean a matching
algorithm. Each note below states what RENKIN actually computes. For
AiZynthFinder, see [AiZynthFinder feature parity](aizynthfinder-parity.md).

| Source | Setting | RENKIN | Notes |
| --- | --- | --- | --- |
| Syntheseus / AiZ | `limit_iterations` / `iteration_limit` | `--max-expansions N` | Deterministic expansion budget; `termination: expansion_limit_reached`. |
| Syntheseus | time / calls to first solution | `--first-route-stats`, `renkin-bench --first-route-stats` | Expansions and retro-expansion calls are deterministic. Wall time depends on the machine. |
| Syntheseus | route diversity (packing number) | `--route-diversity`, `--diversity-radius r` | Reaction-Jaccard distance. Exact within a fixed budget, otherwise a lower bound. |
| ASKCOS | banned chemicals | `--ban-molecules FILE`, `--ban-smiles A,B` | Matched by exact stock identity. Removed as precursors everywhere. |
| ASKCOS | `max_ppg` | `--max-bb-price X` (with `--stock` CSV) | Unit is the CSV price column. Unpriced entries are kept and counted. |
| ASKCOS / AiZ | `max_branching` / `cutoff_number` | `--max-branching N` | Keeps the N cheapest distinct precursor sets for each expanded molecule. |

All new JSON fields are opt-in. Default output does not change.

## Deterministic budget and first-route receipt

```bash
renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 6 --max-expansions 500 --first-route-stats
renkin-bench --input targets.smi --depth 5 --first-route-stats --max-expansions 2000
```

The `first_route` fields:

- `nodes_expanded`: frontier expansions performed before the first route
  passed the acceptance boundary.
- `expansion_calls`: retro-expansion calls, meaning rule-set applications to
  new intermediates. This is the closest RENKIN analogue to Syntheseus's
  reaction-model calls.
- `elapsed_ms`: in-search wall time to that event. It is recorded on native
  targets only.

The event is recorded during search. Constraint filters applied after the
search can still drop that route. In coverage and recovery modes, the stats
cover only the selected stage.

`--max-expansions` stops the search at the same point on every machine,
unlike `--time-limit-secs`.

## Banned molecules

```bash
renkin --target "..." --ban-molecules banned.smi      # one SMILES per line, '#' comments
renkin --target "..." --ban-smiles "OC(=O)c1ccccc1O,ClC(Cl)Cl"
```

Bans use the same standardize-then-canonicalize identity policy as stock
lookup, so different spellings of one molecule match. A candidate that
contains a banned precursor is removed before scoring. It never enters the
frontier and never takes up a beam slot. `banned_molecules.candidates_removed`
counts the removed candidates. If the target itself is banned, the run fails.

## Stock price cap

```bash
renkin --target "..." --stock stock.csv --max-bb-price 5000
```

Entries whose `price_jpy` column is above the cap are removed before the stock
is built. Entries without a price are kept and reported as
`unpriced_entries_kept`, so it is clear where price data is missing.

## Branching cap

```bash
renkin --target "..." --max-branching 25
```

The cap applies once to each unique expanded molecule and uses the search's
own step cost, including any configured prior or reranker bonus. Ties keep
proposal order. If several templates produce the same precursor set, all of
those templates are retained. A cap of `N` can therefore keep more than `N`
entries, but never more than `N` distinct precursor sets.

## Route-set packing number

```bash
renkin --target "..." --max-routes 50 --route-diversity
renkin --target "..." --max-routes 50 --diversity-radius 0.5
```

Each route is represented as a set of reactions, keyed as
`sorted precursors>>target`. Two routes count as distinct when their Jaccard
distance is strictly greater than the radius. `packing_number` is the size of
the largest subset of routes that are pairwise distinct, and
`packing_route_indices` lists one such subset. The value measures structural
route diversity. It does not measure chemical-idea diversity; for that, see the
template-disconnection CDS proxy in
[route-set diversity](route-set-diversity.md).

## Not covered

- ML models from ASKCOS: context/condition recommendation, SCScore, the fast
  filter, site selectivity, and impurity prediction.
- ASKCOS termination rules based on chemical properties or popularity. They
  would treat molecules that are not in stock as terminal, which conflicts
  with RENKIN's exact stock-identity policy.
- Syntheseus search algorithms (MCTS, PDVN) and wrappers for neural
  single-step models.
