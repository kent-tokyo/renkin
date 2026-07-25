use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};

/// How a step's metadata (today: `conditions`/`reaction_family`; future phases:
/// yield, references, warnings) was determined. Distinct from `step_confidence`/
/// `success_probability`, which are template-frequency-derived search-ranking
/// scores, not experimental measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataSource {
    /// Rule-author-supplied placeholder conditions (`conditions_for_rule` et al.) --
    /// a plausible default, not a measured or literature-sourced result. The only
    /// variant currently constructed by search itself; sidecar metadata typically
    /// uses `Literature` or `DatasetRecord`.
    HandcraftedDefault,
    /// Derived from a structured reaction dataset (sidecar metadata).
    DatasetRecord,
    /// Sourced from a cited paper or patent (sidecar metadata).
    Literature,
    /// Reserved for a later phase: output of a trained yield/condition-prediction model.
    ModelPrediction,
}

/// What scope a piece of step metadata was assigned at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceScope {
    /// Assigned to the reaction family as a whole (e.g. all Suzuki couplings) -- how
    /// hand-crafted-rule tags are scoped today.
    ReactionFamily,
    /// Assigned to one specific extracted SMIRKS template (sidecar metadata,
    /// keyed by `RetroRule::template_id`).
    Template,
    /// Reserved: assigned to this exact target/precursor substrate. No code path
    /// before per-substrate literature lookup exists can produce this.
    SubstrateSpecific,
}

/// Kind of external record an [`EvidenceReference`] points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Doi,
    Patent,
    Url,
    DatasetRecord,
}

/// A citable external source (paper, patent, URL, or dataset record) backing
/// one or more [`ConditionCandidate`]/[`ReportedYield`]/[`ReactionWarning`]
/// entries via `reference_ids`. `id` is scoped to the template it's declared
/// under in the metadata sidecar, not globally unique.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReference {
    pub id: String,
    pub kind: ReferenceKind,
    pub identifier: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
}

/// An inclusive numeric range (e.g. temperature in °C, time in hours, yield
/// percentage).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloatRange {
    pub min: f64,
    pub max: f64,
}

/// A reported yield's percentage: either a single measured value or a range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YieldPercentage {
    Single(f64),
    Range(FloatRange),
}

/// What the reported yield percentage was measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YieldBasis {
    Isolated,
    Assay,
    Conversion,
    Unknown,
}

/// Severity of a [`ReactionWarning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    Info,
    Low,
    Medium,
    High,
}

/// A curated set of reaction conditions reported for a template, sourced from
/// external evidence (never fabricated). Not a prediction -- see `source`/`scope`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionCandidate {
    #[serde(default)]
    pub catalysts: Vec<String>,
    #[serde(default)]
    pub reagents: Vec<String>,
    #[serde(default)]
    pub bases: Vec<String>,
    #[serde(default)]
    pub solvents: Vec<String>,
    /// Temperature range in degrees Celsius.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub temperature_c: Option<FloatRange>,
    /// Reaction time range in hours.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub time_hours: Option<FloatRange>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub atmosphere: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub notes: Option<String>,
    pub source: MetadataSource,
    pub scope: EvidenceScope,
    #[serde(default)]
    pub reference_ids: Vec<String>,
}

/// A yield reported for a template in external evidence. Not a RENKIN
/// prediction -- purely a curated record of what was reported.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedYield {
    pub percentage: YieldPercentage,
    pub basis: YieldBasis,
    pub source: MetadataSource,
    pub scope: EvidenceScope,
    #[serde(default)]
    pub reference_ids: Vec<String>,
}

/// A curated caveat about a template (e.g. a known side reaction), sourced
/// from external evidence. Not an automatic side-reaction detector -- only
/// what's explicitly present in the metadata sidecar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactionWarning {
    pub code: String,
    pub severity: WarningSeverity,
    pub message: String,
    pub source: MetadataSource,
    pub scope: EvidenceScope,
    #[serde(default)]
    pub reference_ids: Vec<String>,
}

/// Curated external evidence attached to one [`crate::search::ReactionStep`]
/// via its `template_id`. Absent (`None` on the step) unless a metadata
/// sidecar was supplied and matched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepEvidence {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub condition_candidates: Vec<ConditionCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reported_yields: Vec<ReportedYield>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<EvidenceReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ReactionWarning>,
}

impl StepEvidence {
    fn is_empty(&self) -> bool {
        self.condition_candidates.is_empty()
            && self.reported_yields.is_empty()
            && self.references.is_empty()
            && self.warnings.is_empty()
    }
}

/// One template's entry in a metadata sidecar file (`--template-metadata`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateMetadataEntry {
    #[serde(default)]
    pub references: Vec<EvidenceReference>,
    #[serde(default)]
    pub condition_candidates: Vec<ConditionCandidate>,
    #[serde(default)]
    pub reported_yields: Vec<ReportedYield>,
    #[serde(default)]
    pub warnings: Vec<ReactionWarning>,
}

impl TemplateMetadataEntry {
    /// Builds the `StepEvidence` to attach to a matching `ReactionStep`, or
    /// `None` if this entry carries no actual data (all four lists empty).
    pub fn to_step_evidence(&self) -> Option<StepEvidence> {
        let evidence = StepEvidence {
            condition_candidates: self.condition_candidates.clone(),
            reported_yields: self.reported_yields.clone(),
            references: self.references.clone(),
            warnings: self.warnings.clone(),
        };
        (!evidence.is_empty()).then_some(evidence)
    }
}

/// A metadata sidecar file (`--template-metadata <path>` / Python
/// `template_metadata_path`), keyed by `RetroRule::template_id`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateMetadataFile {
    pub schema_version: u32,
    #[serde(deserialize_with = "dedup_templates")]
    pub templates: HashMap<String, TemplateMetadataEntry>,
}

const SUPPORTED_SCHEMA_VERSIONS: &[u32] = &[1];

/// JSON objects silently keep the last value on a duplicate key -- both
/// `serde_json::Value` and a plain `HashMap` `Deserialize` impl already lose
/// that information by the time you have a parsed value. Detecting the
/// duplicate requires intercepting the live `MapAccess` during deserialization.
fn dedup_templates<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, TemplateMetadataEntry>, D::Error>
where
    D: Deserializer<'de>,
{
    struct TemplatesVisitor;

    impl<'de> Visitor<'de> for TemplatesVisitor {
        type Value = HashMap<String, TemplateMetadataEntry>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a JSON object mapping template_id to metadata, with no duplicate keys")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut out = HashMap::new();
            while let Some((k, v)) = map.next_entry::<String, TemplateMetadataEntry>()? {
                if out.insert(k.clone(), v).is_some() {
                    return Err(de::Error::custom(format!(
                        "duplicate template_id in metadata sidecar: {k:?}"
                    )));
                }
            }
            Ok(out)
        }
    }

    deserializer.deserialize_map(TemplatesVisitor)
}

/// Loads and validates a template metadata sidecar file. Fails loudly (before
/// any search runs) on malformed JSON or any of the checks in
/// `validate_template_metadata`.
pub fn load_template_metadata(path: &str) -> Result<TemplateMetadataFile> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read template metadata file {path}"))?;
    let file: TemplateMetadataFile = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse template metadata file {path}"))?;
    validate_template_metadata(&file)?;
    Ok(file)
}

fn validate_template_metadata(file: &TemplateMetadataFile) -> Result<()> {
    if !SUPPORTED_SCHEMA_VERSIONS.contains(&file.schema_version) {
        bail!(
            "unsupported template metadata schema_version {} (supported: {SUPPORTED_SCHEMA_VERSIONS:?})",
            file.schema_version
        );
    }

    for (template_id, entry) in &file.templates {
        let mut seen_ref_ids: HashSet<&str> = HashSet::new();
        for r in &entry.references {
            if !seen_ref_ids.insert(r.id.as_str()) {
                bail!(
                    "template {template_id:?}: duplicate reference id {:?}",
                    r.id
                );
            }
            if matches!(r.kind, ReferenceKind::Doi | ReferenceKind::Patent)
                && r.identifier.trim().is_empty()
            {
                bail!(
                    "template {template_id:?}: empty {:?} identifier for reference {:?}",
                    r.kind,
                    r.id
                );
            }
        }

        let check_reference_ids = |reference_ids: &[String], what: &str| -> Result<()> {
            for rid in reference_ids {
                if !seen_ref_ids.contains(rid.as_str()) {
                    bail!(
                        "template {template_id:?}: {what} references unknown reference id {rid:?}"
                    );
                }
            }
            Ok(())
        };

        for c in &entry.condition_candidates {
            check_reference_ids(&c.reference_ids, "condition_candidate")?;
            if let Some(r) = &c.temperature_c
                && r.min > r.max
            {
                bail!(
                    "template {template_id:?}: temperature_c range min {} > max {}",
                    r.min,
                    r.max
                );
            }
            if let Some(r) = &c.time_hours
                && r.min > r.max
            {
                bail!(
                    "template {template_id:?}: time_hours range min {} > max {}",
                    r.min,
                    r.max
                );
            }
        }

        for y in &entry.reported_yields {
            check_reference_ids(&y.reference_ids, "reported_yield")?;
            match &y.percentage {
                YieldPercentage::Single(p) => {
                    if !(0.0..=100.0).contains(p) {
                        bail!(
                            "template {template_id:?}: yield percentage {p} out of range [0, 100]"
                        );
                    }
                }
                YieldPercentage::Range(r) => {
                    if r.min > r.max {
                        bail!(
                            "template {template_id:?}: yield percentage range min {} > max {}",
                            r.min,
                            r.max
                        );
                    }
                    if !(0.0..=100.0).contains(&r.min) || !(0.0..=100.0).contains(&r.max) {
                        bail!(
                            "template {template_id:?}: yield percentage range [{}, {}] out of range [0, 100]",
                            r.min,
                            r.max
                        );
                    }
                }
            }
        }

        for w in &entry.warnings {
            check_reference_ids(&w.reference_ids, "warning")?;
        }
    }

    Ok(())
}

/// Warns (does not fail) about `template_id`s present in the metadata sidecar
/// that don't match any currently-loaded rule. Separate from
/// `validate_template_metadata` because it needs the loaded rule set, which
/// isn't known until after `load_template_metadata` returns.
pub fn warn_unknown_templates(file: &TemplateMetadataFile, known_template_ids: &HashSet<&str>) {
    for template_id in file.templates.keys() {
        if !known_template_ids.contains(template_id.as_str()) {
            eprintln!(
                "Warning: template metadata references unknown template_id {template_id:?} (no matching rule loaded)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_sidecar(dir: &std::path::Path, name: &str, content: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn valid_sidecar_loads_and_matches_expected_shape() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_valid.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "smirks-sha256:abc": {
                        "references": [{"id": "ref-1", "kind": "doi", "identifier": "10.xxxx/example"}],
                        "condition_candidates": [{
                            "catalysts": ["Pd(PPh3)4"],
                            "bases": ["K2CO3"],
                            "solvents": ["EtOH", "water"],
                            "temperature_c": {"min": 75.0, "max": 85.0},
                            "source": "literature",
                            "scope": "template",
                            "reference_ids": ["ref-1"]
                        }],
                        "reported_yields": [{
                            "percentage": {"min": 72.0, "max": 81.0},
                            "basis": "isolated",
                            "source": "literature",
                            "scope": "template",
                            "reference_ids": ["ref-1"]
                        }],
                        "warnings": [{
                            "code": "possible_protodeboronation",
                            "severity": "medium",
                            "message": "reported under prolonged aqueous heating",
                            "source": "literature",
                            "scope": "template",
                            "reference_ids": ["ref-1"]
                        }]
                    }
                }
            }"#,
        );
        let file = load_template_metadata(&path).unwrap();
        assert_eq!(file.schema_version, 1);
        let entry = file.templates.get("smirks-sha256:abc").unwrap();
        assert_eq!(entry.references.len(), 1);
        let evidence = entry.to_step_evidence().unwrap();
        assert_eq!(evidence.condition_candidates.len(), 1);
        assert_eq!(evidence.reported_yields.len(), 1);
        assert_eq!(evidence.warnings.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_entry_yields_no_step_evidence() {
        let entry = TemplateMetadataEntry::default();
        assert!(entry.to_step_evidence().is_none());
    }

    #[test]
    fn duplicate_template_id_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_dup_template.json",
            r#"{"schema_version": 1, "templates": {"smirks-sha256:abc": {}, "smirks-sha256:abc": {}}}"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("duplicate template_id"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn duplicate_reference_id_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_dup_ref.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {
                        "references": [
                            {"id": "ref-1", "kind": "doi", "identifier": "10.a/b"},
                            {"id": "ref-1", "kind": "doi", "identifier": "10.c/d"}
                        ]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("duplicate reference id"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn dangling_reference_id_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_dangling_ref.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {
                        "warnings": [{
                            "code": "x", "severity": "low", "message": "m",
                            "source": "literature", "scope": "template",
                            "reference_ids": ["missing-ref"]
                        }]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("unknown reference id"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn out_of_range_yield_percentage_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_bad_yield.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {
                        "reported_yields": [{
                            "percentage": 150.0,
                            "basis": "isolated",
                            "source": "literature",
                            "scope": "template"
                        }]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(format!("{err:#}").contains("out of range"), "got: {err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn range_min_greater_than_max_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_bad_range.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {
                        "condition_candidates": [{
                            "temperature_c": {"min": 90.0, "max": 80.0},
                            "source": "literature",
                            "scope": "template"
                        }]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("min") && format!("{err:#}").contains("max"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn typo_d_field_name_is_rejected_not_silently_ignored() {
        // A typo'd key (here "reported_yield" singular) must not silently
        // deserialize to an empty entry -- that would defeat the same
        // "don't silently ignore malformed metadata" goal as the other checks.
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_typo_field.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {
                        "reported_yield": [{
                            "percentage": 80.0,
                            "basis": "isolated",
                            "source": "literature",
                            "scope": "template"
                        }]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(format!("{err:#}").contains("unknown field"), "got: {err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_doi_identifier_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_empty_doi.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {"references": [{"id": "ref-1", "kind": "doi", "identifier": "   "}]}
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(format!("{err:#}").contains("empty"), "got: {err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_url_identifier_is_accepted() {
        // Intentional asymmetry per spec: only doi/patent identifiers must be non-empty.
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_empty_url.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {"references": [{"id": "ref-1", "kind": "url", "identifier": ""}]}
                }
            }"#,
        );
        assert!(load_template_metadata(&path).is_ok());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_bad_schema.json",
            r#"{"schema_version": 99, "templates": {}}"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(format!("{err:#}").contains("schema_version"), "got: {err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn unknown_template_id_warns_not_errors() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_unknown_template.json",
            r#"{"schema_version": 1, "templates": {"smirks-sha256:unknown": {}}}"#,
        );
        let file = load_template_metadata(&path).unwrap();
        let known: HashSet<&str> = HashSet::from(["rule:suzuki_retro"]);
        // Should not panic and should not affect the Result — just a stderr warning.
        warn_unknown_templates(&file, &known);
        std::fs::remove_file(&path).ok();
    }
}
