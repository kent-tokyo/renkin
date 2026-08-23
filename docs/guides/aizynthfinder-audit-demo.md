---
title: "Audit Real AiZynthFinder Routes with RENKIN: A 5-Minute Demo"
description: "A copy-pasteable walkthrough of renkin audit-route against real captured AiZynthFinder v4.4.1 output, including what a partial/pass/fail verdict actually means."
---

# Audit a Real AiZynthFinder Route with RENKIN

*Keep AiZynthFinder. Audit its routes with RENKIN.*

RENKIN Bridge audits route JSON from a competitor tool through the same
tool-neutral pipeline it applies to its own routes: structural integrity,
stock, and declared-reaction forward-replay validation, each reported
independently, rolled up into a route-level `pass`/`fail`/`partial` verdict.
This page walks through it end to end against **real** captured
`aizynthcli 4.4.1` output — not a hand-authored example — so every command
and every line of output below is something you can reproduce yourself from
a checkout of this repo.

## The fixture

`tests/fixtures/aizynthfinder/v4.4.1/single_trees.json` is real output from
a real `aizynthcli --smiles "CCOC(=O)c1ccc(N)cc1" ...` run (benzocaine),
captured once and committed for CI to use without needing AiZynthFinder
installed. Full capture provenance — exact command, model/stock file
SHA-256 hashes, ZINC stock size — is in the sibling
[`PROVENANCE.md`](https://github.com/kent-tokyo/renkin/blob/master/tests/fixtures/aizynthfinder/v4.4.1/PROVENANCE.md).

## Step 1: audit without a configured stock

```bash
renkin audit-route tests/fixtures/aizynthfinder/v4.4.1/single_trees.json \
  --format aizynthfinder \
  --output human
```

```text
3 routes audited — 0 pass, 0 fail, 3 partial
route 1/3: PARTIAL
  - stock: StockNotProvided
route 2/3: PARTIAL
  - stock: StockNotProvided
route 3/3: PARTIAL
  - stock: StockNotProvided
```

Every route comes back `PARTIAL`, not `PASS`. **This is correct, not a
weaker result.** No `--stock` was given, so RENKIN has nothing to check the
leaf molecules against — it reports `not_evaluable` (`stock_not_provided`)
rather than silently treating "we didn't check" as "it's fine." `partial`
means at least one check couldn't reach a verdict and nothing outright
failed; it is a distinct, three-valued outcome from `pass`, never collapsed
into a boolean.

## Step 2: audit with a configured stock

```bash
renkin audit-route tests/fixtures/aizynthfinder/v4.4.1/single_trees.json \
  --format aizynthfinder \
  --stock data/building_blocks.smi \
  --output human
```

```text
3 routes audited — 1 pass, 2 fail, 0 partial
route 1/3: FAIL
  - LeafClaimedStockNotMatched
route 2/3: PASS
route 3/3: FAIL
  - LeafClaimedStockNotMatched
```

Now RENKIN checks every leaf molecule against `data/building_blocks.smi`
(RENKIN's own default building-block set, unrelated to AiZynthFinder's own
`in_stock` claims). Route 2 passes outright — its two precursors (ethanol
and 4-aminobenzoic acid) are both in RENKIN's stock, and every step's
declared reaction replays correctly. Routes 1 and 3 fail because a precursor
AiZynthFinder's own policy proposed isn't in *this particular* stock file —
a real, informative disagreement about what counts as "available," not a
parsing or adapter defect.

`--output json` gives the same verdicts as a machine-readable report
(`schema_version`, an `audit_manifest` recording what was audited and
under what conditions — RENKIN version, source format, input/stock
content hashes, audit policy — for reproducing the same audit later,
per-route `status`/`stock_validation`/`steps`/`findings`, and a
route-level `summary`) — pipe it to `jq`/`python -m json.tool`/whatever
you already use for the RENKIN-native report shape.

## What `--format auto` does here

Both commands above pass `--format aizynthfinder` explicitly. Omit it and
`--format auto` (the default) detects the same result on its own — reading
`single_trees.json`'s top-level shape (an array whose first element has
`type: "mol"`) as AiZynthFinder's single-target export, distinct from
RENKIN-native JSON's `{"target": ..., "routes": [...]}` shape or a batch
export's Pandas `{"schema": ..., "data": [...]}` shape. If the input doesn't
match any of those three recognized shapes, `audit-route` hard-errors rather
than guessing.

## Compatibility

| Source | Verified version | Input |
|---|---|---|
| RENKIN | 0.26.0 | native JSON |
| AiZynthFinder | 4.3.2, 4.4.0, 4.4.1 | single-target JSON, batch JSON, gzip-compressed batch JSON |
| Other planners / other AiZynthFinder versions | unverified | unknown fields are tolerated (forward-compatible); a shape RENKIN doesn't recognize is a hard error, never a guess |

Syntheseus and SynPlanner are also verified adapters (`--format syntheseus`
/ `--format synplanner`) — see the
[Syntheseus audit demo](syntheseus-audit-demo.md) and
[SynPlanner audit demo](synplanner-audit-demo.md) for their own
walkthroughs and compatibility rows.

"Verified against" is deliberate phrasing, not "supported" — this adapter is
confirmed against real, individually captured `aizynthcli` output from
three separate versions specifically (`4.3.2`, `4.4.0`, `4.4.1` — see each
version's own `tests/fixtures/aizynthfinder/vX.Y.Z/PROVENANCE.md`), not
claimed to work with every AiZynthFinder release. All three versions were
run against the identical public model/stock data bundle (confirmed
byte-identical by SHA-256 across all three captures) and the identical
target molecules, so any behavioral difference found is attributable to
the `aizynthfinder` package itself, not to different inputs —
[`tests/aizynthfinder_version_matrix.rs`](https://github.com/kent-tokyo/renkin/blob/master/tests/aizynthfinder_version_matrix.rs)
asserts all three produce identical audit verdicts for the same real
routes. One confirmed, harmless cross-version JSON difference was found in
the process: `4.3.2`'s route `scores` object carries an extra
`"average template occurrence"` field absent from `4.4.0`/`4.4.1`'s
output — outside the tree structure RENKIN's normalizer reads, so it
doesn't affect any verdict (see `v4.3.2`'s own `PROVENANCE.md` for the
full finding). This is one instance of a general rule every adapter
follows — see the
[Audit Reproducibility and Compatibility Contract](audit-reproducibility-contract.md)
for the full set, including what `audit_manifest` guarantees and how a new
adapter or fixture is added.

## Hit a compatibility problem?

If `audit-route` rejects real output from AiZynthFinder (any version) or
another planner, or a real route gets flagged `not_evaluable` in a way that
looks wrong, please
[open an adapter-compatibility issue](https://github.com/kent-tokyo/renkin/issues/new?template=adapter_compatibility.yml) —
it's set up to capture exactly what turns a real report into the next test
fixture.
