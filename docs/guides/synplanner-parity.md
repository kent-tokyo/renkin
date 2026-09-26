# SynPlanner feature parity

This page maps [SynPlanner](https://github.com/Laboratoire-de-Chemoinformatique/SynPlanner)
(checked against the 1.6.0 source) planning and interoperability features to
their RENKIN equivalents. A matching name does not mean a matching algorithm:
SynPlanner runs MCTS, while RENKIN runs A* over rules and templates. Each
section below states what RENKIN actually computes.

| SynPlanner | RENKIN | Notes |
| --- | --- | --- |
| `min_mol_size`, `exclude_small` | `--small-molecule-terminal N` | Opt-in. Small molecules become search terminals but are never counted as stock. |
| `max_tree_size` | `--max-tree-size N` | Deterministic node budget; `termination: tree_size_limit_reached`. |
| `use_priority` + priority rules | `--priority-templates FILE`, `--priority-rules a,b` | A proposal from a listed rule gets the cheapest sibling step cost. Multi-application is not implemented. |
| `write_routes_json` | `--format synplanner` | `{route_id: RouteNode}`. Reactions are not atom-mapped. |
| `--export_routes` (`results.json.gz`) | `renkin audit-route results.json.gz` | Auto-detected. Routes are identified as `<target>#<index>`. |
| `max_iterations`, `max_time`, `max_depth` | `--max-expansions`, `--time-limit-secs`, `--depth` | See [ASKCOS and Syntheseus parity](askcos-syntheseus-parity.md) and [AiZynthFinder parity](aizynthfinder-parity.md). |
| `stop_at_first` | `--max-routes 1` | |
| Route visualisation (SVG/HTML) | `--format html`, `renkin.routes_html()` | Self-contained HTML with 2D depictions. Leaf badges distinguish `stock` from `size terminal`. |
| `synplan planning` (batch) | `renkin batch` | Per-target JSON/HTML, `summary.csv`, `manifest.json`. |
| `Tree.report()` / `TreeStats` | `--search-stats` | Node, cache, and first-route counts, plus wall time. |

All new JSON fields are opt-in. Default output does not change.

## Small-molecule terminals

```bash
renkin --target "CC(=O)Oc1ccccc1C(=O)O" --building-blocks stock.smi --small-molecule-terminal 6
```

SynPlanner treats every molecule with `len(molecule) <= min_mol_size` as a
building block, and does so by default (`min_mol_size = 6`). RENKIN offers the
same rule only when asked, because stock identity in RENKIN is exact
standardized canonical-SMILES membership:

- Search treats small molecules as terminals through
  `ChemEnv::is_search_terminal*`. `is_building_block*`, `renkin audit-route`,
  and stock reports still apply exact membership. A route that ends in a
  small, non-stock molecule therefore does not pass a stock audit.
- `small_molecule_terminal.non_stock_leaves` lists, for each returned route,
  the leaves that are size terminals rather than stock.

Use this option to run like-for-like comparisons with SynPlanner runs that
kept its default. For purchasability claims, leave it off.

## Tree-size budget

```bash
renkin --target "..." --depth 6 --max-tree-size 100000
```

The budget counts the root plus every child pushed onto the frontier. The
limit is never exceeded, and the stopping point is the same on every machine.
The opt-in retry passes (`retry-on-integrity-failure`,
`retry-on-beam-exhaustion`) do not retry a run that stopped on a budget.

## Priority templates

```bash
renkin --target "..." --priority-rules boc_deprotection_retro,cbz_deprotection_retro
renkin --target "..." --templates t.smi --priority-templates priority.txt   # template IDs or rule names
```

In every expansion, a proposal from a listed template or rule has its step cost
lowered to the cheapest sibling's cost. That proposal is therefore never
ordered behind a sibling, and it survives `--max-branching` and beam pruning
as well as the cheapest sibling does. The route `score` of promoted steps
changes, as with any ordering prior. No candidates are added.
`priority_templates.unknown` lists names that match no loaded rule. SynPlanner
also offers repeated application to a fixpoint
(`priority_rule_multiapplication`); RENKIN does not implement it.

## Route report, batch planning, and tree statistics

```bash
renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 4 --format html > routes.html
renkin batch --input targets.smi --output-dir results --html -- --depth 5 --beam-width 100 --time-limit-secs 60
renkin --target "..." --search-stats
```

The HTML page has no scripts and no external resources, so it can be archived
or attached as evidence exactly as generated. `renkin batch` runs the normal
search once per target, in a separate process, with the options given after
`--`. `--target` and `--format` are set by batch itself. The default is
`--jobs 1`, so a batch leaves the rest of the machine usable. `summary.csv`
has one row per input line, and a target that fails to parse is recorded as
an `error` row; the batch does not abort. HTML depiction needs the default
`depict` feature, which WASM builds leave out.

## Interoperability

```bash
renkin --target "..." --format synplanner > routes.json
renkin audit-route routes.json --stock stock.smi
renkin audit-route results.json.gz          # SynPlanner --export_routes output
```

The export follows SynPlanner's forward reaction direction
(`reactants>>product`). Because the reactions are not atom-mapped, forward
validation of those steps is reported as `not_evaluable`. The `--export_routes`
reader parses route trees only. It does not read the separate `manifest.json`.

## Not covered

- SynPlanner's MCTS variants (UCT/PUCT, NMCS, LazyNMCS), rollout and
  value-network evaluation, and policy networks.
- The reaction-data curation, mapping, rule-extraction, and training
  pipelines.
- Route clustering by strategic bonds through route CGRs. This needs atom
  mapping that is carried through the whole route. For RENKIN's structural
  alternative, see `--cluster` in the
  [AiZynthFinder parity guide](aizynthfinder-parity.md).
- GUI. (A static HTML route report is available through `--format html`.)
