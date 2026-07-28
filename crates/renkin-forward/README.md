# renkin-forward

Template-based forward reaction prediction for [RENKIN](https://github.com/kent-tokyo/renkin).

Two independent subcommands:

```bash
renkin-forward predict --reactants <SMILES>... [--templates <path>] [--max-results N] [--report]
renkin-forward validate --route-json <JSON> [--templates <path>] [--max-results N]
```

`predict` is a **standalone** capability: given reactant SMILES, it reverses
every reversible SMIRKS-backed retrosynthetic template RENKIN knows about
(`product >> precursors` becomes `precursors >> product`), forward-applies
each one, and returns ranked, deduplicated product candidates with full
per-template provenance. `validate` uses the same engine internally to check
whether a retrosynthetic route's steps reproduce their targets when
forward-applied.

Full documentation, including limitations, error/warning codes, and the
Rust API: [Forward Reaction Prediction guide](../../docs/guides/forward-prediction.md).

## Quick example

```bash
cargo build --release -p renkin-forward
target/release/renkin-forward predict \
  --reactants "Oc1ccccc1C(=O)O" "CCO" \
  --report --max-results 5
```

## What this is not

Not a learned/neural forward-reaction model, not a yield predictor, not a
reaction-condition recommender, and not a side-product predictor. Ranking is
a transparent, template-frequency-derived signal only — see the guide's
Limitations section for the full list.

## License

MIT, same as the rest of the RENKIN workspace.
