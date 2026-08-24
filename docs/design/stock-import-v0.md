# Stock Importer v0 — Design & Contract Doc

Status: **Implemented, PR 1 of v0.36.0 Phase 2 ("Scalable Stock & Audited
Coverage").** Module: `src/stock_import.rs`. Phase 1 (rule-safety census,
PR #193) is complete and merged; this is the first Phase 2 PR, scoped
deliberately narrow per the phase plan: importer core + manifest schema
only, no CLI, no large-file fetch, no default-stock replacement.

## 0. What this is, in one paragraph

`ChemEnv::load`/`from_smiles_iter` (`src/chem_env.rs:75-107`) — RENKIN's
existing stock loader — silently `continue`s past both unparseable lines
and duplicate compounds, with no count of either ever surfaced to a
caller. That's fine for a 402-compound curated file where nobody has
needed to audit the gap, but it doesn't scale to a stock pilot where the
whole point is knowing exactly what got in, what got rejected, and why.
This PR adds a new, independent import path — `stock_import::import_stock`
— that applies the *same* stock-identity policy
(`chem_env::canonical_stock_identity_from_smiles`, i.e. `ChemEnv::load`'s
own standardize-then-canonicalize rule) but records every rejection and
every duplicate with a typed reason, and emits a versioned provenance
manifest alongside the deduped, deterministically-sorted output list.

It does not replace `ChemEnv::load`. The embedded/default stock path is
untouched; this is a new, opt-in tool for producing a *new* stock file
with real provenance, not a rewrite of how RENKIN loads its existing one.

## 1. Scope for this PR

**In:**
- Line-oriented `.smi` input: SMILES as the first whitespace-separated
  token per line; any remaining tokens (a name, a price column, ...)
  ignored; `#`-prefixed and blank lines skipped as non-data rows —
  matches `ChemEnv::load`'s own tokenization exactly, so results are
  directly comparable.
- Streaming, single-pass (`BufReader` over a `Read`, one line at a time —
  never buffers the whole input).
- Canonical identity via RENKIN's own parser and existing
  `canonical_stock_identity_from_smiles` — no new chemistry logic.
- Deterministic dedup (first occurrence wins, canonical structure is the
  key) and deterministic sorted output.
- SHA-256 of both the raw input bytes and the exact output bytes.
- Per-row rejection reasons (typed enum, not a free-text string) and
  per-row duplicate provenance (which earlier line a duplicate matches).
- Source label/revision/license metadata (caller-supplied, never
  guessed).
- Explicit, machine-readable normalization policy snapshot in the
  manifest (mirrors `chem_env::STANDARDIZE_OPTS` field-for-field, built
  fresh from that static every call so it can't drift out of sync).
- `schema_version` on the manifest.
- Unit + integration-style tests, including a run against the real,
  already-committed `data/building_blocks.smi`.

**Out (deliberately, for a later PR):**
- CSV, SDF, `.smi.gz` — the format enum/dispatch point doesn't exist yet;
  `RejectionReason::EmptyField` is kept in the shared enum specifically
  because a delimited format *can* produce it, even though this PR's
  `.smi` path provably cannot (see the module doc comment).
- mmap / on-disk index — not needed until a real large-file pilot shows
  the naive in-memory `Vec<String>` approach is actually a problem.
- 10k/100k measurement runs — a separate PR per the phase plan, gated on
  this PR's contract being solid first.
- `renkin stock import` CLI subcommand — this PR is a library module
  only; the CLI surface is explicitly a later PR so it can be reviewed
  against a stable core API rather than co-evolving with it.
- Replacing the embedded/default stock — `DEFAULT_BUILDING_BLOCKS`
  (`src/lib.rs`) and `data/building_blocks.smi` are both untouched, and
  nothing in this PR bundles a large external stock file into the
  package.
- Issue #86 / AiZynthFinder shared-stock comparison — a separate,
  explicitly-gated track (Phase 3), not touched here.

## 2. Contract

The properties below are each backed by a test in `stock_import.rs`'s own
`#[cfg(test)] mod tests`, not just asserted in prose:

1. **Byte-identical reproducibility, for a fixed importer version.** The
   same input bytes + the same `StockImportOptions`, built with the same
   `renkin` crate version, always produce the same output list and the
   same manifest, down to JSON serialization
   (`same_input_and_options_produce_byte_identical_output_and_manifest`).
   Nothing in the pipeline reads a clock, a random source, or depends on
   hashmap iteration order for anything that ends up in either output —
   the only hash map used (`seen: FxHashMap<...>`) is purely an internal
   membership check during the streaming pass, never iterated for output.
   The one field that legitimately varies across otherwise-identical runs
   is `manifest.importer_version` (`env!("CARGO_PKG_VERSION")`) — a crate
   version bump changes it by design, so a manifest always records which
   importer produced it; that field is intentionally outside this
   property's scope, not an exception to it.
   `input_sha256` itself is verified against an independent SHA-256 of
   the same raw bytes (not just cross-run self-consistency) by
   `input_sha256_matches_an_independently_computed_hash_of_the_same_bytes`,
   including a no-trailing-newline input, since EOF handling is exactly
   where a streaming hasher could silently drop or double-count a byte.
2. **No silent drops.** Every non-comment, non-blank input line is
   accounted for as exactly one of: accepted-and-unique, accepted-and-
   duplicate, or rejected. There is no fourth outcome
   (`accepted_rejected_duplicate_arithmetic_is_internally_consistent`
   checks both arithmetic identities directly: `input_rows == accepted_rows
   + rejected_rows` and `accepted_rows == unique_structures +
   duplicate_rows`).
3. **Typed rejection reasons**, not a bare boolean or a free-text string
   — `RejectionReason` is a real enum, serialized `snake_case`, and
   every rejected row is recorded with its exact 1-based source line
   number and the raw token that failed.
4. **Typed duplicate provenance** — every duplicate row records which
   earlier line number it's a duplicate of, not just a count.
5. **Order-independence of the output**, not just the manifest —
   `output_is_sorted_regardless_of_input_order` feeds the same three
   compounds in two different orders and asserts identical output.
6. **The normalization policy actually used is always visible in the
   manifest**, sourced from the same `STANDARDIZE_OPTS` static
   `ChemEnv::load` uses (bumped from module-private to `pub(crate)` for
   this — `src/chem_env.rs`), not a separately-maintained copy that could
   silently drift.

## 3. Manifest shape

```rust
pub struct StockManifest {
    pub schema_version: u32,           // STOCK_MANIFEST_SCHEMA_VERSION = 1
    pub source: StockSource,           // label, revision, license — caller-supplied
    pub importer_version: String,      // this crate's own CARGO_PKG_VERSION
    pub input_sha256: String,          // sha256:<hex> of the raw input bytes
    pub output_sha256: String,         // sha256:<hex> of render_output(accepted)
    pub input_rows: u64,               // non-comment, non-blank lines
    pub accepted_rows: u64,            // parsed successfully (unique + duplicate)
    pub rejected_rows: u64,
    pub unique_structures: u64,        // == accepted.len()
    pub duplicate_rows: u64,
    pub rejection_reasons: BTreeMap<String, u64>,  // aggregate, by reason
    pub rejected: Vec<RejectedRow>,    // full per-row detail
    pub duplicates: Vec<DuplicateRow>, // full per-row detail
    pub normalization: NormalizationContract,
}
```

This follows the two closest existing manifest precedents in this
codebase rather than inventing a third shape:
`scripts/compare_shared_stock.py`'s manifest (per-row `excluded`/
`duplicates` lists with reasons, source/output SHA-256, round-trip-style
verification) for the *content*, and `pool_export::PoolManifest`/
`MANIFEST_SCHEMA_VERSION` (`src/pool_export.rs:573-611`) for the
*Rust-side schema-versioning discipline*.

**Known limitation, not addressed this PR**: `rejected`/`duplicates` are
stored as full per-row `Vec`s in the manifest. For `data/building_blocks.smi`
(449 data rows) this is trivially small. It has not been evaluated against
a 100k-row input — if a future pilot finds the per-row detail makes the
manifest unwieldy at that scale, that's the point to reconsider (e.g. a
cap with an explicit "N more not shown" count, matching this codebase's
general no-silent-truncation discipline), not before.

## 4. Fixture-consistency finding

Per this PR's explicit requirement, `data/building_blocks.smi` was
imported through the new path (read-only — the file itself was not
modified) and cross-checked against `ChemEnv::load`'s own `bb_count()` on
the identical file:

```
input_rows=449, unique_structures=402, rejected_rows=3, duplicate_rows=44,
rejection_reasons={"unparseable_smiles": 3}
```

`stock_import`'s `unique_structures` (402) matches `ChemEnv::load(...)
.bb_count()` (402) exactly — the two code paths agree, confirmed by
`import_of_real_building_blocks_fixture_matches_chem_env_load_unique_count`,
not just asserted.

This also independently reproduces, for the first time with actual
reproducible tooling, the "402 unique / 3 parse failures" figure that
`docs/guides/open-source-retrosynthesis-comparison.md` had previously
quoted from an unverified one-off measurement (that same doc separately
notes an RDKit re-parse of the same file finds 393 unique / 9 parse
failures — a parser-dependent gap between RENKIN's own SMILES parser and
RDKit's, not something this PR's importer changes or resolves). New,
previously-undocumented information from this run: **44 in-file
duplicate rows** — compounds listed more than once under different SMILES
spellings that canonicalize to the same structure. `data/building_blocks.smi`
itself is unchanged; deduping it, if ever wanted, is a separate decision
for a separate PR, not something this importer does automatically to an
existing file.

## 5. Non-goals restated

This PR does not: run a 10k/100k pilot, add a CLI subcommand, replace the
embedded stock, touch `data/building_blocks.smi`, fetch or commit a large
external file, or make any claim about AiZynthFinder-scale comparison
(issue #86). Each is real, named future work, not silently dropped scope
— see the phase plan for sequencing.
