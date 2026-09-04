# Stock Importer v0 — Design & Contract Doc

Status: **Implemented, PR 1 + PR 2 of v0.36.0 Phase 2 ("Scalable Stock &
Audited Coverage").** Module: `src/stock_import.rs`. Phase 1 (rule-safety
census, PR #193) is complete and merged. PR 1 (#194) added the importer
core + manifest schema as a library module only, deliberately no CLI. PR
2 (this update, §6 below) adds the `renkin stock import` /
`renkin doctor stock` CLI on top of that same core -- no new
canonicalize/dedup/manifest logic, no large-file fetch, no default-stock
replacement.

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
- `renkin stock import` CLI subcommand — PR 1 was a library module only;
  the CLI surface was deliberately deferred to a later PR so it could be
  reviewed against a stable core API rather than co-evolving with it.
  **Delivered in PR 2 (§6 below).**
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

## 5. PR 1 non-goals restated

PR 1 did not: run a 10k/100k pilot, add a CLI subcommand, replace the
embedded stock, touch `data/building_blocks.smi`, fetch or commit a large
external file, or make any claim about AiZynthFinder-scale comparison
(issue #86). Each was real, named future work, not silently dropped
scope. The CLI subcommand is now delivered by PR 2 (§6); the rest remain
open for a later PR — see the phase plan for sequencing.

## 6. CLI (v0.36.0 Phase 2 PR 2)

Two new subcommands on the existing `renkin` binary (manual `args[1]`
dispatch, same convention as `stock`/`template`/`evidence`/`audit-route`
— no `clap`/argument-parsing crate in this codebase). Neither
reimplements any of §0-4's canonicalize/dedup/manifest logic; both call
straight into `stock_import`'s public functions
(`import_stock_from_path`, `render_output`, `current_normalization_contract`).

### 6.1 `renkin stock import`

```
renkin stock import \
  --input <path> --output <path> --manifest <path> \
  --source-label <label> \
  [--source-revision <rev>] [--license <license>] \
  [--fail-on-rejection] [--force]
```

Contract:
- `--input`/`--output`/`--manifest`/`--source-label` are required;
  `--source-revision`/`--license` are optional and recorded as `None`
  (not an empty string) when omitted.
- `--input`, `--output`, and `--manifest` must be three distinct paths
  (checked via `std::path::absolute`, a pure lexical comparison — not
  symlink-aware, but enough to catch the direct-collision mistake before
  any file is touched).
- `--output`/`--manifest` are refused if either already exists, unless
  `--force` is given.
- Both artifacts are written to `<path>.stock-import-tmp` siblings
  first (same directory as the real destination, so the final rename is
  same-filesystem and atomic), fsynced, then both renamed into place
  back-to-back (`write_two_artifacts_atomically` in `src/main.rs`). If
  the second rename fails after the first succeeded, the first
  destination is removed again to restore the pre-call state — *unless*
  it was a pre-existing file being overwritten under `--force`, in which
  case the original bytes were never backed up and can't be restored;
  the returned error says explicitly which artifact is now out of sync
  with which. This narrow post-`--force`-overwrite window is a known,
  documented ceiling, not silently swallowed — a real backup-and-restore
  path is future work if anyone actually hits it.
- Rejected/duplicate rows never abort the import by themselves — the
  run still succeeds (exit 0) and both artifacts are written. Only
  `--fail-on-rejection` turns a nonzero `rejected_rows` into a nonzero
  exit code (1), and even then the artifacts are written first: a
  rejected-but-imported run leaves useful output on disk, not nothing.
- stdout carries exactly one pretty-printed JSON object (the output/
  manifest paths plus the full `StockManifest`) and nothing else;
  progress warnings (rejected-row count, duplicate-row count) go to
  stderr only.

### 6.2 `renkin doctor stock`

```
renkin doctor stock \
  --stock <path> --manifest <path> \
  [--input <path>] [--output human|json]
```

`renkin doctor stock` is a subcommand of the main `renkin` binary
(`args[1] == "doctor"`, same dispatch style as `stock`/`template`/
`evidence`/`audit-route`) and is unrelated to the pre-existing standalone
`renkin-doctor` binary (`src/bin/doctor.rs`, installed as its own `[[bin]]`
target) — that one is a flat, argument-free asset/environment checker
(templates, reranker model, WASM package, Python bindings, ...) with no
subcommands, no severities, and no distinct exit codes. `renkin doctor
stock` is a new, separate, typed report specifically for a `stock import`
output/manifest pair; it does not call into or replace `renkin-doctor`.

Independently re-verifies a `renkin stock import` output/manifest pair
— it does not trust the manifest's own claims, it recomputes and
compares. Nine checks, run unconditionally except `input_hash` (only
when `--input` is given):

| check | what it compares | severity on mismatch |
|---|---|---|
| `schema_version` | manifest's `schema_version` vs `STOCK_MANIFEST_SCHEMA_VERSION` | Fail |
| `output_hash` | SHA-256 of `--stock`'s real bytes vs `manifest.output_sha256` | Fail |
| `input_hash` | SHA-256 of `--input`'s real bytes vs `manifest.input_sha256` | Fail |
| `manifest_arithmetic` | the two identities from §2 item 2 ("No silent drops") | Fail |
| `stock_line_count` | non-blank line count of `--stock` vs `manifest.unique_structures` | Fail |
| `reimport_idempotency` | re-running `import_stock` on `--stock`'s own bytes reproduces it byte-identically with zero new rejections/duplicates | Fail |
| `normalization_contract` | manifest's `normalization` vs `current_normalization_contract()` (the *currently running* binary's live policy, not whatever was live when the manifest was generated) | Fail |
| `importer_version` | manifest's `importer_version` vs this binary's `CARGO_PKG_VERSION` | **Warn** |
| `source_provenance` | whether `source.revision`/`source.license` are present | **Warn** |

Only `importer_version` and `source_provenance` are Warn-only by
design: a manifest generated by a different (but still trustworthy)
`renkin` build, or one missing optional licensing metadata, is not
itself evidence the stock file is wrong — it's flagged, not failed.
Every other check failing means the stock/manifest pair is
self-inconsistent or doesn't match what's on disk, which is always a
real problem.

Exit codes (checked directly by `tests/stock_import_cli.rs`, not just
documented):
- **0** — every check Pass or Warn (`report.overall != Fail`).
- **1** — at least one check Fail.
- **2** — invocation-level problem: missing/unsupported `--output`,
  missing `--stock`/`--manifest`, an unreadable `--stock`/`--manifest`/
  `--input` file, or a `--manifest` file that isn't valid JSON for the
  current `StockManifest` shape at all (as opposed to valid JSON with a
  `schema_version` this build disagrees with, which is the
  `schema_version` check above — exit 1, not 2, since the doctor *could*
  read it and form an opinion).

`--output human` (default) prints one summary line plus one
`[SEVERITY] name: message` line per check; `--output json` prints the
full typed `StockDoctorReport` (`stock_path`, `manifest_path`,
`input_path`, `overall`, `checks[]`) and nothing else on stdout.

### 6.3 Security/atomicity assumptions

- Single-process, local-filesystem tool — no network access, no
  concurrent-writer coordination beyond the temp-file-then-rename
  pattern (a second concurrent `renkin stock import` targeting the same
  `--output` could still interleave; not addressed here, matching this
  tool's stated scope as a local pilot/provenance utility, not a
  multi-writer service).
- `std::fs::rename` is atomic per-file on every platform this crate
  targets as long as source and destination are on the same filesystem
  — guaranteed here since the temp file is always written as a sibling
  of its real destination, never in a shared/system temp directory.
- No sandboxing beyond what the OS/filesystem permissions already
  provide; `--force` is the only privilege being modeled (whether an
  existing destination may be overwritten), not a filesystem ACL.

## 7. Non-goals restated (still open after PR 2)

Still not done by either PR: run a 10k/100k pilot, replace the embedded
stock, touch `data/building_blocks.smi`, fetch or commit a large
external file, add CSV/SDF/gzip input support, or make any claim about
AiZynthFinder-scale comparison (issue #86). Each is real, named future
work — see the phase plan for sequencing.
