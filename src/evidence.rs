use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::chem_env::{mol_from_smiles, to_canonical};

/// How a step's metadata (`conditions`/`reaction_family`, and -- via
/// `StepEvidence` -- yield/references/warnings/examples) was determined.
/// Distinct from `step_confidence`/`success_probability`, which are
/// template-frequency-derived search-ranking scores, not experimental
/// measurements.
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
    /// Assigned to this exact target/precursor substrate. Required scope for
    /// every condition/yield/warning nested inside a `ReactionExample`
    /// (schema_version 2+); rejected at any other scope there.
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

/// One concrete literature/dataset example of this template being run on a
/// specific substrate: "this target, from these precursors, under these
/// conditions, reportedly in this yield" (schema_version 2+ only). Unlike
/// the template-scoped `condition_candidates`/`reported_yields`/`warnings`
/// above, every nested condition/yield/warning here must be scoped
/// `substrate_specific` -- see `validate_template_metadata`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactionExample {
    pub id: String,

    /// Forward-direction product, corresponding to ReactionStep.target.
    pub target_smiles: String,

    /// Forward-direction reactants, corresponding to ReactionStep.precursors.
    pub precursor_smiles: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub conditions: Option<ConditionCandidate>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reported_yield: Option<ReportedYield>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ReactionWarning>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_ids: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub dataset_record_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub notes: Option<String>,
}

/// How closely a curated [`ReactionExample`] matches a specific route step's
/// exact substrate -- see [`match_example`]. Serialized onto
/// [`ResolvedReactionExample`] so JSON/Python consumers, not just
/// `--format explain`, can tell an exact-substrate match from a
/// same-template literature precedent without re-canonicalizing SMILES
/// themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExampleMatch {
    /// Target and full precursor set match the step's substrate exactly
    /// (canonical SMILES; precursor order is irrelevant).
    ExactSubstrate,
    /// Same template, but a different substrate: a literature precedent for
    /// the reaction type, not evidence for this exact transformation.
    TemplateOnly,
}

/// Canonicalizes one SMILES string via RENKIN's standard parse+canonicalize
/// pipeline (`chem_env::mol_from_smiles`/`to_canonical`). Returns `None` if it
/// fails to parse. Shared by `match_example` and the batch template matcher
/// (`crate::evidence_match`) so authoring-time and route-display matching
/// never diverge from each other.
pub(crate) fn canonicalize(smiles: &str) -> Option<String> {
    mol_from_smiles(smiles).ok().map(|m| to_canonical(&m))
}

/// Canonicalizes every SMILES in `smiles`, then sorts and dedups the result
/// for order-independent set comparison. Returns `None` if any entry fails to
/// parse.
pub(crate) fn canonical_set(smiles: &[String]) -> Option<Vec<String>> {
    let mut out = smiles
        .iter()
        .map(|s| canonicalize(s))
        .collect::<Option<Vec<_>>>()?;
    out.sort();
    out.dedup();
    Some(out)
}

/// Classifies `example` against a step's `target`/`precursors` SMILES.
/// Comparison follows RENKIN's current canonical-SMILES behavior; no
/// separate stereo-ignoring normalization is applied. No partial-structure
/// similarity is attempted. A SMILES that fails to parse never matches
/// (conservatively yields `TemplateOnly`).
pub fn match_example(
    example: &ReactionExample,
    target: &str,
    precursors: &[String],
) -> ExampleMatch {
    let target_matches = matches!(
        (canonicalize(&example.target_smiles), canonicalize(target)),
        (Some(a), Some(b)) if a == b
    );
    let precursors_match = target_matches
        && matches!(
            (
                canonical_set(&example.precursor_smiles),
                canonical_set(precursors),
            ),
            (Some(a), Some(b)) if a == b
        );

    if precursors_match {
        ExampleMatch::ExactSubstrate
    } else {
        ExampleMatch::TemplateOnly
    }
}

/// A [`ReactionExample`] resolved against one route step's substrate,
/// carrying its [`ExampleMatch`] classification alongside the example
/// itself. This is what actually gets attached to a step -- see
/// `TemplateMetadataEntry::to_step_evidence` -- precisely so that JSON/
/// Python consumers can tell "evidence for this exact reaction" apart from
/// "literature precedent for a different substrate" without redoing the
/// canonical-SMILES comparison themselves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedReactionExample {
    pub match_kind: ExampleMatch,
    #[serde(flatten)]
    pub example: ReactionExample,
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
    /// Substrate-specific examples (schema_version 2+), resolved against
    /// this step's exact target/precursors: every exact-substrate match,
    /// plus up to 3 same-template-different-substrate precedents. Empty for
    /// schema_version 1 sidecars. See `template_examples_total` for how many
    /// were declared for this template in total.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<ResolvedReactionExample>,
    /// Total number of `examples` declared for this template in the
    /// sidecar, before the per-step exact/template-only resolution and cap
    /// above -- lets a consumer tell `"... and N more"` from `examples.len()`
    /// without needing the raw sidecar. 0 when `examples` is empty.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub template_examples_total: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl StepEvidence {
    fn is_empty(&self) -> bool {
        self.condition_candidates.is_empty()
            && self.reported_yields.is_empty()
            && self.references.is_empty()
            && self.warnings.is_empty()
            && self.examples.is_empty()
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
    /// Substrate-specific examples (schema_version 2+ only -- rejected by
    /// `validate_template_metadata` under schema_version 1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<ReactionExample>,
}

impl TemplateMetadataEntry {
    /// Builds the `StepEvidence` to attach to a matching `ReactionStep`,
    /// resolved against that step's exact `target`/`precursors` -- or `None`
    /// if this entry carries no actual data.
    ///
    /// Examples are resolved, not merely cloned: every `target`/`precursors`
    /// is compared via [`match_example`], every exact-substrate match is
    /// kept, and same-template-different-substrate precedents are capped at
    /// [`MAX_TEMPLATE_ONLY_EXAMPLES`] (exact matches always sort first) --
    /// this is what keeps a step's JSON output bounded even when a template
    /// carries hundreds of dataset-derived examples. When `examples` is
    /// non-empty, `references` is likewise trimmed to only the ids actually
    /// cited by what's kept (template-level condition/yield/warning entries
    /// plus the retained examples). schema_version 1 entries never have
    /// `examples` (rejected by `validate_template_metadata`), so this never
    /// touches them: their full `references` list -- including standalone
    /// citations not cited by anything else in the entry -- is preserved
    /// exactly as before `examples` existed.
    pub fn to_step_evidence(&self, target: &str, precursors: &[String]) -> Option<StepEvidence> {
        const MAX_TEMPLATE_ONLY_EXAMPLES: usize = 3;

        let mut exact = Vec::new();
        let mut template_only = Vec::new();
        for example in &self.examples {
            let match_kind = match_example(example, target, precursors);
            let resolved = ResolvedReactionExample {
                match_kind,
                example: example.clone(),
            };
            match match_kind {
                ExampleMatch::ExactSubstrate => exact.push(resolved),
                ExampleMatch::TemplateOnly => template_only.push(resolved),
            }
        }
        let template_examples_total = self.examples.len();
        template_only.truncate(MAX_TEMPLATE_ONLY_EXAMPLES);
        exact.extend(template_only);
        let examples = exact;

        fn note_refs<'a>(used: &mut HashSet<&'a str>, ids: &'a [String]) {
            used.extend(ids.iter().map(String::as_str));
        }

        let mut used_ref_ids: HashSet<&str> = HashSet::new();
        for c in &self.condition_candidates {
            note_refs(&mut used_ref_ids, &c.reference_ids);
        }
        for y in &self.reported_yields {
            note_refs(&mut used_ref_ids, &y.reference_ids);
        }
        for w in &self.warnings {
            note_refs(&mut used_ref_ids, &w.reference_ids);
        }
        for resolved in &examples {
            let ex = &resolved.example;
            note_refs(&mut used_ref_ids, &ex.reference_ids);
            if let Some(c) = &ex.conditions {
                note_refs(&mut used_ref_ids, &c.reference_ids);
            }
            if let Some(y) = &ex.reported_yield {
                note_refs(&mut used_ref_ids, &y.reference_ids);
            }
            for w in &ex.warnings {
                note_refs(&mut used_ref_ids, &w.reference_ids);
            }
        }
        let references = if self.examples.is_empty() {
            // No examples to bound the output against (schema_version 1 always
            // takes this path) -- preserve the full reference list, including
            // standalone citations not cited by any condition/yield/warning.
            self.references.clone()
        } else {
            self.references
                .iter()
                .filter(|r| used_ref_ids.contains(r.id.as_str()))
                .cloned()
                .collect()
        };

        let evidence = StepEvidence {
            condition_candidates: self.condition_candidates.clone(),
            reported_yields: self.reported_yields.clone(),
            references,
            warnings: self.warnings.clone(),
            examples,
            template_examples_total,
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

const SUPPORTED_SCHEMA_VERSIONS: &[u32] = &[1, 2];

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

/// Checks that every id in `reference_ids` is present in `seen_ref_ids`
/// (the enclosing template's own `references` list). `context` names the
/// field being checked, purely for the error message (e.g. `"reported_yield"`
/// or `"example \"ex-1\" conditions"`).
fn check_reference_ids(
    reference_ids: &[String],
    context: &str,
    template_id: &str,
    seen_ref_ids: &HashSet<&str>,
) -> Result<()> {
    for rid in reference_ids {
        if !seen_ref_ids.contains(rid.as_str()) {
            bail!("template {template_id:?}: {context} references unknown reference id {rid:?}");
        }
    }
    Ok(())
}

fn check_range(r: &FloatRange, template_id: &str, context: &str) -> Result<()> {
    if r.min > r.max {
        bail!(
            "template {template_id:?}: {context} range min {} > max {}",
            r.min,
            r.max
        );
    }
    Ok(())
}

fn check_scope(
    actual: EvidenceScope,
    required: Option<EvidenceScope>,
    template_id: &str,
    context: &str,
) -> Result<()> {
    if let Some(required) = required
        && actual != required
    {
        bail!("template {template_id:?}: {context} scope must be {required:?}, got {actual:?}");
    }
    Ok(())
}

fn check_condition_candidate(
    c: &ConditionCandidate,
    template_id: &str,
    seen_ref_ids: &HashSet<&str>,
    required_scope: Option<EvidenceScope>,
    context: &str,
) -> Result<()> {
    check_reference_ids(&c.reference_ids, context, template_id, seen_ref_ids)?;
    if let Some(r) = &c.temperature_c {
        check_range(r, template_id, &format!("{context} temperature_c"))?;
    }
    if let Some(r) = &c.time_hours {
        check_range(r, template_id, &format!("{context} time_hours"))?;
    }
    check_scope(c.scope, required_scope, template_id, context)
}

fn check_yield_percentage(p: &YieldPercentage, template_id: &str, context: &str) -> Result<()> {
    match p {
        YieldPercentage::Single(v) => {
            if !(0.0..=100.0).contains(v) {
                bail!("template {template_id:?}: {context} percentage {v} out of range [0, 100]");
            }
        }
        YieldPercentage::Range(r) => {
            if r.min > r.max {
                bail!(
                    "template {template_id:?}: {context} percentage range min {} > max {}",
                    r.min,
                    r.max
                );
            }
            if !(0.0..=100.0).contains(&r.min) || !(0.0..=100.0).contains(&r.max) {
                bail!(
                    "template {template_id:?}: {context} percentage range [{}, {}] out of range [0, 100]",
                    r.min,
                    r.max
                );
            }
        }
    }
    Ok(())
}

fn check_reported_yield(
    y: &ReportedYield,
    template_id: &str,
    seen_ref_ids: &HashSet<&str>,
    required_scope: Option<EvidenceScope>,
    context: &str,
) -> Result<()> {
    check_reference_ids(&y.reference_ids, context, template_id, seen_ref_ids)?;
    check_yield_percentage(&y.percentage, template_id, context)?;
    check_scope(y.scope, required_scope, template_id, context)
}

fn check_warning(
    w: &ReactionWarning,
    template_id: &str,
    seen_ref_ids: &HashSet<&str>,
    required_scope: Option<EvidenceScope>,
    context: &str,
) -> Result<()> {
    check_reference_ids(&w.reference_ids, context, template_id, seen_ref_ids)?;
    check_scope(w.scope, required_scope, template_id, context)
}

fn validate_template_metadata(file: &TemplateMetadataFile) -> Result<()> {
    if !SUPPORTED_SCHEMA_VERSIONS.contains(&file.schema_version) {
        bail!(
            "unsupported template metadata schema_version {} (supported: {SUPPORTED_SCHEMA_VERSIONS:?})",
            file.schema_version
        );
    }

    for (template_id, entry) in &file.templates {
        if file.schema_version == 1 && !entry.examples.is_empty() {
            bail!(
                "template {template_id:?}: schema_version 1 does not support `examples` (requires schema_version 2)"
            );
        }
        if file.schema_version == 2 && !entry.reported_yields.is_empty() {
            bail!(
                "template {template_id:?}: schema_version 2 requires reported yields under examples[].reported_yield (template-level `reported_yields` is not allowed under schema_version 2)"
            );
        }

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

        for c in &entry.condition_candidates {
            check_condition_candidate(c, template_id, &seen_ref_ids, None, "condition_candidate")?;
        }
        for y in &entry.reported_yields {
            check_reported_yield(y, template_id, &seen_ref_ids, None, "reported_yield")?;
        }
        for w in &entry.warnings {
            check_warning(w, template_id, &seen_ref_ids, None, "warning")?;
        }

        let mut seen_example_ids: HashSet<&str> = HashSet::new();
        for example in &entry.examples {
            if example.id.trim().is_empty() {
                bail!("template {template_id:?}: example id must not be empty");
            }
            if !seen_example_ids.insert(example.id.as_str()) {
                bail!(
                    "template {template_id:?}: duplicate example id {:?}",
                    example.id
                );
            }
            let ctx = format!("example {:?}", example.id);

            mol_from_smiles(&example.target_smiles).with_context(|| {
                format!(
                    "template {template_id:?}: {ctx}: target_smiles {:?} does not parse",
                    example.target_smiles
                )
            })?;
            if example.precursor_smiles.is_empty() {
                bail!("template {template_id:?}: {ctx}: precursor_smiles must not be empty");
            }
            for p in &example.precursor_smiles {
                mol_from_smiles(p).with_context(|| {
                    format!(
                        "template {template_id:?}: {ctx}: precursor_smiles {p:?} does not parse"
                    )
                })?;
            }

            check_reference_ids(&example.reference_ids, &ctx, template_id, &seen_ref_ids)?;
            if let Some(c) = &example.conditions {
                check_condition_candidate(
                    c,
                    template_id,
                    &seen_ref_ids,
                    Some(EvidenceScope::SubstrateSpecific),
                    &format!("{ctx} conditions"),
                )?;
            }
            if let Some(y) = &example.reported_yield {
                check_reported_yield(
                    y,
                    template_id,
                    &seen_ref_ids,
                    Some(EvidenceScope::SubstrateSpecific),
                    &format!("{ctx} reported_yield"),
                )?;
            }
            for w in &example.warnings {
                check_warning(
                    w,
                    template_id,
                    &seen_ref_ids,
                    Some(EvidenceScope::SubstrateSpecific),
                    &format!("{ctx} warning"),
                )?;
            }
            if let Some(drid) = &example.dataset_record_id
                && drid.trim().is_empty()
            {
                bail!(
                    "template {template_id:?}: {ctx}: dataset_record_id must not be empty when present"
                );
            }
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
        let evidence = entry.to_step_evidence("irrelevant", &[]).unwrap();
        assert_eq!(evidence.condition_candidates.len(), 1);
        assert_eq!(evidence.reported_yields.len(), 1);
        assert_eq!(evidence.warnings.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_entry_yields_no_step_evidence() {
        let entry = TemplateMetadataEntry::default();
        assert!(entry.to_step_evidence("irrelevant", &[]).is_none());
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

    #[test]
    fn v1_sidecar_examples_absent_and_omitted_from_json() {
        // A v1 sidecar with no `examples` key must behave exactly as it did
        // before this field existed: `entry.examples` empty, and the field
        // omitted from serialized StepEvidence (unchanged JSON shape).
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_v1_no_examples.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {
                        "warnings": [{
                            "code": "x", "severity": "low", "message": "m",
                            "source": "literature", "scope": "template",
                            "reference_ids": []
                        }]
                    }
                }
            }"#,
        );
        let file = load_template_metadata(&path).unwrap();
        let entry = file.templates.get("t1").unwrap();
        assert!(entry.examples.is_empty());
        let evidence = entry.to_step_evidence("irrelevant", &[]).unwrap();
        let json = serde_json::to_string(&evidence).unwrap();
        assert!(!json.contains("examples"), "got: {json}");
        assert!(!json.contains("template_examples_total"), "got: {json}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn schema_v1_reference_only_entry_is_preserved_in_step_evidence() {
        // A template entry whose only content is a standalone `references`
        // list (not cited by any condition/yield/warning) must still produce
        // Some(StepEvidence) carrying that reference -- reference trimming is
        // only ever applied when `examples` is in play, and schema_version 1
        // never has `examples`.
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_v1_reference_only.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "rule:suzuki_retro": {
                        "references": [
                            {"id": "review", "kind": "doi", "identifier": "10.xxxx/review"}
                        ]
                    }
                }
            }"#,
        );
        let file = load_template_metadata(&path).unwrap();
        let entry = file.templates.get("rule:suzuki_retro").unwrap();
        let evidence = entry.to_step_evidence("irrelevant", &[]).unwrap();
        let json = serde_json::to_string(&evidence).unwrap();
        assert!(json.contains("10.xxxx/review"), "got: {json}");
        assert!(!json.contains("examples"), "got: {json}");
        assert!(!json.contains("template_examples_total"), "got: {json}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn schema_v2_example_loads_and_matches_expected_shape() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_v2_example.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "smirks-sha256:abc": {
                        "references": [{"id": "ref-1", "kind": "doi", "identifier": "10.xxxx/example"}],
                        "examples": [{
                            "id": "ex-1",
                            "target_smiles": "c1ccc(-c2ccccc2)cc1",
                            "precursor_smiles": ["Brc1ccccc1", "c1ccccc1"],
                            "conditions": {
                                "catalysts": ["Pd(PPh3)4"],
                                "solvents": ["EtOH"],
                                "source": "literature",
                                "scope": "substrate_specific",
                                "reference_ids": ["ref-1"]
                            },
                            "reported_yield": {
                                "percentage": 78.0,
                                "basis": "isolated",
                                "source": "literature",
                                "scope": "substrate_specific",
                                "reference_ids": ["ref-1"]
                            },
                            "reference_ids": ["ref-1"]
                        }]
                    }
                }
            }"#,
        );
        let file = load_template_metadata(&path).unwrap();
        assert_eq!(file.schema_version, 2);
        let entry = file.templates.get("smirks-sha256:abc").unwrap();
        assert_eq!(entry.examples.len(), 1);
        let evidence = entry
            .to_step_evidence(
                "c1ccc(-c2ccccc2)cc1",
                &["Brc1ccccc1".to_string(), "c1ccccc1".to_string()],
            )
            .unwrap();
        assert_eq!(evidence.examples.len(), 1);
        assert_eq!(evidence.examples[0].example.id, "ex-1");
        assert_eq!(
            evidence.examples[0].match_kind,
            ExampleMatch::ExactSubstrate
        );
        assert_eq!(evidence.template_examples_total, 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn template_level_reported_yields_rejected_under_schema_v2() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_v2_template_level_yield.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "t1": {
                        "reported_yields": [{
                            "percentage": 78.0,
                            "basis": "isolated",
                            "source": "literature",
                            "scope": "template"
                        }]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("requires reported yields under examples[].reported_yield"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn template_level_reported_yields_still_allowed_under_schema_v1() {
        // Backward compatibility: schema_version 1 sidecars never had `examples`,
        // so template-level reported_yields must keep working exactly as before.
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_v1_template_level_yield.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {
                        "reported_yields": [{
                            "percentage": 78.0,
                            "basis": "isolated",
                            "source": "literature",
                            "scope": "template"
                        }]
                    }
                }
            }"#,
        );
        assert!(load_template_metadata(&path).is_ok());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn to_step_evidence_keeps_all_exact_and_caps_template_only_at_three() {
        let target = "c1ccc(-c2ccccc2)cc1";
        let precursors = vec!["Brc1ccccc1".to_string(), "c1ccccc1".to_string()];

        let mut entry = TemplateMetadataEntry::default();
        for i in 0..2 {
            let mut ex = sample_example(target, &["Brc1ccccc1", "c1ccccc1"]);
            ex.id = format!("exact-{i}");
            entry.examples.push(ex);
        }
        for i in 0..5 {
            let mut ex = sample_example("CCO", &["CCO"]);
            ex.id = format!("template-only-{i}");
            entry.examples.push(ex);
        }

        let evidence = entry.to_step_evidence(target, &precursors).unwrap();
        // All exact matches kept, template-only capped at 3 -- 2 + 3 = 5 shown
        // out of 7 declared.
        assert_eq!(evidence.examples.len(), 5);
        assert_eq!(evidence.template_examples_total, 7);
        assert!(
            evidence.examples[..2]
                .iter()
                .all(|r| r.match_kind == ExampleMatch::ExactSubstrate),
            "exact matches must sort first"
        );
        assert!(
            evidence.examples[2..]
                .iter()
                .all(|r| r.match_kind == ExampleMatch::TemplateOnly)
        );
    }

    #[test]
    fn to_step_evidence_trims_references_to_only_used_ids() {
        let target = "c1ccc(-c2ccccc2)cc1";
        let precursors = vec!["Brc1ccccc1".to_string(), "c1ccccc1".to_string()];

        let mut example = sample_example(target, &["Brc1ccccc1", "c1ccccc1"]);
        example.reference_ids = vec!["used-ref".to_string()];

        let entry = TemplateMetadataEntry {
            references: vec![
                EvidenceReference {
                    id: "used-ref".to_string(),
                    kind: ReferenceKind::Doi,
                    identifier: "10.aaa/used".to_string(),
                    title: None,
                },
                EvidenceReference {
                    id: "unused-ref".to_string(),
                    kind: ReferenceKind::Doi,
                    identifier: "10.bbb/unused".to_string(),
                    title: None,
                },
            ],
            examples: vec![example],
            ..Default::default()
        };

        let evidence = entry.to_step_evidence(target, &precursors).unwrap();
        let ref_ids: Vec<&str> = evidence.references.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ref_ids, vec!["used-ref"]);
    }

    #[test]
    fn examples_under_schema_v1_are_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_v1_with_examples.json",
            r#"{
                "schema_version": 1,
                "templates": {
                    "t1": {
                        "examples": [{
                            "id": "ex-1",
                            "target_smiles": "c1ccccc1",
                            "precursor_smiles": ["c1ccccc1"]
                        }]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("schema_version 1 does not support"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn duplicate_example_id_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_dup_example_id.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "t1": {
                        "examples": [
                            {"id": "ex-1", "target_smiles": "c1ccccc1", "precursor_smiles": ["c1ccccc1"]},
                            {"id": "ex-1", "target_smiles": "CCO", "precursor_smiles": ["CCO"]}
                        ]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("duplicate example id"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_example_id_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_empty_example_id.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "t1": {
                        "examples": [{"id": "   ", "target_smiles": "c1ccccc1", "precursor_smiles": ["c1ccccc1"]}]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("example id must not be empty"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn malformed_target_smiles_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_bad_target_smiles.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "t1": {
                        "examples": [{"id": "ex-1", "target_smiles": "not(a smiles", "precursor_smiles": ["c1ccccc1"]}]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("target_smiles") && msg.contains("does not parse"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn malformed_precursor_smiles_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_bad_precursor_smiles.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "t1": {
                        "examples": [{"id": "ex-1", "target_smiles": "c1ccccc1", "precursor_smiles": ["not(a smiles"]}]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("precursor_smiles") && msg.contains("does not parse"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_precursor_list_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_empty_precursors.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "t1": {
                        "examples": [{"id": "ex-1", "target_smiles": "c1ccccc1", "precursor_smiles": []}]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("precursor_smiles must not be empty"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn dangling_reference_id_in_example_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_example_dangling_ref.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "t1": {
                        "examples": [{
                            "id": "ex-1",
                            "target_smiles": "c1ccccc1",
                            "precursor_smiles": ["c1ccccc1"],
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
    fn non_substrate_specific_scope_in_example_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_example_bad_scope.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "t1": {
                        "examples": [{
                            "id": "ex-1",
                            "target_smiles": "c1ccccc1",
                            "precursor_smiles": ["c1ccccc1"],
                            "conditions": {"source": "literature", "scope": "template"}
                        }]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("scope must be SubstrateSpecific"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_dataset_record_id_is_rejected() {
        let dir = std::env::temp_dir();
        let path = write_sidecar(
            &dir,
            "renkin_evidence_empty_dataset_record_id.json",
            r#"{
                "schema_version": 2,
                "templates": {
                    "t1": {
                        "examples": [{
                            "id": "ex-1",
                            "target_smiles": "c1ccccc1",
                            "precursor_smiles": ["c1ccccc1"],
                            "dataset_record_id": "  "
                        }]
                    }
                }
            }"#,
        );
        let err = load_template_metadata(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("dataset_record_id must not be empty"),
            "got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    fn sample_example(target: &str, precursors: &[&str]) -> ReactionExample {
        ReactionExample {
            id: "ex-1".to_string(),
            target_smiles: target.to_string(),
            precursor_smiles: precursors.iter().map(|s| s.to_string()).collect(),
            conditions: None,
            reported_yield: None,
            warnings: vec![],
            reference_ids: vec![],
            dataset_record_id: None,
            notes: None,
        }
    }

    #[test]
    fn exact_substrate_match_ignores_precursor_order() {
        let example = sample_example("c1ccc(-c2ccccc2)cc1", &["Brc1ccccc1", "c1ccccc1"]);
        let step_precursors = vec!["c1ccccc1".to_string(), "Brc1ccccc1".to_string()];
        assert_eq!(
            match_example(&example, "c1ccc(-c2ccccc2)cc1", &step_precursors),
            ExampleMatch::ExactSubstrate
        );
    }

    #[test]
    fn different_target_yields_template_only() {
        let example = sample_example("c1ccc(-c2ccccc2)cc1", &["Brc1ccccc1", "c1ccccc1"]);
        let step_precursors = vec!["Brc1ccccc1".to_string(), "c1ccccc1".to_string()];
        assert_eq!(
            match_example(&example, "CCO", &step_precursors),
            ExampleMatch::TemplateOnly
        );
    }

    #[test]
    fn different_precursors_yield_template_only() {
        let example = sample_example("c1ccc(-c2ccccc2)cc1", &["Brc1ccccc1", "c1ccccc1"]);
        let step_precursors = vec!["Clc1ccccc1".to_string(), "c1ccccc1".to_string()];
        assert_eq!(
            match_example(&example, "c1ccc(-c2ccccc2)cc1", &step_precursors),
            ExampleMatch::TemplateOnly
        );
    }
}
