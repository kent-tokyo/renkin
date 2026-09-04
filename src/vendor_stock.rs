//! Vendor stock import and explicit match-mode classification (v0.38).
//!
//! This module is deliberately separate from the legacy `stock.csv` CLI
//! reader.  Vendor data is richer and must not silently become a building
//! block: callers choose the strongest acceptable [`MatchMode`] explicitly.
//! In particular, parent, stereo-ignored, and tautomer-related matches are
//! never reported as exact stock identity.

use anyhow::{Context, Result, anyhow, bail};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use chematic::chem::canonical::{CanonicalMode, canonical_smiles_mode};
use chematic::chem::standardize::standardize;
use chematic::inchi::{inchi, inchi_key};
use chematic::smiles::parse;

use crate::chem_env::{STANDARDIZE_OPTS, canonical_stock_identity_from_smiles};

pub const VENDOR_STOCK_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchMode {
    Exact,
    ParentIgnoringSalts,
    StereoIgnored,
    TautomerRelated,
}

impl MatchMode {
    pub fn rank(self) -> usize {
        match self {
            Self::Exact => 0,
            Self::ParentIgnoringSalts => 1,
            Self::StereoIgnored => 2,
            Self::TautomerRelated => 3,
        }
    }

    pub fn all() -> [Self; 4] {
        [
            Self::Exact,
            Self::ParentIgnoringSalts,
            Self::StereoIgnored,
            Self::TautomerRelated,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VendorStockRecord {
    pub id: Option<String>,
    pub smiles: String,
    pub vendor: Option<String>,
    pub price: Option<f64>,
    pub lead_time_days: Option<u32>,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VendorStockMatch {
    pub mode: MatchMode,
    pub record_indices: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
struct IdentityKeys {
    exact: String,
    parent: String,
    stereo_ignored: String,
    tautomer: String,
    inchi_key: String,
}

#[derive(Debug, Clone, Default)]
pub struct VendorStockIndex {
    records: Vec<VendorStockRecord>,
    keys: Vec<IdentityKeys>,
    by_inchi_key: FxHashMap<String, Vec<usize>>,
    by_mode: [FxHashMap<String, Vec<usize>>; 4],
}

impl VendorStockIndex {
    pub fn from_records(records: Vec<VendorStockRecord>) -> Result<Self> {
        let mut index = Self {
            records,
            ..Self::default()
        };
        for (record_index, record) in index.records.iter().enumerate() {
            let keys = identity_keys(&record.smiles)
                .with_context(|| format!("invalid vendor row {} SMILES", record_index + 1))?;
            index
                .by_inchi_key
                .entry(keys.inchi_key.clone())
                .or_default()
                .push(record_index);
            for (mode, key) in MatchMode::all().into_iter().zip([
                keys.exact.clone(),
                keys.parent.clone(),
                keys.stereo_ignored.clone(),
                keys.tautomer.clone(),
            ]) {
                index.by_mode[mode.rank()]
                    .entry(key)
                    .or_default()
                    .push(record_index);
            }
            index.keys.push(keys);
        }
        Ok(index)
    }

    pub fn records(&self) -> &[VendorStockRecord] {
        &self.records
    }

    pub fn unique_inchi_keys(&self) -> usize {
        self.by_inchi_key.len()
    }

    /// Looks up at most the requested mode. `Exact` is the safe default.
    /// Results are always returned at the strongest matching mode, in the
    /// fixed priority order exact > parent > stereo-ignored > tautomer.
    pub fn lookup(&self, smiles: &str, max_mode: MatchMode) -> Result<Option<VendorStockMatch>> {
        let query = identity_keys(smiles).context("invalid query SMILES")?;
        for mode in MatchMode::all() {
            if mode.rank() > max_mode.rank() {
                break;
            }
            let key = match mode {
                MatchMode::Exact => &query.exact,
                MatchMode::ParentIgnoringSalts => &query.parent,
                MatchMode::StereoIgnored => &query.stereo_ignored,
                MatchMode::TautomerRelated => &query.tautomer,
            };
            if let Some(indices) = self.by_mode[mode.rank()].get(key) {
                return Ok(Some(VendorStockMatch {
                    mode,
                    record_indices: indices.clone(),
                }));
            }
        }
        Ok(None)
    }

    /// The pure-Rust InChIKey candidate index used for fast exact-candidate
    /// retrieval. Classification still uses the explicit canonical keys.
    pub fn inchi_candidates(&self, smiles: &str) -> Result<Vec<usize>> {
        let query = identity_keys(smiles).context("invalid query SMILES")?;
        Ok(self
            .by_inchi_key
            .get(&query.inchi_key)
            .cloned()
            .unwrap_or_default())
    }
}

fn identity_keys(smiles: &str) -> Result<IdentityKeys> {
    let molecule = parse(smiles).map_err(|e| anyhow!("failed to parse SMILES: {e}"))?;
    let standardized = standardize(&molecule, &STANDARDIZE_OPTS);
    let exact = canonical_stock_identity_from_smiles(smiles)?;
    let parent = smiles
        .split('.')
        .map(|fragment| {
            let key = canonical_stock_identity_from_smiles(fragment)?;
            let atom_count = parse(fragment)?.atom_count();
            Ok((atom_count, key))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max_by_key(|(atom_count, key)| (*atom_count, key.clone()))
        .map(|(_, key)| key)
        .unwrap_or_else(|| exact.clone());
    Ok(IdentityKeys {
        exact,
        parent,
        stereo_ignored: canonical_smiles_mode(&standardized, CanonicalMode::NoStereo),
        tautomer: canonical_smiles_mode(&standardized, CanonicalMode::Tautomer),
        inchi_key: inchi_key(&inchi(&standardized)),
    })
}

/// Import a CSV or TSV table. Required column: `smiles`. Optional aliases:
/// `id`, `vendor`, `price`, `lead_time_days`/`lead_time`, and `available`.
pub fn import_vendor_table(input: &str, delimiter: Option<u8>) -> Result<Vec<VendorStockRecord>> {
    let delimiter = delimiter.unwrap_or_else(|| detect_delimiter(input));
    let mut rows = input.lines().enumerate().filter(|(_, line)| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#')
    });
    let (_, header_line) = rows
        .next()
        .ok_or_else(|| anyhow!("vendor table is empty"))?;
    let headers = parse_row(header_line, delimiter)?
        .into_iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let col = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| headers.iter().position(|h| h == name))
    };
    let smiles_col = col(&["smiles", "canonical_smiles"])
        .ok_or_else(|| anyhow!("vendor table requires a smiles column"))?;
    let id_col = col(&["id", "vendor_id", "catalog_id"]);
    let vendor_col = col(&["vendor", "supplier"]);
    let price_col = col(&["price", "price_jpy", "price_usd"]);
    let lead_col = col(&["lead_time_days", "lead_time"]);
    let available_col = col(&["available", "in_stock"]);
    let mut records = Vec::new();
    for (line_no, line) in rows {
        let fields = parse_row(line, delimiter)
            .with_context(|| format!("invalid vendor row {}", line_no + 1))?;
        let field = |position: Option<usize>| {
            position
                .and_then(|i| fields.get(i))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
        };
        let smiles = field(Some(smiles_col))
            .ok_or_else(|| anyhow!("row {} has empty smiles", line_no + 1))?
            .to_owned();
        let price = field(price_col)
            .map(|s| {
                s.parse::<f64>()
                    .with_context(|| format!("row {} has invalid price", line_no + 1))
            })
            .transpose()?;
        let lead_time_days = field(lead_col)
            .map(|s| {
                s.parse::<u32>()
                    .with_context(|| format!("row {} has invalid lead time", line_no + 1))
            })
            .transpose()?;
        let available = field(available_col)
            .map(|s| match s.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "y" => Ok(true),
                "false" | "0" | "no" | "n" => Ok(false),
                _ => Err(anyhow!("invalid availability")),
            })
            .transpose()
            .with_context(|| format!("row {} has invalid available value", line_no + 1))?
            .unwrap_or(true);
        records.push(VendorStockRecord {
            id: field(id_col).map(str::to_owned),
            smiles,
            vendor: field(vendor_col).map(str::to_owned),
            price,
            lead_time_days,
            available,
        });
    }
    Ok(records)
}

fn detect_delimiter(input: &str) -> u8 {
    input
        .lines()
        .find(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .map(|line| {
            if line.matches('\t').count() > line.matches(',').count() {
                b'\t'
            } else {
                b','
            }
        })
        .unwrap_or(b',')
}

fn parse_row(line: &str, delimiter: u8) -> Result<Vec<String>> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let delimiter = delimiter as char;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ch if ch == delimiter && !quoted => {
                fields.push(std::mem::take(&mut field));
            }
            _ => field.push(ch),
        }
    }
    if quoted {
        bail!("unterminated quoted field");
    }
    fields.push(field);
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_csv_and_tsv_with_metadata() {
        let csv =
            "smiles,id,price,vendor,lead_time_days,available\n\"CCO\",ethanol,12.5,Acme,3,yes\n";
        let records = import_vendor_table(csv, None).unwrap();
        assert_eq!(records[0].id.as_deref(), Some("ethanol"));
        assert_eq!(records[0].price, Some(12.5));
        assert_eq!(records[0].lead_time_days, Some(3));
        let tsv = "smiles\tid\nCCO\tE1\n";
        assert_eq!(
            import_vendor_table(tsv, None).unwrap()[0].id.as_deref(),
            Some("E1")
        );
    }

    #[test]
    fn exact_is_default_and_relaxed_modes_are_explicit() {
        let index = VendorStockIndex::from_records(vec![VendorStockRecord {
            id: Some("1".into()),
            smiles: "CCO".into(),
            vendor: None,
            price: None,
            lead_time_days: None,
            available: true,
        }])
        .unwrap();
        assert_eq!(
            index.lookup("CCO", MatchMode::Exact).unwrap().unwrap().mode,
            MatchMode::Exact
        );
        assert!(index.lookup("CCO.O", MatchMode::Exact).unwrap().is_none());
        assert_eq!(
            index
                .lookup("CCO.O", MatchMode::ParentIgnoringSalts)
                .unwrap()
                .unwrap()
                .mode,
            MatchMode::ParentIgnoringSalts
        );
    }

    #[test]
    fn stereo_and_tautomer_matches_do_not_upgrade_to_exact() {
        let index = VendorStockIndex::from_records(vec![VendorStockRecord {
            id: None,
            smiles: "N[C@H](C)C(=O)O".into(),
            vendor: None,
            price: None,
            lead_time_days: None,
            available: true,
        }])
        .unwrap();
        assert!(
            index
                .lookup("N[C@@H](C)C(=O)O", MatchMode::Exact)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            index
                .lookup("N[C@@H](C)C(=O)O", MatchMode::StereoIgnored)
                .unwrap()
                .unwrap()
                .mode,
            MatchMode::StereoIgnored
        );
    }

    #[test]
    fn duplicate_vendor_rows_are_preserved() {
        let index = VendorStockIndex::from_records(vec![
            VendorStockRecord {
                id: Some("a".into()),
                smiles: "CCO".into(),
                vendor: Some("A".into()),
                price: None,
                lead_time_days: None,
                available: true,
            },
            VendorStockRecord {
                id: Some("b".into()),
                smiles: "OCC".into(),
                vendor: Some("B".into()),
                price: None,
                lead_time_days: None,
                available: true,
            },
        ])
        .unwrap();
        assert_eq!(
            index
                .lookup("CCO", MatchMode::Exact)
                .unwrap()
                .unwrap()
                .record_indices,
            vec![0, 1]
        );
    }
}
