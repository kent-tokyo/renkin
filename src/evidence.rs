use serde::Serialize;

/// How a step's metadata (today: `conditions`/`reaction_family`; future phases:
/// yield, references, warnings) was determined. Distinct from `step_confidence`/
/// `success_probability`, which are template-frequency-derived search-ranking
/// scores, not experimental measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataSource {
    /// Rule-author-supplied placeholder conditions (`conditions_for_rule` et al.) --
    /// a plausible default, not a measured or literature-sourced result. The only
    /// variant currently constructed.
    HandcraftedDefault,
    /// Reserved for a later phase: derived from a structured reaction dataset.
    DatasetRecord,
    /// Reserved for a later phase: sourced from a cited paper or patent.
    Literature,
    /// Reserved for a later phase: output of a trained yield/condition-prediction model.
    ModelPrediction,
}

/// What scope a piece of step metadata was assigned at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceScope {
    /// Assigned to the reaction family as a whole (e.g. all Suzuki couplings) -- how
    /// hand-crafted-rule tags are scoped today. The only variant currently constructed.
    ReactionFamily,
    /// Reserved: assigned to one specific extracted SMIRKS template.
    Template,
    /// Reserved: assigned to this exact target/precursor substrate. No code path
    /// before per-substrate literature lookup exists can produce this.
    SubstrateSpecific,
}
