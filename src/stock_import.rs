//! Deterministic stock importer core (v0.36.0 Phase 2, PR 1).
//!
//! Reads a line-oriented `.smi` stock file, applies RENKIN's own single
//! stock-identity policy (`chem_env::canonical_stock_identity_from_smiles`,
//! i.e. the same standardize-then-canonicalize rule `ChemEnv::load` itself
//! uses), and produces a deterministically sorted, deduped compound list
//! plus a versioned provenance manifest. Unlike `ChemEnv::load`/
//! `from_smiles_iter` (which silently `continue`s past both unparseable
//! lines and duplicates, with no count surfaced anywhere), every rejected
//! row and every duplicate is recorded with a reason.
//!
//! Scope for this PR: `.smi` input only (SMILES as the first
//! whitespace-separated token per line; any remaining tokens -- a name, a
//! price column, ... -- are ignored; `#`-prefixed and blank lines are
//! skipped as non-data rows, matching `ChemEnv::load`'s own convention for
//! this exact file format). Streaming, single-pass. No CSV/SDF, no CLI
//! surface, no large-file fetch, no default-stock replacement -- see
//! `docs/design/stock-import-v0.md` for the full contract and what's
//! deliberately deferred to a later PR.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::chem_env::{STANDARDIZE_OPTS, canonical_stock_identity_from_smiles};

/// Bumped whenever `StockManifest`'s shape changes in a way a consumer
/// must not silently treat as the same schema (same discipline as
/// `pool_export::MANIFEST_SCHEMA_VERSION`).
pub const STOCK_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const MAX_STOCK_INPUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_STOCK_LINE_BYTES: usize = 64 * 1024;
pub const MAX_STOCK_DATA_ROWS: u64 = 1_000_000;

/// Caller-supplied provenance for one import run. Never guessed: a
/// missing `source_revision`/`license` is recorded as `None`, not
/// invented -- mirrors `pool_export::build_manifest`'s own
/// never-guess-a-label discipline for `stock_identity`.
#[derive(Debug, Clone)]
pub struct StockImportOptions {
    pub source_label: String,
    pub source_revision: Option<String>,
    pub license: Option<String>,
}

/// Why one input row was rejected outright -- never included in the
/// output stock, but always counted and recorded here, never silently
/// dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    /// The row's SMILES field is present but empty. Unreachable for this
    /// PR's whitespace-tokenized `.smi` format specifically (a non-blank
    /// trimmed line always yields a non-empty first token -- see the
    /// `expect` at the call site), but kept as a real variant since a
    /// future delimited format (CSV) can have a genuinely empty field
    /// between two separators, and this enum is shared across formats.
    EmptyField,
    /// RENKIN's own SMILES parser rejected the token outright.
    UnparseableSmiles,
}

impl RejectionReason {
    fn as_str(self) -> &'static str {
        match self {
            RejectionReason::EmptyField => "empty_field",
            RejectionReason::UnparseableSmiles => "unparseable_smiles",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedRow {
    pub line_no: u64,
    pub smiles: String,
    pub reason: RejectionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateRow {
    pub line_no: u64,
    pub canonical_smiles: String,
    pub duplicate_of_line_no: u64,
}

/// Snapshot of the exact standardization policy applied, so a manifest
/// reader never has to trust an unstated default. Mirrors
/// `chem_env::STANDARDIZE_OPTS` field-for-field, built fresh from that
/// static every time via [`current_normalization_contract`] -- never
/// hand-duplicated, so it can't silently drift out of sync with the real
/// policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizationContract {
    pub canonical_tautomer: bool,
    pub neutralize_charges: bool,
    pub remove_explicit_h: bool,
    pub largest_fragment_only: bool,
    pub zwitterion_handling: String,
}

/// Builds a fresh snapshot of the normalization policy this build of
/// RENKIN actually applies right now (from the live `STANDARDIZE_OPTS`,
/// never hand-duplicated). `renkin doctor stock` calls this directly to
/// compare a manifest's recorded `normalization` against what the
/// *currently running* binary would produce, independent of whatever
/// policy was live when the manifest was originally generated.
pub fn current_normalization_contract() -> NormalizationContract {
    NormalizationContract {
        canonical_tautomer: STANDARDIZE_OPTS.canonical_tautomer,
        neutralize_charges: STANDARDIZE_OPTS.neutralize_charges,
        remove_explicit_h: STANDARDIZE_OPTS.remove_explicit_h,
        largest_fragment_only: STANDARDIZE_OPTS.largest_fragment_only,
        zwitterion_handling: format!("{:?}", STANDARDIZE_OPTS.zwitterion_handling),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockSource {
    pub label: String,
    pub revision: Option<String>,
    pub license: Option<String>,
}

/// Full provenance record for one import run. `input_rows`,
/// `accepted_rows` (rows whose SMILES token parsed successfully --
/// includes rows later folded into `duplicate_rows`, since parsing and
/// deduping are separate questions), `rejected_rows`, `unique_structures`,
/// and `duplicate_rows` always satisfy two arithmetic identities, checked
/// by this module's own tests rather than left as an implicit invariant:
/// `input_rows == accepted_rows + rejected_rows` and
/// `accepted_rows == unique_structures + duplicate_rows`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockManifest {
    pub schema_version: u32,
    pub source: StockSource,
    pub importer_version: String,
    /// Whole-input SHA-256, `sha256:<hex>`.
    pub input_sha256: String,
    /// SHA-256 of the exact bytes [`render_output`] produces for the
    /// accepted list, `sha256:<hex>` -- matches an output `.smi` file
    /// written from that same list byte-for-byte.
    pub output_sha256: String,
    pub input_rows: u64,
    pub accepted_rows: u64,
    pub rejected_rows: u64,
    pub unique_structures: u64,
    pub duplicate_rows: u64,
    pub rejection_reasons: BTreeMap<String, u64>,
    pub rejected: Vec<RejectedRow>,
    pub duplicates: Vec<DuplicateRow>,
    pub normalization: NormalizationContract,
}

/// Streaming SHA-256 pass-through: hashes every byte read without
/// buffering the whole input separately, so `input_sha256` doesn't cost a
/// second full read of a large file.
struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

fn read_bounded_line<R: BufRead>(reader: &mut R, total_bytes: &mut u64) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf().context("failed reading stock input")?;
        if buffer.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(buffer.len());
        let consumed = newline.map_or(buffer.len(), |index| index + 1);
        if *total_bytes + consumed as u64 > MAX_STOCK_INPUT_BYTES {
            anyhow::bail!(
                "resource_exhausted: stock input exceeds {} bytes",
                MAX_STOCK_INPUT_BYTES
            );
        }
        if line.len() + content_len > MAX_STOCK_LINE_BYTES {
            anyhow::bail!(
                "resource_exhausted: stock line exceeds {} bytes",
                MAX_STOCK_LINE_BYTES
            );
        }
        line.extend_from_slice(&buffer[..content_len]);
        reader.consume(consumed);
        *total_bytes += consumed as u64;
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

/// Imports a line-oriented `.smi` stock file from any `Read`. Returns the
/// deterministically sorted, deduped list of accepted canonical SMILES
/// (the exact content [`render_output`] would write to an output `.smi`)
/// plus the full manifest. Same input bytes + same `options` always
/// produce the same output list and the same manifest once serialized --
/// nothing here reads a clock, a random source, or relies on hashmap
/// iteration order for anything that ends up in either.
pub fn import_stock(
    input: impl Read,
    options: &StockImportOptions,
) -> Result<(Vec<String>, StockManifest)> {
    let hashing = HashingReader {
        inner: input,
        hasher: Sha256::new(),
    };
    let mut reader = BufReader::new(hashing);

    let mut accepted: Vec<String> = Vec::new();
    let mut rejected: Vec<RejectedRow> = Vec::new();
    let mut duplicates: Vec<DuplicateRow> = Vec::new();
    let mut seen: FxHashMap<String, u64> = FxHashMap::default();
    let mut input_rows: u64 = 0;

    let mut line_no: u64 = 0;
    let mut total_bytes: u64 = 0;
    loop {
        let Some(line_bytes) = read_bounded_line(&mut reader, &mut total_bytes)? else {
            break;
        };
        let line = String::from_utf8(line_bytes).context("stock input contains invalid UTF-8")?;
        line_no += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue; // comment / blank line: not a data row, not counted
        }
        input_rows += 1;
        if input_rows > MAX_STOCK_DATA_ROWS {
            anyhow::bail!(
                "resource_exhausted: stock input exceeds {} data rows",
                MAX_STOCK_DATA_ROWS
            );
        }

        // `trimmed` is non-empty (checked above), so this is always
        // `Some` for this whitespace-tokenized format -- see
        // `RejectionReason::EmptyField`'s doc comment.
        let token = trimmed
            .split_whitespace()
            .next()
            .expect("trimmed is non-empty, checked above");

        match canonical_stock_identity_from_smiles(token) {
            Ok(canonical) => {
                if let Some(&first_line) = seen.get(&canonical) {
                    duplicates.push(DuplicateRow {
                        line_no,
                        canonical_smiles: canonical,
                        duplicate_of_line_no: first_line,
                    });
                } else {
                    seen.insert(canonical.clone(), line_no);
                    accepted.push(canonical);
                }
            }
            Err(_) => {
                rejected.push(RejectedRow {
                    line_no,
                    smiles: token.to_string(),
                    reason: RejectionReason::UnparseableSmiles,
                });
            }
        }
    }

    // Deterministic output regardless of input row order. `seen` already
    // guarantees `accepted` has no duplicates; the `dedup()` is a cheap
    // defensive no-op, not load-bearing.
    accepted.sort_unstable();
    accepted.dedup();

    let input_sha256 = format!(
        "sha256:{}",
        crate::sha256_hex(reader.into_inner().hasher.finalize())
    );

    let output_bytes = render_output(&accepted);
    let output_sha256 = format!(
        "sha256:{}",
        crate::sha256_hex(Sha256::digest(&output_bytes))
    );

    let mut rejection_reasons: BTreeMap<String, u64> = BTreeMap::new();
    for row in &rejected {
        *rejection_reasons
            .entry(row.reason.as_str().to_string())
            .or_insert(0) += 1;
    }

    let rejected_rows = rejected.len() as u64;
    let duplicate_rows = duplicates.len() as u64;
    let unique_structures = accepted.len() as u64;
    let accepted_rows = unique_structures + duplicate_rows;

    let manifest = StockManifest {
        schema_version: STOCK_MANIFEST_SCHEMA_VERSION,
        source: StockSource {
            label: options.source_label.clone(),
            revision: options.source_revision.clone(),
            license: options.license.clone(),
        },
        importer_version: env!("CARGO_PKG_VERSION").to_string(),
        input_sha256,
        output_sha256,
        input_rows,
        accepted_rows,
        rejected_rows,
        unique_structures,
        duplicate_rows,
        rejection_reasons,
        rejected,
        duplicates,
        normalization: current_normalization_contract(),
    };

    Ok((accepted, manifest))
}

/// Diagnostic-only bridge: re-applies the exact same stock-identity
/// canonicalization `import_stock` itself uses to a single SMILES string.
/// Exists only so out-of-crate diagnostic tooling (e.g. an `examples/`
/// probe) can reuse the real canonicalization path -- which is
/// `pub(crate)` -- instead of re-deriving it, which would risk the probe
/// silently testing different logic than what actually ships. Not part of
/// the documented import/manifest API and not wired into the CLI; added
/// for the v0.36.0 Phase 2 PR 2 `reimport_idempotency` investigation.
pub fn recanonicalize_stock_smiles(smiles: &str) -> Result<String> {
    canonical_stock_identity_from_smiles(smiles)
}

/// Convenience wrapper: import directly from a file path.
pub fn import_stock_from_path(
    path: &Path,
    options: &StockImportOptions,
) -> Result<(Vec<String>, StockManifest)> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open stock file: {}", path.display()))?;
    import_stock(file, options)
}

/// Exact bytes an output `.smi` file would contain for `accepted`: one
/// canonical SMILES per line, in the order given (callers pass the
/// already-sorted list from [`import_stock`]), trailing newline. Shared
/// by `import_stock` (for `output_sha256`) and any caller that writes the
/// accepted list to disk, so the hash always matches what's actually
/// written.
pub fn render_output(accepted: &[String]) -> Vec<u8> {
    let mut out = String::new();
    for smi in accepted {
        out.push_str(smi);
        out.push('\n');
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> StockImportOptions {
        StockImportOptions {
            source_label: "test-fixture".to_string(),
            source_revision: None,
            license: None,
        }
    }

    #[test]
    fn empty_input_produces_zero_everything() {
        let (accepted, manifest) = import_stock(std::io::empty(), &opts()).unwrap();
        assert!(accepted.is_empty());
        assert_eq!(manifest.input_rows, 0);
        assert_eq!(manifest.accepted_rows, 0);
        assert_eq!(manifest.rejected_rows, 0);
        assert_eq!(manifest.unique_structures, 0);
        assert_eq!(manifest.duplicate_rows, 0);
        assert!(manifest.rejection_reasons.is_empty());
    }

    #[test]
    fn comments_and_blank_lines_are_skipped_not_counted() {
        let input = "# a comment\n\n   \nCCO ethanol\n# another\n";
        let (accepted, manifest) = import_stock(input.as_bytes(), &opts()).unwrap();
        assert_eq!(accepted.len(), 1);
        // 5 raw lines, but only 1 is a data row -- comments/blanks never
        // reach `input_rows`.
        assert_eq!(manifest.input_rows, 1);
        assert_eq!(manifest.accepted_rows, 1);
    }

    #[test]
    fn trailing_name_column_is_ignored() {
        let input = "CCO ethanol some extra metadata columns\n";
        let (accepted, _manifest) = import_stock(input.as_bytes(), &opts()).unwrap();
        assert_eq!(accepted.len(), 1);
        // Only the first whitespace-separated token was used as the SMILES.
        assert!(!accepted[0].contains("ethanol"));
    }

    #[test]
    fn oversized_stock_line_is_rejected_before_smiles_parsing() {
        let input = format!("{}\n", "C".repeat(MAX_STOCK_LINE_BYTES + 1));
        let error = import_stock(input.as_bytes(), &opts())
            .expect_err("oversized stock line must be rejected");
        assert!(error.to_string().contains("resource_exhausted"));
    }

    #[test]
    fn invalid_utf8_stock_input_is_rejected() {
        let error = import_stock([0xff, 0xfe].as_slice(), &opts())
            .expect_err("invalid UTF-8 stock input must be rejected");
        assert!(error.to_string().contains("invalid UTF-8"));
    }

    #[test]
    fn unparseable_smiles_is_rejected_with_reason_and_processing_continues() {
        let input = "CCO\nnot(a valid smiles(((\nCC(=O)O\n";
        let (accepted, manifest) = import_stock(input.as_bytes(), &opts()).unwrap();
        assert_eq!(
            accepted.len(),
            2,
            "the two valid rows must still be accepted"
        );
        assert_eq!(manifest.rejected.len(), 1);
        assert_eq!(manifest.rejected[0].line_no, 2);
        assert_eq!(
            manifest.rejected[0].reason,
            RejectionReason::UnparseableSmiles
        );
        assert_eq!(
            manifest.rejection_reasons.get("unparseable_smiles"),
            Some(&1)
        );
    }

    #[test]
    fn duplicate_by_canonical_identity_is_recorded_not_silently_dropped() {
        // CCO and OCC are the same molecule (ethanol) under different
        // SMILES spellings -- must canonicalize to the same identity.
        let input = "CCO\nOCC\nCC(=O)O\n";
        let (accepted, manifest) = import_stock(input.as_bytes(), &opts()).unwrap();
        assert_eq!(accepted.len(), 2, "ethanol counted once, acetic acid once");
        assert_eq!(manifest.duplicates.len(), 1);
        assert_eq!(manifest.duplicates[0].line_no, 2);
        assert_eq!(manifest.duplicates[0].duplicate_of_line_no, 1);
    }

    #[test]
    fn accepted_rejected_duplicate_arithmetic_is_internally_consistent() {
        let input = "CCO\nOCC\nCC(=O)O\nnot(valid(((\nCCC\nCCC\n";
        let (_accepted, m) = import_stock(input.as_bytes(), &opts()).unwrap();
        assert_eq!(
            m.input_rows,
            m.accepted_rows + m.rejected_rows,
            "every input row is either accepted (parsed) or rejected, never both or neither"
        );
        assert_eq!(
            m.accepted_rows,
            m.unique_structures + m.duplicate_rows,
            "every accepted (parsed) row is either the first sighting of a structure or a duplicate of one"
        );
    }

    /// `HashingReader` hashes bytes as `BufReader` pulls them from the
    /// underlying source, not per logical line -- this proves that
    /// plumbing actually reproduces a plain, independent SHA-256 of the
    /// exact same bytes, not just "the same value across two runs of our
    /// own code" (which the determinism test above would pass even if
    /// `HashingReader` had a systematic bug, as long as it were
    /// consistent). Covers both a trailing-newline and a no-trailing-
    /// newline input, since EOF handling is exactly where a streaming
    /// hasher is most likely to drop or double-count a byte.
    #[test]
    fn input_sha256_matches_an_independently_computed_hash_of_the_same_bytes() {
        for input in [
            "CCO ethanol\nOCC dup\nnot(valid(((\nCC(=O)O acetic\n",
            "CCO ethanol\nOCC dup\nnot(valid(((\nCC(=O)O acetic",
        ] {
            let (_accepted, manifest) = import_stock(input.as_bytes(), &opts()).unwrap();
            let expected = format!(
                "sha256:{}",
                crate::sha256_hex(Sha256::digest(input.as_bytes()))
            );
            assert_eq!(manifest.input_sha256, expected);
        }
    }

    #[test]
    fn same_input_and_options_produce_byte_identical_output_and_manifest() {
        let input = "CCO ethanol\nOCC dup-of-ethanol\nCC(=O)O acetic acid\nnot(valid(((\n";
        let (accepted_a, manifest_a) = import_stock(input.as_bytes(), &opts()).unwrap();
        let (accepted_b, manifest_b) = import_stock(input.as_bytes(), &opts()).unwrap();
        assert_eq!(render_output(&accepted_a), render_output(&accepted_b));
        assert_eq!(manifest_a, manifest_b);
        // Also stable across two independent JSON serializations, not
        // just equal as Rust values -- catches a hidden HashMap-ordering
        // dependency that `assert_eq!` on the struct alone could miss if
        // a future field ever added one.
        assert_eq!(
            serde_json::to_string_pretty(&manifest_a).unwrap(),
            serde_json::to_string_pretty(&manifest_b).unwrap()
        );
    }

    #[test]
    fn output_is_sorted_regardless_of_input_order() {
        let forward = "CCC propane\nCCO ethanol\nC methane\n";
        let backward = "C methane\nCCO ethanol\nCCC propane\n";
        let (a, _) = import_stock(forward.as_bytes(), &opts()).unwrap();
        let (b, _) = import_stock(backward.as_bytes(), &opts()).unwrap();
        assert_eq!(a, b, "output must not depend on input row order");
        let mut sorted_a = a.clone();
        sorted_a.sort_unstable();
        assert_eq!(a, sorted_a, "output must already be sorted");
    }

    #[test]
    fn normalization_contract_matches_the_real_standardize_opts() {
        let (_accepted, manifest) = import_stock(std::io::empty(), &opts()).unwrap();
        assert_eq!(
            manifest.normalization.remove_explicit_h,
            STANDARDIZE_OPTS.remove_explicit_h
        );
        assert_eq!(
            manifest.normalization.zwitterion_handling,
            format!("{:?}", STANDARDIZE_OPTS.zwitterion_handling)
        );
    }

    /// `StockManifest` gained `Deserialize` in v0.36.0 Phase 2 PR 2 so
    /// `renkin doctor stock` can read a manifest a prior `renkin stock
    /// import` run wrote -- round-trips through the exact JSON text a
    /// file on disk would contain (not just `serde_json::Value`) to
    /// confirm the derive actually mirrors `Serialize` field-for-field.
    #[test]
    fn manifest_round_trips_through_json_deserialize() {
        let input = "CCO ethanol\nOCC dup\nnot(valid(((\n";
        let mut options = opts();
        options.source_revision = Some("rev-1".to_string());
        options.license = Some("CC0".to_string());
        let (_accepted, manifest) = import_stock(input.as_bytes(), &options).unwrap();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let round_tripped: StockManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, round_tripped);
    }

    /// Fixture-consistency check requested for v0.36.0 Phase 2 PR 1: import
    /// the real, already-committed `data/building_blocks.smi` through this
    /// new importer and confirm its unique-structure count agrees with
    /// `ChemEnv::load`'s own `bb_count()` on the exact same file -- both
    /// apply `chem_env::canonical_stock_identity`, so they must agree if
    /// this importer's dedup logic is genuinely equivalent, not just
    /// similar. Does NOT modify `data/building_blocks.smi` itself; read-only.
    #[test]
    fn import_of_real_building_blocks_fixture_matches_chem_env_load_unique_count() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/building_blocks.smi");
        let (accepted, manifest) = import_stock_from_path(&path, &opts())
            .expect("data/building_blocks.smi must exist and be importable");

        let loaded = crate::chem_env::ChemEnv::load(path.to_str().unwrap())
            .expect("ChemEnv::load must succeed on the same file");

        assert_eq!(
            manifest.unique_structures as usize,
            loaded.bb_count(),
            "the new importer's unique-structure count must agree with ChemEnv::load's \
             bb_count() on the same file -- both apply canonical_stock_identity, so a \
             mismatch here means the two code paths have silently diverged"
        );
        assert_eq!(accepted.len(), loaded.bb_count());

        // Document (not assert against a magic number): how many rows
        // were rejected as unparseable, and how many were in-file
        // duplicates. These figures are allowed to differ from any
        // previously-published number (e.g. this repo's own docs once
        // quoted "402 unique / 3 parse failures" from an unverified
        // one-off measurement) -- this test's job is to make the current,
        // reproducible numbers visible and self-consistent, not to lock
        // in a specific historical figure.
        eprintln!(
            "data/building_blocks.smi via stock_import: input_rows={}, unique_structures={}, \
             rejected_rows={}, duplicate_rows={}, rejection_reasons={:?}",
            manifest.input_rows,
            manifest.unique_structures,
            manifest.rejected_rows,
            manifest.duplicate_rows,
            manifest.rejection_reasons
        );
    }
}
