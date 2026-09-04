//! Deterministic, integrity-checked stock snapshots.
//!
//! A normal `.smi` stock is the source-of-truth interchange format. Loading it
//! requires parsing, standardizing, and canonicalizing every molecule. A
//! compiled `.rstock` snapshot stores the already-canonical identities and the
//! exact normalization contract that produced them, so [`crate::chem_env::ChemEnv`]
//! can rebuild its lookup set without repeating chemistry work on every process
//! start.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::stock_import::{
    MAX_STOCK_DATA_ROWS, MAX_STOCK_LINE_BYTES, NormalizationContract,
    current_normalization_contract,
};

pub const COMPILED_STOCK_SCHEMA_VERSION: u32 = 1;
pub const COMPILED_STOCK_MAGIC: &str = "RENKIN-COMPILED-STOCK-V1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledStockHeader {
    pub schema_version: u32,
    pub producer_version: String,
    pub normalization: NormalizationContract,
    /// Hash of the source `.smi` bytes before canonicalization.
    pub source_sha256: String,
    /// Hash of the exact newline-delimited payload bytes in this artifact.
    pub payload_sha256: String,
    /// Order-independent semantic hash used by `ChemEnv::content_sha256`.
    pub content_sha256: String,
    pub molecule_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedCompiledStock {
    pub header: CompiledStockHeader,
    pub canonical_smiles: Vec<String>,
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{}", crate::sha256_hex(Sha256::digest(bytes)))
}

fn validate_sha256(label: &str, value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        bail!("compiled stock {label} must start with 'sha256:'");
    };
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("compiled stock {label} is not a valid SHA-256 digest");
    }
    Ok(())
}

/// Computes the same order-independent semantic stock hash exposed by
/// `ChemEnv::content_sha256`.
pub fn semantic_content_sha256<'a>(entries: impl IntoIterator<Item = &'a str>) -> String {
    let mut sorted: Vec<&str> = entries.into_iter().collect();
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-retrospect-stock-v1\0");
    hasher.update((sorted.len() as u64).to_be_bytes());
    for smiles in sorted {
        hasher.update((smiles.len() as u64).to_be_bytes());
        hasher.update(smiles.as_bytes());
    }
    format!("sha256:{}", crate::sha256_hex(hasher.finalize()))
}

fn validate_canonical_entries(entries: &[String]) -> Result<()> {
    if entries.len() as u64 > MAX_STOCK_DATA_ROWS {
        bail!(
            "resource_exhausted: compiled stock exceeds {} molecules",
            MAX_STOCK_DATA_ROWS
        );
    }
    let mut previous: Option<&str> = None;
    for (index, entry) in entries.iter().enumerate() {
        if entry.is_empty() {
            bail!("compiled stock molecule {} is empty", index + 1);
        }
        if entry.len() > MAX_STOCK_LINE_BYTES {
            bail!(
                "resource_exhausted: compiled stock molecule {} exceeds {} bytes",
                index + 1,
                MAX_STOCK_LINE_BYTES
            );
        }
        if entry.bytes().any(|b| b.is_ascii_whitespace()) {
            bail!(
                "compiled stock molecule {} contains whitespace; expected one canonical SMILES",
                index + 1
            );
        }
        if let Some(prev) = previous
            && prev >= entry.as_str()
        {
            bail!(
                "compiled stock entries must be strictly sorted and unique (molecule {})",
                index + 1
            );
        }
        previous = Some(entry);
    }
    Ok(())
}

/// Renders a deterministic compiled artifact from a sorted, deduplicated list
/// produced by [`crate::stock_import::import_stock`].
pub fn render_compiled_stock(entries: &[String], source_sha256: &str) -> Result<Vec<u8>> {
    validate_sha256("source_sha256", source_sha256)?;
    validate_canonical_entries(entries)?;

    let mut payload = Vec::new();
    for entry in entries {
        payload.extend_from_slice(entry.as_bytes());
        payload.push(b'\n');
    }
    let header = CompiledStockHeader {
        schema_version: COMPILED_STOCK_SCHEMA_VERSION,
        producer_version: env!("CARGO_PKG_VERSION").to_string(),
        normalization: current_normalization_contract(),
        source_sha256: source_sha256.to_string(),
        payload_sha256: sha256_prefixed(&payload),
        content_sha256: semantic_content_sha256(entries.iter().map(String::as_str)),
        molecule_count: entries.len() as u64,
    };

    let mut output = Vec::new();
    output.extend_from_slice(COMPILED_STOCK_MAGIC.as_bytes());
    output.push(b'\n');
    output.extend_from_slice(
        serde_json::to_string(&header)
            .context("failed to serialize compiled stock header")?
            .as_bytes(),
    );
    output.push(b'\n');
    output.extend_from_slice(&payload);
    Ok(output)
}

/// Validates and decodes an artifact without parsing any molecule. Integrity,
/// normalization-policy equality, sorted uniqueness, and semantic identity are
/// all checked before the returned entries can become a `ChemEnv`.
pub fn decode_compiled_stock(bytes: &[u8]) -> Result<DecodedCompiledStock> {
    let first_newline = bytes
        .iter()
        .position(|b| *b == b'\n')
        .context("compiled stock is missing its magic-line terminator")?;
    if &bytes[..first_newline] != COMPILED_STOCK_MAGIC.as_bytes() {
        bail!("not a {COMPILED_STOCK_MAGIC} artifact");
    }
    let header_start = first_newline + 1;
    let header_end = bytes[header_start..]
        .iter()
        .position(|b| *b == b'\n')
        .map(|offset| header_start + offset)
        .context("compiled stock is missing its header-line terminator")?;
    if header_end - header_start > MAX_STOCK_LINE_BYTES {
        bail!(
            "resource_exhausted: compiled stock header exceeds {} bytes",
            MAX_STOCK_LINE_BYTES
        );
    }

    let header: CompiledStockHeader = serde_json::from_slice(&bytes[header_start..header_end])
        .context("compiled stock header is not valid JSON")?;
    if header.schema_version != COMPILED_STOCK_SCHEMA_VERSION {
        bail!(
            "unsupported compiled stock schema_version {} (expected {})",
            header.schema_version,
            COMPILED_STOCK_SCHEMA_VERSION
        );
    }
    if header.normalization != current_normalization_contract() {
        bail!(
            "compiled stock normalization contract differs from this RENKIN build; recompile the source stock"
        );
    }
    validate_sha256("source_sha256", &header.source_sha256)?;
    validate_sha256("payload_sha256", &header.payload_sha256)?;
    validate_sha256("content_sha256", &header.content_sha256)?;

    let payload = &bytes[header_end + 1..];
    let actual_payload_sha256 = sha256_prefixed(payload);
    if actual_payload_sha256 != header.payload_sha256 {
        bail!(
            "compiled stock payload SHA-256 mismatch: expected {}, got {}",
            header.payload_sha256,
            actual_payload_sha256
        );
    }
    if !payload.is_empty() && !payload.ends_with(b"\n") {
        bail!("compiled stock payload must end with a newline");
    }
    let payload_text =
        std::str::from_utf8(payload).context("compiled stock payload contains invalid UTF-8")?;
    let canonical_smiles: Vec<String> = payload_text.lines().map(str::to_owned).collect();
    validate_canonical_entries(&canonical_smiles)?;
    if canonical_smiles.len() as u64 != header.molecule_count {
        bail!(
            "compiled stock molecule_count mismatch: header says {}, payload has {}",
            header.molecule_count,
            canonical_smiles.len()
        );
    }
    let actual_content_sha256 =
        semantic_content_sha256(canonical_smiles.iter().map(String::as_str));
    if actual_content_sha256 != header.content_sha256 {
        bail!(
            "compiled stock semantic content SHA-256 mismatch: expected {}, got {}",
            header.content_sha256,
            actual_content_sha256
        );
    }

    Ok(DecodedCompiledStock {
        header,
        canonical_smiles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock_import::{StockImportOptions, import_stock};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn imported(input: &str) -> (Vec<String>, String) {
        let options = StockImportOptions {
            source_label: "compiled-stock-test".to_string(),
            source_revision: None,
            license: None,
        };
        let (entries, manifest) = import_stock(input.as_bytes(), &options).unwrap();
        (entries, manifest.input_sha256)
    }

    fn temp_path(extension: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "renkin-compiled-stock-{}-{count}.{extension}",
            std::process::id()
        ))
    }

    #[test]
    fn round_trip_is_deterministic_and_preserves_semantic_hash() {
        let (entries, source_sha256) = imported("CCO ethanol\nOCC duplicate\nCC(=O)O acetic\n");
        let a = render_compiled_stock(&entries, &source_sha256).unwrap();
        let b = render_compiled_stock(&entries, &source_sha256).unwrap();
        assert_eq!(a, b);

        let decoded = decode_compiled_stock(&a).unwrap();
        assert_eq!(decoded.canonical_smiles, entries);
        assert_eq!(
            decoded.header.content_sha256,
            semantic_content_sha256(entries.iter().map(String::as_str))
        );
    }

    #[test]
    fn rejects_corrupted_payload() {
        let (entries, source_sha256) = imported("CCO\nCC(=O)O\n");
        let mut artifact = render_compiled_stock(&entries, &source_sha256).unwrap();
        *artifact.last_mut().unwrap() = b'X';
        let error = decode_compiled_stock(&artifact).unwrap_err();
        assert!(error.to_string().contains("payload SHA-256 mismatch"));
    }

    #[test]
    fn rejects_different_normalization_contract() {
        let (entries, source_sha256) = imported("CCO\n");
        let artifact = render_compiled_stock(&entries, &source_sha256).unwrap();
        let mut lines = artifact.splitn(3, |b| *b == b'\n');
        let magic = lines.next().unwrap();
        let mut header: CompiledStockHeader =
            serde_json::from_slice(lines.next().unwrap()).unwrap();
        let payload = lines.next().unwrap();
        header.normalization.remove_explicit_h = !header.normalization.remove_explicit_h;

        let mut changed = Vec::new();
        changed.extend_from_slice(magic);
        changed.push(b'\n');
        changed.extend_from_slice(serde_json::to_string(&header).unwrap().as_bytes());
        changed.push(b'\n');
        changed.extend_from_slice(payload);

        let error = decode_compiled_stock(&changed).unwrap_err();
        assert!(error.to_string().contains("normalization contract differs"));
    }

    #[test]
    fn renderer_rejects_unsorted_or_duplicate_entries() {
        let source = format!("sha256:{}", "0".repeat(64));
        assert!(render_compiled_stock(&["O".into(), "C".into()], &source).is_err());
        assert!(render_compiled_stock(&["C".into(), "C".into()], &source).is_err());
    }

    #[test]
    fn plain_and_compiled_loads_have_identical_membership_and_content_hash() {
        let plain_path = temp_path("smi");
        let compiled_path = temp_path("rstock");
        let source = "CCO ethanol\nOCC duplicate\nCC(=O)O acetic\n";
        std::fs::write(&plain_path, source).unwrap();
        let (entries, source_sha256) = imported(source);
        std::fs::write(
            &compiled_path,
            render_compiled_stock(&entries, &source_sha256).unwrap(),
        )
        .unwrap();

        let plain = crate::chem_env::ChemEnv::load(plain_path.to_str().unwrap()).unwrap();
        let compiled = crate::chem_env::ChemEnv::load(compiled_path.to_str().unwrap()).unwrap();
        assert_eq!(plain.bb_count(), compiled.bb_count());
        assert_eq!(plain.content_sha256(), compiled.content_sha256());
        for smiles in ["CCO", "OCC", "CC(=O)O", "CCC"] {
            let molecule = crate::chem_env::mol_from_smiles(smiles).unwrap();
            assert_eq!(
                plain.is_building_block(&molecule),
                compiled.is_building_block(&molecule),
                "membership differs for {smiles}"
            );
        }

        std::fs::remove_file(plain_path).ok();
        std::fs::remove_file(compiled_path).ok();
    }
}
