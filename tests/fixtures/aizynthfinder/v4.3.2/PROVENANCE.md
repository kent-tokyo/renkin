# AiZynthFinder v4.3.2 fixture provenance

All fixtures in this directory come from real `aizynthcli` runs against the
official public model/stock data bundle, captured once locally and committed
here so CI never needs AiZynthFinder installed or network access to run
these tests. Do not re-capture routinely -- these are frozen reference
fixtures, not a live benchmark.

Part of the v0.32.0 Phase 2B AiZynthFinder version matrix -- same target
molecules, same capture methodology, and the same route-selection criteria
as [`v4.4.1`'s own fixtures](../v4.4.1/PROVENANCE.md), so the two are
directly comparable rather than testing different things under the same
version label.

## Software

- **aizynthfinder**: `4.3.2` (PyPI, installed via `pip install aizynthfinder`
  into a clean `python3.11` venv -- 4.3.2 requires `>=3.9,<3.12`, so the
  system default Python 3.13 could not be used directly).
- **Public data bundle**: downloaded via `download_public_data <path>`
  (the package's own bundled command). File SHA-256 (all under the
  download target dir):
  - `uspto_model.onnx`: `bd0a3cb74cd7068de474c8fb789a00a66bc42c75636d66510ccac585ebe928f8`
  - `uspto_templates.csv.gz`: `a4f1945e90cfa195538320833d68aed38f14e2fcc2f8afb5d958bc920edcafbe`
  - `uspto_filter_model.onnx`: `ad29aa32bdfcbe37065045546493806cf04899c55386c438905d83fb14bb6320`
  - `zinc_stock.hdf5`: `99d39a6f807c3e815487500bafc2b4a9dc66a31af189e3b1776874fb0d4a188d`
  - **Byte-identical to the files downloaded for the `v4.4.1` fixtures** --
    confirmed by matching SHA-256, not assumed. The public model/stock data
    release is evidently not versioned in lockstep with the `aizynthfinder`
    package itself; whatever compatibility differences exist between
    versions come from the package's own code (search algorithm, JSON
    export shape), not from different model/stock inputs. This makes the
    cross-version comparison in this matrix a genuinely controlled one.
  - Stock: ZINC, 17,422,831 compounds (as reported by `aizynthcli`'s own
    startup log: "Compounds in stock: 17422831").
  - Policies used: expansion policy `uspto` (template-based), filter policy
    `uspto`. The `ringbreaker` expansion policy was loaded (present in the
    generated `config.yml`) but not selected for these specific runs.
- **Capture date**: 2026-08-22 (UTC).

## Fixture A: `single_trees.json`

Real single-target `aizynthcli` output, trimmed.

- **Source target SMILES**: `CCOC(=O)c1ccc(N)cc1` (ethyl 4-aminobenzoate /
  benzocaine) -- same target as `v4.4.1`'s Fixture A.
- **Capture command**:
  ```
  aizynthcli --smiles "CCOC(=O)c1ccc(N)cc1" --config config.yml --output single_trees.json
  ```
- **Search result** (from `aizynthcli`'s own summary): solved, 23 total
  routes found, 53 solved routes reported at the route-count level before
  dedup/collection into the tree list, first solution at iteration 1 --
  identical search statistics to `v4.4.1`'s own capture (same model, same
  data, same target).
- **Modification from raw output**: real `aizynthcli` output contains 23
  routes; this fixture keeps only routes `[0, 2, 3]` (by original index) --
  one 1-step solved route and two distinct 2-step solved routes, all with
  `metadata.mapped_reaction_smiles` present on every reaction node and
  `in_stock` present on every leaf mol node -- the identical selection
  criteria used for `v4.4.1`'s Fixture A. No field within a kept route was
  altered. Selected, not synthesized.
- **Output SHA-256** (of the committed file, after trimming):
  `14238c1ed9e1e72e34034686fecfa93fa80d5c1bfde6a9e924a80fb70692a539`

**A real, confirmed cross-version schema difference**: each route's
`scores` object here includes an `"average template occurrence"` field
that is absent from the equivalent `v4.4.0`/`v4.4.1` output for the
identical search (confirmed by diffing the two real captures directly,
not inferred). This field lives in AiZynthFinder's own route-ranking
metadata (`route["scores"]`), not in the tree structure RENKIN's
`normalize_aizynthfinder_route` actually reads (`type`/`smiles`/
`children`/`metadata.mapped_reaction_smiles`/`in_stock`) -- confirmed
harmless in practice: `renkin audit-route` against this fixture produces
identical verdicts to the `v4.4.0`/`v4.4.1` fixtures for the same route
shapes (see the compatibility table in
[`docs/guides/aizynthfinder-audit-demo.md`](https://github.com/kent-tokyo/renkin/blob/master/docs/guides/aizynthfinder-audit-demo.md)).

## Fixture B: `batch_output.json.gz`

Real multi-target `aizynthcli` output (Pandas `orient="table"` JSON,
gzip-compressed), trimmed.

- **Source target SMILES** (one file, one SMILES per line):
  - `CCOC(=O)c1ccc(N)cc1` (benzocaine) -- solved.
  - `CC(C)Cc1ccc(C(C)C(=O)O)cc1` (ibuprofen) -- **not** solved (9 raw
    routes saved, `is_solved: False`) -- same "route present but not
    solved" case `v4.4.1`'s Fixture B exercises.
- **Capture command**:
  ```
  printf "CCOC(=O)c1ccc(N)cc1\nCC(C)Cc1ccc(C(C)C(=O)O)cc1\n" > batch_targets.smi
  aizynthcli --smiles batch_targets.smi --config config.yml --output batch_output.json.gz
  ```
- **Modification from raw output**: each target's real `trees` array
  trimmed to its first 2 routes (benzocaine had 23, ibuprofen had 9 in the
  raw batch run); every other field (`schema`, `is_solved`,
  `number_of_routes`, etc.) is the tool's real, unmodified output. No
  field within a kept route was altered.
- **Output SHA-256** (of the committed file, after trimming):
  `01a793cfe86f6a4c6f19a50e88db89a9b6351e07ccf57b33fe2ecb7e42d57919`

## No separate missing-atom-mapping / extra-future-fields fixture here

`v4.4.1`'s directory already has a `single_trees_missing_atom_mapping.json`
-- a minimal, explicitly-labeled synthetic mutation exercising RENKIN's
own `not_evaluable: missing_atom_mapping` handling. That behavior is a
property of RENKIN's adapter code, not of which real AiZynthFinder version
supposedly produced the input, so re-deriving an equivalent synthetic
mutation per version here would add test surface without adding real
compatibility signal. Real per-version differences belong in this file as
confirmed findings (see the `average template occurrence` note above),
not as more hand-typed mutations.
