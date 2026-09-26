# AiZynthFinder feature parity

This page maps common [AiZynthFinder](https://github.com/MolecularAI/aizynthfinder)
options to their RENKIN equivalents. Each equivalent is scoped to what RENKIN
actually computes. A matching name does not mean a matching algorithm.

| AiZynthFinder | RENKIN | Notes |
| --- | --- | --- |
| `time_limit` | `--time-limit-secs N` / `time_limit_seconds=N` | Standard search only. The deadline is cooperative, and routes found before it are kept. JSON gains `time_limit_secs` and `termination`. |
| `exclude_target_from_stock` | `--exclude-target-from-stock` / `exclude_target_from_stock=True` | The target is never treated as a stock terminal, so no depth-0 route is returned. |
| `AiZynthExpander` | `renkin expand` / `renkin.expand()` | One-step disconnections from RENKIN's rules, merged by precursor set, with stock membership for each precursor. |
| `route_distances` + `RouteCollection.cluster` | `--cluster`, `--n-clusters K`, `--max-clusters N` | Unit-cost tree edit distance and average-linkage clustering. See the limits below. |
| `trees.json` / `ReactionTree.to_dict()` | `--format aizynth` | A route array with the same structure as AiZynthFinder's output. Reactions are not atom-mapped. |

## Time limit

```bash
renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 8 --max-routes 100 --time-limit-secs 10
```

`termination` is `deadline_exceeded` when the budget stopped the search. The
deadline is checked at cooperative checkpoints, so it is not a hard real-time
bound. Coverage and recovery modes keep `--coverage-timeout-secs` and
`--recovery-timeout-secs`.

## Excluding the target from stock

```bash
renkin --target "CC(=O)O" --depth 2 --exclude-target-from-stock
```

Stock identity for precursors is unchanged: exact standardized canonical SMILES.

## Single-step expansion

```bash
renkin expand --target "CC(=O)Oc1ccccc1C(=O)O" --max-candidates 10 --output human
renkin expand --target "..." --templates data/templates_extracted.smi --top-templates 5000
```

```python
import json, renkin
result = json.loads(renkin.expand("CC(=O)Oc1ccccc1C(=O)O", max_candidates=10))
```

Candidates are ordered by RENKIN's heuristic step cost, then by the number of
in-stock precursors. This order is not a neural-policy probability, and it says
nothing about feasibility or yield.

## Route distance and clustering

```bash
renkin --target "..." --max-routes 20 --cluster            # k chosen by silhouette (<= 5)
renkin --target "..." --max-routes 20 --n-clusters 3       # fixed k
```

`route_clusters` contains `distance_matrix` and `labels` (one label per route,
in route order; the first route is always in cluster 0), along with
`n_clusters`, `silhouette`, and `selection`.

Limits:

- Routes become molecule/reaction trees. Children are placed in a canonical
  order, and distance is the Zhang–Shasha ordered tree edit distance with unit
  costs. Two nodes of the same kind with identical labels cost 0 to match;
  different labels cost 1.
- AiZynthFinder weights molecule substitutions by fingerprint Tanimoto
  distance. RENKIN does not, so the two tools' distances are not numerically
  comparable. RENKIN's value is an upper bound on the unordered unit-cost
  distance.
- The distance measures structural differences between routes. It is not a
  chemical-similarity score.

## AiZynthFinder-format export

```bash
renkin --target "CC(=O)Oc1ccccc1C(=O)O" --format aizynth > trees.json
renkin audit-route trees.json --format aizynthfinder --stock data/building_blocks.smi
```

Reaction `smiles` follows AiZynthFinder's retro direction
(`product>>reactants`). `mapped_reaction_smiles`, `template_hash`, and
`policy_probability` are left out on purpose. RENKIN's own template identity
is recorded as `metadata.renkin_template_id`. Because the reactions have no
mapping, the audit reports forward validation as `not_evaluable` for those
steps.

## Not covered

- MCTS, DFPN, and Retro* with neural expansion or filter policies. RENKIN
  searches with A* over rules and templates.
- Bond constraints (`freeze_bonds`, `break_bonds`). These require atom-map
  tracking through rule application.
- Route images (PNG/SVG). `--format mermaid` and `--format tree` provide text
  renderings.
