# `ord_evidence_audit.py` — ORD → RENKIN evidence sidecar

Offline, deterministic conversion from a locally-downloaded
[Open Reaction Database](https://github.com/open-reaction-database/ord-data)
corpus to a RENKIN `schema_version: 2` metadata sidecar. See
[`docs/guides/reaction-evidence.md`](../docs/guides/reaction-evidence.md) for
the full evidence-metadata model this feeds into.

## Setup

```bash
python3 -m venv .venv-ord-evidence
.venv-ord-evidence/bin/pip install -r scripts/requirements-ord-evidence.txt
cargo build --release   # need a built `renkin` binary
```

## Usage

```bash
.venv-ord-evidence/bin/python scripts/ord_evidence_audit.py \
  --ord-data /path/to/ord-data \
  --renkin-bin target/release/renkin \
  --templates data/templates_extracted_5000.smi \
  --output-sidecar artifacts/ord_candidates.json \
  --output-report artifacts/ord_audit.json \
  --output-manifest artifacts/ord_manifest.json
```

`--ord-data` is read-only and local only: `.pb.gz`, `.pbtxt`, and `.pb` files
are discovered recursively. This script never downloads, clones, or makes any
network request — fetch your own copy of ORD data first.

## What gets accepted

A record is written to the sidecar only if **all** of the following hold:

- unique `dataset_id` + `reaction_id`
- exactly one desired product, with a parseable SMILES
- at least one `REACTANT`-role component, each with a parseable SMILES
- `renkin evidence match` reports a **unique** template match (not
  `no_match`, `ambiguous`, or `invalid_input`) — see below for the three
  rules excluded from export even when unique
- at least one of {yield, condition} is present
- a single, unambiguous yield candidate (or none) — multiple non-duplicate
  yield measurements reject the record rather than picking one
- at least one provenance reference (always true once dataset/reaction id are
  validated — the `ord:<dataset-id>:<reaction-id>` reference is minted from
  those, never fabricated)

Everything that fails one of these is **not** silently dropped — it's counted
under a named reason in `by_rejection_reason` in the audit report.

`rule:cn_aliphatic_cleavage`, `rule:michael_retro`, and
`rule:co_aliphatic_cleavage` are counted in the audit report
(`by_template_id`, `records_audit_only_excluded`) but never written to the
sidecar in this phase — see `docs/guides/reaction-evidence.md` for why.

## Field mapping (short version)

| ORD field | RENKIN field | Notes |
|---|---|---|
| `ReactionRole.CATALYST`/`REAGENT`/`SOLVENT` component | `conditions.catalysts`/`reagents`/`solvents` | Deduped, sorted. ORD has no `BASE` role; `bases` is always `[]`. |
| `conditions.temperature.setpoint` | `conditions.temperature_c` | Converted to °C; represented as a degenerate `{min,max}` range when there's no reported precision. |
| `outcomes[0].reaction_time` | `conditions.time_hours` | Converted to hours. |
| `conditions.pressure.atmosphere` | `conditions.atmosphere` | Named enum values map to a fixed lowercase string; `CUSTOM` uses `details` verbatim. |
| `outcomes[0].conversion` | `reported_yield` (`basis: "conversion"`) | |
| product `ProductMeasurement.YIELD` (`percentage`) | `reported_yield` (`basis: "unknown"`) | ORD's `YIELD` type doesn't itself distinguish isolated vs. calibrated-assay — never guessed. Measurement provenance (`analysis_key`, `uses_internal_standard`, `uses_authentic_standard`, `details`) is kept as a note on the example instead. |
| `provenance.doi` / `.patent` / `.publication_url` | `references[].kind = doi/patent/url` | Normalized (prefix-stripped, lowercased for DOI) but never repaired or completed; unnormalizable ones are simply not cited. |
| `dataset_id` + `reaction_id` | `references[].kind = dataset_record` | Always present once both ids are validated non-empty. |

## Determinism & reproducibility

- Running the script twice on the same input produces byte-identical
  `--output-sidecar` and `--output-report` (see
  `scripts/tests/test_ord_evidence_audit.py`'s
  `test_two_runs_are_byte_identical_except_manifest_timestamp`).
- `--output-manifest` records input file hashes, the RENKIN version/git
  commit, dependency versions, and the exact CLI invocation used — everything
  except `generated_at` (explicitly marked informational, excluded from
  reproducibility comparison).
- The generated sidecar is re-validated via `renkin evidence
  validate-sidecar` before this script reports success; a sidecar that fails
  that validation makes the whole run exit non-zero.

## Licensing

This script is MIT (same as the rest of RENKIN). `ord-schema`
(`scripts/requirements-ord-evidence.txt`) is Apache-2.0. The ORD *data* you
point `--ord-data` at is
[CC-BY-SA-4.0](https://github.com/open-reaction-database/ord-data/blob/main/LICENSE) —
a generated sidecar/report is a derivative of that data and should be treated
as CC-BY-SA-4.0 with attribution, not folded into RENKIN's MIT code license.
No real ORD corpus is committed to this repository; only the hand-authored
fixture under `scripts/tests/fixtures/` (see that directory's `README.md`).

## Tests

```bash
python -m unittest discover scripts/tests               # no ord-schema needed
.venv-ord-evidence/bin/python -m unittest discover scripts/tests   # full suite
```
