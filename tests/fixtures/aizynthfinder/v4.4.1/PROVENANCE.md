# AiZynthFinder v4.4.1 fixture provenance

All fixtures in this directory come from real `aizynthcli` runs against the
official public model/stock data bundle, captured once locally and committed
here so CI never needs AiZynthFinder installed or network access to run
these tests. Do not re-capture routinely -- these are frozen reference
fixtures, not a live benchmark.

## Software

- **aizynthfinder**: `4.4.1` (PyPI, installed via `pip install aizynthfinder`
  into a clean `python3.11` venv -- 4.4.1 requires `>=3.10,<3.13`, so the
  system default Python 3.13 could not be used directly).
- **Public data bundle**: downloaded via `download_public_data <path>`
  (the package's own bundled command), same release as the pinned
  `aizynthfinder==4.4.1`. File SHA-256 (all under the download target dir):
  - `uspto_model.onnx`: `bd0a3cb74cd7068de474c8fb789a00a66bc42c75636d66510ccac585ebe928f8`
  - `uspto_templates.csv.gz`: `a4f1945e90cfa195538320833d68aed38f14e2fcc2f8afb5d958bc920edcafbe`
  - `uspto_filter_model.onnx`: `ad29aa32bdfcbe37065045546493806cf04899c55386c438905d83fb14bb6320`
  - `zinc_stock.hdf5`: `99d39a6f807c3e815487500bafc2b4a9dc66a31af189e3b1776874fb0d4a188d`
  - Stock: ZINC, 17,422,831 compounds (as reported by `aizynthcli`'s own
    startup log: "Compounds in stock: 17422831").
  - Policies used: expansion policy `uspto` (template-based), filter policy
    `uspto`. The `ringbreaker` expansion policy was loaded (present in the
    generated `config.yml`) but not selected for these specific runs.
- **Capture date**: 2026-08-18 (UTC).

## Fixture A: `single_trees.json`

Real single-target `aizynthcli` output, trimmed.

- **Source target SMILES**: `CCOC(=O)c1ccc(N)cc1` (ethyl 4-aminobenzoate /
  benzocaine).
- **Capture command**:
  ```
  aizynthcli --smiles "CCOC(=O)c1ccc(N)cc1" --config config.yml --output single_trees.json
  ```
- **Search result** (from `aizynthcli`'s own summary): solved, 23 total
  routes found, 53 solved routes reported at the route-count level before
  dedup/collection into the tree list (`number of solved routes: 53` in the
  run summary vs. 23 distinct trees in the saved output -- AiZynthFinder's
  own route dedup, not something this fixture alters), first solution at
  iteration 1.
- **Modification from raw output**: real `aizynthcli` output contains 23
  routes; this fixture keeps only routes `[0, 2, 3]` (by original index) --
  one 1-step solved route and two distinct 2-step solved routes, all with
  `metadata.mapped_reaction_smiles` present on every reaction node and
  `in_stock` present on every leaf mol node. No field within a kept route
  was altered. Selected, not synthesized.
- **Output SHA-256** (of the committed file, after trimming):
  `2cafc54b32ee142909bb63cff4c54407331f1aad586dd1f8eb2eb7ba84c5a070`

## Fixture B: `batch_output.json.gz`

Real multi-target `aizynthcli` output (Pandas `orient="table"` JSON,
gzip-compressed), trimmed.

- **Source target SMILES** (one file, one SMILES per line):
  - `CCOC(=O)c1ccc(N)cc1` (benzocaine) -- solved.
  - `CC(C)Cc1ccc(C(C)C(=O)O)cc1` (ibuprofen) -- **not** solved (confirmed
    separately: 100 search iterations, 0 solved routes, `is_solved: False`
    both as a single-target run and in this batch run) -- included
    specifically to exercise the "route present but not solved" case, not
    a "route entirely absent" case.
- **Capture command**:
  ```
  printf "CCOC(=O)c1ccc(N)cc1\nCC(C)Cc1ccc(C(C)C(=O)O)cc1\n" > batch_targets.smi
  aizynthcli --smiles batch_targets.smi --config config.yml --output batch_output.json.gz
  ```
- **Modification from raw output**: each target's real `trees` array
  trimmed to its first 2 routes (benzocaine had 23, ibuprofen had 11 in the
  raw batch run); every other field (`schema`, `is_solved`,
  `number_of_routes`, etc.) is the tool's real, unmodified output. No
  field within a kept route was altered.
- **Output SHA-256** (of the committed file, after trimming):
  `c9d99d6e413fa7613e04c24132f059191cc141662feef005246201edce603aeb`

## Fixture C: `single_trees_missing_atom_mapping.json`

**Not real AiZynthFinder output.** Every reaction node produced by the
`uspto`/`ringbreaker` template-based expansion policies in this local
capture carried `metadata.mapped_reaction_smiles` -- confirmed by scanning
every reaction node in both Fixture A's full 23-route raw output and both
targets' full raw output in the Fixture B capture (34 routes total
inspected, zero missing). This matches AiZynthFinder's own documented
behavior: a template-based expansion always derives the mapped reaction
SMILES from the matched template, so this specific policy configuration
cannot produce the missing-mapping case naturally.

Per the fallback agreed for this case: this fixture is a **minimal,
explicitly-labeled mutation** of Fixture A's route index 1 (one of the
2-step routes) -- `metadata.mapped_reaction_smiles` deleted from that
route's single outermost reaction node, nothing else changed. It exists
solely to exercise `not_evaluable: missing_atom_mapping` in the adapter's
test suite, and must never be cited as evidence of real AiZynthFinder
output lacking this field -- the real capture data above shows the
opposite is true for this policy configuration.
