# AiZynthFinder v4.4.0 fixture provenance

All fixtures in this directory come from real `aizynthcli` runs against the
official public model/stock data bundle, captured once locally and committed
here so CI never needs AiZynthFinder installed or network access to run
these tests. Do not re-capture routinely -- these are frozen reference
fixtures, not a live benchmark.

Part of the v0.32.0 Phase 2B AiZynthFinder version matrix -- same target
molecules, same capture methodology, and the same route-selection criteria
as [`v4.4.1`'s own fixtures](../v4.4.1/PROVENANCE.md) and
[`v4.3.2`'s own fixtures](../v4.3.2/PROVENANCE.md), so all three are
directly comparable rather than testing different things under the same
version label.

## Software

- **aizynthfinder**: `4.4.0` (PyPI, installed via `pip install aizynthfinder`
  into a clean `python3.11` venv -- 4.4.0 requires `>=3.9,<3.12`, so the
  system default Python 3.13 could not be used directly).
- **Public data bundle**: downloaded via `download_public_data <path>`
  (the package's own bundled command). File SHA-256 (all under the
  download target dir) -- **byte-identical to the files downloaded for
  both `v4.3.2` and `v4.4.1`**, confirmed by matching SHA-256:
  - `uspto_model.onnx`: `bd0a3cb74cd7068de474c8fb789a00a66bc42c75636d66510ccac585ebe928f8`
  - `uspto_templates.csv.gz`: `a4f1945e90cfa195538320833d68aed38f14e2fcc2f8afb5d958bc920edcafbe`
  - `uspto_filter_model.onnx`: `ad29aa32bdfcbe37065045546493806cf04899c55386c438905d83fb14bb6320`
  - `zinc_stock.hdf5`: `99d39a6f807c3e815487500bafc2b4a9dc66a31af189e3b1776874fb0d4a188d`
  - Stock: ZINC, 17,422,831 compounds (as reported by `aizynthcli`'s own
    startup log: "Compounds in stock: 17422831").
  - Policies used: expansion policy `uspto` (template-based), filter policy
    `uspto`. The `ringbreaker` expansion policy was loaded (present in the
    generated `config.yml`) but not selected for these specific runs.
- **Capture date**: 2026-08-22 (UTC).

## Fixture A: `single_trees.json`

Real single-target `aizynthcli` output, trimmed.

- **Source target SMILES**: `CCOC(=O)c1ccc(N)cc1` (ethyl 4-aminobenzoate /
  benzocaine) -- same target as `v4.3.2`/`v4.4.1`'s Fixture A.
- **Capture command**:
  ```
  aizynthcli --smiles "CCOC(=O)c1ccc(N)cc1" --config config.yml --output single_trees.json
  ```
- **Search result** (from `aizynthcli`'s own summary): solved, 23 total
  routes found, 53 solved routes reported at the route-count level before
  dedup/collection into the tree list, first solution at iteration 1 --
  identical search statistics to `v4.3.2`/`v4.4.1`'s own captures (same
  model, same data, same target).
- **Modification from raw output**: real `aizynthcli` output contains 23
  routes; this fixture keeps only routes `[0, 2, 3]` (by original index) --
  one 1-step solved route and two distinct 2-step solved routes, all with
  `metadata.mapped_reaction_smiles` present on every reaction node and
  `in_stock` present on every leaf mol node -- the identical selection
  criteria used for `v4.3.2`/`v4.4.1`'s Fixture A. No field within a kept
  route was altered. Selected, not synthesized.
- **Output SHA-256** (of the committed file, after trimming):
  `2cafc54b32ee142909bb63cff4c54407331f1aad586dd1f8eb2eb7ba84c5a070`
  -- **byte-identical to `v4.4.1`'s own committed `single_trees.json`**,
  confirmed by matching SHA-256: the JSON export shape (and this specific
  search's result) did not change between `4.4.0` and `4.4.1`. `v4.3.2`'s
  equivalent output differs -- see the `average template occurrence` note
  in [`v4.3.2/PROVENANCE.md`](../v4.3.2/PROVENANCE.md).

## Fixture B: `batch_output.json.gz`

Real multi-target `aizynthcli` output (Pandas `orient="table"` JSON,
gzip-compressed), trimmed.

- **Source target SMILES** (one file, one SMILES per line):
  - `CCOC(=O)c1ccc(N)cc1` (benzocaine) -- solved.
  - `CC(C)Cc1ccc(C(C)C(=O)O)cc1` (ibuprofen) -- **not** solved (5 raw
    routes saved, `is_solved: False`) -- same "route present but not
    solved" case `v4.3.2`/`v4.4.1`'s Fixture B exercises. (The raw route
    count for this not-solved target differs across the three captures --
    11 for `v4.4.1`, 9 for `v4.3.2`, 5 here. The single-target search
    above is fully deterministic across all three versions -- identical
    23 routes, identical `top_score` to the last digit -- so this is not
    generic run-to-run nondeterminism; the actual cause was not
    investigated. Not a fixture inconsistency either way: the trimmed
    fixture keeps 2 routes regardless of how many the raw run produced.)
- **Capture command**:
  ```
  printf "CCOC(=O)c1ccc(N)cc1\nCC(C)Cc1ccc(C(C)C(=O)O)cc1\n" > batch_targets.smi
  aizynthcli --smiles batch_targets.smi --config config.yml --output batch_output.json.gz
  ```
- **Modification from raw output**: each target's real `trees` array
  trimmed to its first 2 routes; every other field (`schema`, `is_solved`,
  `number_of_routes`, etc.) is the tool's real, unmodified output. No
  field within a kept route was altered.
- **Output SHA-256** (of the committed file, after trimming):
  `40d8b7b985cd839e60ffaaf5bd1460d3feed16457442ff0c0614e4dddb9e44e3`

## No separate missing-atom-mapping / extra-future-fields fixture here

Same reasoning as [`v4.3.2/PROVENANCE.md`](../v4.3.2/PROVENANCE.md)'s
identical section: `v4.4.1`'s `single_trees_missing_atom_mapping.json`
already exercises RENKIN's own adapter-side handling of that case, and
that behavior doesn't depend on which real AiZynthFinder version supplied
the input.
