//! Versioned, evidence-carrying route interchange output.
//!
//! This schema is an export contract for the normalized Bridge audit. It
//! preserves facts RENKIN actually observed and marks source metadata that
//! current adapters do not retain as `null`; it never invents source versions
//! or original node identifiers.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bridge::audit::{
    AuditFinding, AuditPolicy, AuditReport, AuditStatus, CheckStatus, audit_document_with_policy,
};
use crate::bridge::forward::EvidenceBasis;
use crate::bridge::private_stock::PrivateStockReport;
use crate::bridge::route_graph::{ReactionEvidence, RouteDocument, RouteNode, RouteSource};
use crate::chem_env::{RetroRule, mol_from_smiles, to_canonical};

pub const ROUTE_INTERCHANGE_SCHEMA_VERSION: u32 = 1;
/// Explicit-tree interchange.  Version 1 remains supported for historical
/// documents, but cannot faithfully carry direct-purchase routes or repeated
/// molecule occurrences.
pub const ROUTE_INTERCHANGE_V2_SCHEMA_VERSION: u32 = 2;
pub const ADAPTER_LOSS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LossDisposition {
    Preserved,
    Normalized,
    Inferred,
    Dropped,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterLossField {
    pub field: String,
    pub disposition: LossDisposition,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterLossReport {
    pub schema_version: u32,
    pub fields: Vec<AdapterLossField>,
}

impl AdapterLossReport {
    fn for_audit(report: &AuditReport) -> Self {
        let mut fields = vec![
            AdapterLossField {
                field: "target".into(),
                disposition: LossDisposition::Normalized,
                reason: "target is emitted in RENKIN canonical form".into(),
            },
            AdapterLossField {
                field: "precursors".into(),
                disposition: LossDisposition::Normalized,
                reason: "precursor identities are emitted in RENKIN canonical form".into(),
            },
            AdapterLossField {
                field: "canonical_node_id".into(),
                disposition: LossDisposition::Inferred,
                reason: "derived from normalized route hash and step index".into(),
            },
            AdapterLossField {
                field: "conditions".into(),
                disposition: LossDisposition::Unsupported,
                reason: "condition records are not part of the current canonical schema".into(),
            },
        ];
        if report
            .steps
            .iter()
            .all(|step| step.reaction_evidence.is_some())
        {
            fields.push(AdapterLossField {
                field: "reaction_provenance".into(),
                disposition: LossDisposition::Preserved,
                reason: "source reaction evidence is retained when supplied".into(),
            });
        } else {
            fields.push(AdapterLossField {
                field: "reaction_provenance".into(),
                disposition: LossDisposition::Unsupported,
                reason: "one or more source steps supplied no reaction evidence".into(),
            });
        }
        Self {
            schema_version: ADAPTER_LOSS_SCHEMA_VERSION,
            fields,
        }
    }

    pub fn validate_for_strict_import(&self) -> anyhow::Result<()> {
        if self.schema_version != ADAPTER_LOSS_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported adapter loss schema_version {}",
                self.schema_version
            );
        }
        if self.fields.is_empty() {
            anyhow::bail!("adapter loss report must contain at least one field record");
        }
        if self
            .fields
            .iter()
            .any(|field| field.field.trim().is_empty())
        {
            anyhow::bail!("adapter loss report contains an empty field name");
        }
        Ok(())
    }
}

/// Validate a canonical interchange document before an audit or replay uses
/// it. This validates the envelope and loss accounting only; chemistry remains
/// the responsibility of the normal audit pipeline.
pub fn validate_strict_import(value: &Value) -> anyhow::Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("canonical interchange must be a JSON object"))?;
    const ENVELOPE_FIELDS: &[&str] = &[
        "schema_version",
        "source_tool",
        "source_version",
        "source_route_id",
        "route_id",
        "audit_status",
        "steps",
        "audit_findings",
        "loss_report",
        "stock_provenance",
    ];
    if let Some(field) = object
        .keys()
        .find(|field| !ENVELOPE_FIELDS.contains(&field.as_str()))
    {
        anyhow::bail!("unknown canonical interchange v1 field {field:?}");
    }
    if object.get("schema_version").and_then(Value::as_u64)
        != Some(ROUTE_INTERCHANGE_SCHEMA_VERSION as u64)
    {
        anyhow::bail!("unsupported canonical interchange schema_version");
    }
    if object
        .get("route_id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        anyhow::bail!("canonical interchange route_id is required");
    }
    let Some(steps) = object.get("steps").and_then(Value::as_array) else {
        anyhow::bail!("canonical interchange steps must be an array");
    };
    for step in steps {
        let step = step
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("canonical interchange step must be an object"))?;
        const STEP_FIELDS: &[&str] = &[
            "canonical_node_id",
            "original_node_id",
            "target",
            "precursors",
            "reaction_provenance",
        ];
        if let Some(field) = step
            .keys()
            .find(|field| !STEP_FIELDS.contains(&field.as_str()))
        {
            anyhow::bail!("unknown canonical interchange v1 step field {field:?}");
        }
    }
    let loss_report = object
        .get("loss_report")
        .ok_or_else(|| anyhow::anyhow!("canonical interchange loss_report is required"))?;
    let loss_object = loss_report
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("canonical interchange loss_report must be an object"))?;
    if loss_object.get("schema_version").and_then(Value::as_u64)
        != Some(ADAPTER_LOSS_SCHEMA_VERSION as u64)
    {
        anyhow::bail!("unsupported adapter loss schema_version");
    }
    let fields = loss_object
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("adapter loss report fields must be an array"))?;
    if fields.is_empty() {
        anyhow::bail!("adapter loss report must contain at least one field record");
    }
    for field in fields {
        let field = field
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("adapter loss field must be an object"))?;
        if field
            .get("field")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            || field
                .get("reason")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            anyhow::bail!("adapter loss field and reason are required");
        }
        match field.get("disposition").and_then(Value::as_str) {
            Some("preserved" | "normalized" | "inferred" | "dropped" | "unsupported") => {}
            _ => anyhow::bail!("unknown adapter loss disposition"),
        }
    }
    Ok(())
}

/// Result of a strict canonical re-import followed by RENKIN's ordinary
/// structure, stock, element, and forward audit. `route_id` is verified
/// against the reconstructed normalized tree; it is never trusted from the
/// imported document.
#[derive(Debug, Clone, Serialize)]
pub struct InterchangeReaudit {
    pub source_tool: String,
    pub imported_route_id: String,
    pub recomputed_route_id: String,
    pub audit: AuditReport,
}

#[derive(Debug, Deserialize)]
struct ImportEnvelope {
    schema_version: u32,
    source_tool: String,
    route_id: String,
    steps: Vec<ImportStep>,
}

#[derive(Debug, Deserialize)]
struct ImportStep {
    canonical_node_id: String,
    target: String,
    precursors: Vec<String>,
    reaction_provenance: ImportReactionProvenance,
}

#[derive(Debug, Deserialize)]
struct ImportReactionProvenance {
    #[serde(default)]
    reaction_evidence: Option<ReactionEvidence>,
}

/// Strictly re-import an interchange v1 document and run the normal audit.
///
/// Version 1 stores a flattened list of steps rather than an explicit graph.
/// It can therefore be reconstructed only when every decomposed molecule has
/// one step, there is one root, and the resulting graph is acyclic. Ambiguous
/// routes are rejected instead of being guessed. A configured stock set is
/// required because v1 does not preserve leaf-stock assertions; the imported
/// route hash is then recomputed from the resulting current-stock tree and
/// must match exactly.
pub fn reauditable_import_v1(
    value: &Value,
    configured_stock: &HashSet<String>,
    rules: &[RetroRule],
    policy: AuditPolicy,
) -> anyhow::Result<InterchangeReaudit> {
    validate_strict_import(value)?;
    let envelope: ImportEnvelope = serde_json::from_value(value.clone())
        .map_err(|error| anyhow::anyhow!("canonical interchange v1 decode failed: {error}"))?;
    if envelope.schema_version != ROUTE_INTERCHANGE_SCHEMA_VERSION {
        anyhow::bail!("unsupported canonical interchange schema_version");
    }
    let source = import_source(&envelope.source_tool)?;
    let document = document_from_v1_steps(&envelope.steps, source, configured_stock)?;
    let audit = audit_document_with_policy(&document, Some(configured_stock), Some(rules), policy);
    let recomputed_route_id = audit
        .normalized_route_sha256
        .clone()
        .ok_or_else(|| anyhow::anyhow!("re-imported route did not yield a normalized route ID"))?;
    if envelope.route_id != recomputed_route_id {
        anyhow::bail!(
            "canonical interchange route_id mismatch: imported {} but reconstructed {}",
            envelope.route_id,
            recomputed_route_id
        );
    }
    Ok(InterchangeReaudit {
        source_tool: envelope.source_tool,
        imported_route_id: envelope.route_id,
        recomputed_route_id,
        audit,
    })
}

fn import_source(source_tool: &str) -> anyhow::Result<RouteSource> {
    match source_tool {
        "renkin" => Ok(RouteSource::Renkin),
        "aizynthfinder" => Ok(RouteSource::AiZynthFinder),
        "syntheseus" => Ok(RouteSource::Syntheseus),
        "synplanner" => Ok(RouteSource::SynPlanner),
        _ => anyhow::bail!("unsupported canonical interchange source_tool {source_tool:?}"),
    }
}

fn canonical_import_smiles(smiles: &str, field: &str) -> anyhow::Result<String> {
    let canonical = to_canonical(&mol_from_smiles(smiles).map_err(|error| {
        anyhow::anyhow!("canonical interchange {field} is not valid SMILES: {error}")
    })?);
    if canonical != smiles {
        anyhow::bail!("canonical interchange {field} is not RENKIN canonical SMILES: {smiles:?}");
    }
    Ok(canonical)
}

fn document_from_v1_steps(
    steps: &[ImportStep],
    source: RouteSource,
    configured_stock: &HashSet<String>,
) -> anyhow::Result<RouteDocument> {
    if steps.is_empty() {
        anyhow::bail!(
            "canonical interchange v1 cannot re-import a route with no steps: root identity is absent"
        );
    }

    let mut step_ids = HashSet::with_capacity(steps.len());
    let mut by_target = HashMap::with_capacity(steps.len());
    let mut all_precursors = HashSet::new();
    for step in steps {
        if step.canonical_node_id.trim().is_empty() || !step_ids.insert(&step.canonical_node_id) {
            anyhow::bail!("canonical interchange contains an empty or duplicate canonical_node_id");
        }
        let target = canonical_import_smiles(&step.target, "step target")?;
        let precursors = step
            .precursors
            .iter()
            .map(|precursor| canonical_import_smiles(precursor, "step precursor"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        if precursors.is_empty() {
            anyhow::bail!(
                "canonical interchange decomposition step must contain at least one precursor"
            );
        }
        if precursors.iter().any(|precursor| precursor == &target) {
            anyhow::bail!("canonical interchange contains a self-referential decomposition step");
        }
        all_precursors.extend(precursors.iter().cloned());
        if by_target
            .insert(
                target.clone(),
                (
                    precursors,
                    step.reaction_provenance.reaction_evidence.clone(),
                ),
            )
            .is_some()
        {
            anyhow::bail!(
                "canonical interchange v1 has multiple decomposition steps for {target:?}; occurrence topology is ambiguous"
            );
        }
    }

    let roots = by_target
        .keys()
        .filter(|target| !all_precursors.contains(*target))
        .cloned()
        .collect::<Vec<_>>();
    let [root] = roots.as_slice() else {
        anyhow::bail!(
            "canonical interchange v1 must have exactly one root; found {} (cycle or disconnected steps)",
            roots.len()
        );
    };
    let mut on_stack = HashSet::new();
    let root = build_v1_node(root, &by_target, configured_stock, &mut on_stack)?;
    Ok(RouteDocument {
        source,
        step_count_collapsed_edges: crate::bridge::route_graph::count_edges(&root),
        root,
    })
}

fn build_v1_node(
    smiles: &str,
    by_target: &HashMap<String, (Vec<String>, Option<ReactionEvidence>)>,
    configured_stock: &HashSet<String>,
    on_stack: &mut HashSet<String>,
) -> anyhow::Result<RouteNode> {
    let Some((precursors, evidence)) = by_target.get(smiles) else {
        return Ok(RouteNode {
            canonical_smiles: smiles.to_owned(),
            is_stock_leaf: Some(configured_stock.contains(smiles)),
            reaction_evidence: None,
            children: Vec::new(),
        });
    };
    if !on_stack.insert(smiles.to_owned()) {
        anyhow::bail!("canonical interchange v1 contains a route cycle at {smiles:?}");
    }
    let children = precursors
        .iter()
        .map(|precursor| build_v1_node(precursor, by_target, configured_stock, on_stack))
        .collect::<anyhow::Result<Vec<_>>>()?;
    on_stack.remove(smiles);
    Ok(RouteNode {
        canonical_smiles: smiles.to_owned(),
        is_stock_leaf: Some(false),
        reaction_evidence: evidence.clone(),
        children,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteInterchange {
    pub schema_version: u32,
    pub source_tool: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_route_id: Option<String>,
    pub route_id: String,
    pub audit_status: AuditStatus,
    pub steps: Vec<InterchangeStep>,
    pub audit_findings: Vec<AuditFinding>,
    pub loss_report: AdapterLossReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stock_provenance: Option<StockProvenance>,
}

/// Version 2 of the canonical route interchange.  Unlike v1's flat `steps`,
/// each occurrence has an explicit place in the tree, so a molecule used
/// twice is not silently collapsed and a direct-purchase root is representable.
#[derive(Debug, Clone, Serialize)]
pub struct RouteInterchangeV2 {
    pub schema_version: u32,
    pub source_tool: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_route_id: Option<String>,
    pub route_id: String,
    pub audit_status: AuditStatus,
    pub root: InterchangeNodeV2,
    pub audit_findings: Vec<AuditFinding>,
    pub loss_report: AdapterLossReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stock_provenance: Option<StockProvenance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterchangeNodeV2 {
    /// Deterministic occurrence ID, derived from the normalized route ID and
    /// the child-index path (`root`, `0`, `0.1`, ...).
    pub canonical_node_id: String,
    pub canonical_smiles: String,
    pub is_stock_leaf: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_evidence: Option<ReactionEvidence>,
    pub children: Vec<InterchangeNodeV2>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterchangeStep {
    pub canonical_node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_node_id: Option<String>,
    pub target: String,
    pub precursors: Vec<String>,
    pub reaction_provenance: ReactionProvenance,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReactionProvenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_evidence: Option<ReactionEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_basis: Option<EvidenceBasis>,
    pub forward_replay_status: CheckStatus,
    /// Whether this export retained the source tool's reaction representation.
    /// `false` is explicit when the adapter supplied no reaction evidence.
    pub source_representation_retained: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StockProvenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_stock_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_stock_policy_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_stock: Option<PrivateStockReport>,
}

pub fn from_audit_report(
    source_tool: &'static str,
    source_version: Option<String>,
    source_route_id: Option<String>,
    original_node_ids: &[Option<String>],
    report: &AuditReport,
    stock: Option<StockProvenance>,
) -> RouteInterchange {
    let route_id = report
        .normalized_route_sha256
        .clone()
        .unwrap_or_else(|| "unavailable".to_string());
    let steps = report
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| InterchangeStep {
            canonical_node_id: format!("{route_id}:step:{index}"),
            original_node_id: original_node_ids.get(index).cloned().flatten(),
            target: step.target.clone(),
            precursors: step.precursors.clone(),
            reaction_provenance: ReactionProvenance {
                source_representation_retained: step.reaction_evidence.is_some(),
                reaction_evidence: step.reaction_evidence.clone(),
                evidence_basis: step.forward_validation.evidence_basis,
                forward_replay_status: step.forward_validation.status,
            },
        })
        .collect();
    RouteInterchange {
        schema_version: ROUTE_INTERCHANGE_SCHEMA_VERSION,
        source_tool,
        source_version,
        source_route_id,
        route_id,
        audit_status: report.status,
        steps,
        audit_findings: report.findings.clone(),
        loss_report: AdapterLossReport::for_audit(report),
        stock_provenance: stock,
    }
}

/// Export an explicit-tree v2 interchange.  The caller supplies the audited
/// normalized document so topology is taken from the actual tree rather than
/// reconstructed from reporting steps.
pub fn from_document_v2(
    source_tool: &'static str,
    source_version: Option<String>,
    source_route_id: Option<String>,
    document: &RouteDocument,
    report: &AuditReport,
    stock: Option<StockProvenance>,
) -> RouteInterchangeV2 {
    let route_id = report
        .normalized_route_sha256
        .clone()
        .unwrap_or_else(|| "unavailable".to_string());
    RouteInterchangeV2 {
        schema_version: ROUTE_INTERCHANGE_V2_SCHEMA_VERSION,
        source_tool,
        source_version,
        source_route_id,
        root: node_to_v2(&document.root, &route_id, "root"),
        route_id,
        audit_status: report.status,
        audit_findings: report.findings.clone(),
        loss_report: AdapterLossReport::for_audit(report),
        stock_provenance: stock,
    }
}

fn node_to_v2(node: &RouteNode, route_id: &str, path: &str) -> InterchangeNodeV2 {
    InterchangeNodeV2 {
        canonical_node_id: format!("{route_id}:node:{path}"),
        canonical_smiles: node.canonical_smiles.clone(),
        is_stock_leaf: node.is_stock_leaf,
        reaction_evidence: node.reaction_evidence.clone(),
        children: node
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let child_path = if path == "root" {
                    index.to_string()
                } else {
                    format!("{path}.{index}")
                };
                node_to_v2(child, route_id, &child_path)
            })
            .collect(),
    }
}

#[derive(Debug, Deserialize)]
struct ImportEnvelopeV2 {
    schema_version: u32,
    source_tool: String,
    route_id: String,
    root: ImportNodeV2,
}

#[derive(Debug, Deserialize)]
struct ImportNodeV2 {
    canonical_node_id: String,
    canonical_smiles: String,
    is_stock_leaf: Option<bool>,
    #[serde(default)]
    reaction_evidence: Option<ReactionEvidence>,
    children: Vec<ImportNodeV2>,
}

/// Strictly re-import an explicit-tree v2 interchange and re-run RENKIN's
/// ordinary audit.  Node identifiers are occurrence paths, so repeated
/// precursors remain distinct.  The imported route hash must still equal the
/// hash recomputed from the reconstructed tree.
pub fn reauditable_import_v2(
    value: &Value,
    configured_stock: &HashSet<String>,
    rules: &[RetroRule],
    policy: AuditPolicy,
) -> anyhow::Result<InterchangeReaudit> {
    validate_strict_import_v2(value)?;
    let envelope: ImportEnvelopeV2 = serde_json::from_value(value.clone())
        .map_err(|error| anyhow::anyhow!("canonical interchange v2 decode failed: {error}"))?;
    if envelope.schema_version != ROUTE_INTERCHANGE_V2_SCHEMA_VERSION {
        anyhow::bail!("unsupported canonical interchange v2 schema_version");
    }
    let source = import_source(&envelope.source_tool)?;
    let root = document_node_from_v2(&envelope.root, &envelope.route_id, "root", 0)?;
    let document = RouteDocument {
        source,
        step_count_collapsed_edges: count_v2_steps(&root),
        root,
    };
    let audit = audit_document_with_policy(&document, Some(configured_stock), Some(rules), policy);
    let recomputed_route_id = audit
        .normalized_route_sha256
        .clone()
        .ok_or_else(|| anyhow::anyhow!("re-imported route did not yield a normalized route ID"))?;
    if envelope.route_id != recomputed_route_id {
        anyhow::bail!(
            "canonical interchange route_id mismatch: imported {} but reconstructed {}",
            envelope.route_id,
            recomputed_route_id
        );
    }
    Ok(InterchangeReaudit {
        source_tool: envelope.source_tool,
        imported_route_id: envelope.route_id,
        recomputed_route_id,
        audit,
    })
}

/// Strict envelope validation for the v2 tree contract.  It is separate from
/// v1 deliberately: adding `root` to v1 would make old consumers guess at
/// schema meaning.
pub fn validate_strict_import_v2(value: &Value) -> anyhow::Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("canonical interchange must be a JSON object"))?;
    const ENVELOPE_FIELDS: &[&str] = &[
        "schema_version",
        "source_tool",
        "source_version",
        "source_route_id",
        "route_id",
        "audit_status",
        "root",
        "audit_findings",
        "loss_report",
        "stock_provenance",
    ];
    if let Some(field) = object
        .keys()
        .find(|field| !ENVELOPE_FIELDS.contains(&field.as_str()))
    {
        anyhow::bail!("unknown canonical interchange v2 field {field:?}");
    }
    if object.get("schema_version").and_then(Value::as_u64)
        != Some(ROUTE_INTERCHANGE_V2_SCHEMA_VERSION as u64)
    {
        anyhow::bail!("unsupported canonical interchange v2 schema_version");
    }
    if object
        .get("route_id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        anyhow::bail!("canonical interchange route_id is required");
    }
    validate_v2_node(
        object
            .get("root")
            .ok_or_else(|| anyhow::anyhow!("canonical interchange root is required"))?,
        0,
    )?;
    let loss: AdapterLossReport = serde_json::from_value(
        object
            .get("loss_report")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("canonical interchange loss_report is required"))?,
    )
    .map_err(|error| {
        anyhow::anyhow!("canonical interchange v2 loss report decode failed: {error}")
    })?;
    loss.validate_for_strict_import()
}

fn validate_v2_node(value: &Value, depth: usize) -> anyhow::Result<()> {
    const MAX_INTERCHANGE_DEPTH: usize = 256;
    if depth > MAX_INTERCHANGE_DEPTH {
        anyhow::bail!("canonical interchange v2 exceeds maximum tree depth");
    }
    let node = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("canonical interchange v2 node must be an object"))?;
    const NODE_FIELDS: &[&str] = &[
        "canonical_node_id",
        "canonical_smiles",
        "is_stock_leaf",
        "reaction_evidence",
        "children",
    ];
    if let Some(field) = node
        .keys()
        .find(|field| !NODE_FIELDS.contains(&field.as_str()))
    {
        anyhow::bail!("unknown canonical interchange v2 node field {field:?}");
    }
    for required in ["canonical_node_id", "canonical_smiles", "children"] {
        if node.get(required).is_none() {
            anyhow::bail!("canonical interchange v2 node {required} is required");
        }
    }
    let children = node
        .get("children")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow::anyhow!("canonical interchange v2 node children must be an array")
        })?;
    for child in children {
        validate_v2_node(child, depth + 1)?;
    }
    Ok(())
}

fn document_node_from_v2(
    node: &ImportNodeV2,
    route_id: &str,
    path: &str,
    depth: usize,
) -> anyhow::Result<RouteNode> {
    if depth > 256 {
        anyhow::bail!("canonical interchange v2 exceeds maximum tree depth");
    }
    let expected_id = format!("{route_id}:node:{path}");
    if node.canonical_node_id != expected_id {
        anyhow::bail!("canonical interchange v2 canonical_node_id does not match occurrence path");
    }
    let canonical_smiles =
        canonical_import_smiles(&node.canonical_smiles, "node canonical_smiles")?;
    let children = node
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let child_path = if path == "root" {
                index.to_string()
            } else {
                format!("{path}.{index}")
            };
            document_node_from_v2(child, route_id, &child_path, depth + 1)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(RouteNode {
        canonical_smiles,
        is_stock_leaf: node.is_stock_leaf,
        reaction_evidence: node.reaction_evidence.clone(),
        children,
    })
}

fn count_v2_steps(node: &RouteNode) -> usize {
    usize::from(!node.children.is_empty()) + node.children.iter().map(count_v2_steps).sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::audit::{AuditReport, AuditedStep};
    use crate::bridge::route_graph::RouteSource;

    #[test]
    fn export_does_not_invent_source_identity() {
        let report = AuditReport {
            source: RouteSource::AiZynthFinder,
            status: AuditStatus::Partial,
            route_tree_parseable: true,
            reaction_steps_parseable: Some(true),
            stock_validation: None,
            target_element_accounting_status: None,
            normalized_route_sha256: Some("sha256:abc".into()),
            steps: vec![AuditedStep {
                target: "CCO".into(),
                precursors: vec!["C".into(), "CO".into()],
                reaction_evidence: Some(ReactionEvidence::SyntheseusReaction {
                    reaction_smiles: "C.CO>>CCO".into(),
                }),
                forward_validation: crate::bridge::forward::ForwardValidationResult {
                    status: CheckStatus::NotEvaluable,
                    method: "declared_reaction_replay",
                    evidence_basis: None,
                    reason: None,
                },
            }],
            findings: vec![],
        };
        let interchange = from_audit_report("aizynthfinder", None, None, &[], &report, None);
        assert_eq!(interchange.schema_version, 1);
        interchange
            .loss_report
            .validate_for_strict_import()
            .unwrap();
        validate_strict_import(&serde_json::to_value(&interchange).unwrap()).unwrap();
        assert!(interchange.loss_report.fields.iter().any(|field| {
            field.field == "reaction_provenance" && field.disposition == LossDisposition::Preserved
        }));
        assert!(interchange.source_version.is_none());
        assert!(interchange.source_route_id.is_none());
        assert!(interchange.steps[0].original_node_id.is_none());
        assert!(
            interchange.steps[0]
                .reaction_provenance
                .source_representation_retained
        );
        assert!(matches!(
            interchange.steps[0].reaction_provenance.reaction_evidence,
            Some(ReactionEvidence::SyntheseusReaction { .. })
        ));
    }

    #[test]
    fn strict_import_rejects_missing_loss_report() {
        let value = serde_json::json!({
            "schema_version": 1,
            "route_id": "sha256:test",
            "steps": []
        });
        let error = validate_strict_import(&value).unwrap_err().to_string();
        assert!(error.contains("loss_report"));
    }

    fn stock() -> HashSet<String> {
        ["CC", "O"].into_iter().map(canon).collect()
    }

    fn canon(smiles: &str) -> String {
        to_canonical(&mol_from_smiles(smiles).unwrap())
    }

    fn exported_v1() -> Value {
        let document = RouteDocument {
            source: RouteSource::Renkin,
            root: RouteNode {
                canonical_smiles: canon("CCO"),
                is_stock_leaf: Some(false),
                reaction_evidence: None,
                children: vec![
                    RouteNode {
                        canonical_smiles: canon("CC"),
                        is_stock_leaf: Some(true),
                        reaction_evidence: None,
                        children: vec![],
                    },
                    RouteNode {
                        canonical_smiles: canon("O"),
                        is_stock_leaf: Some(true),
                        reaction_evidence: None,
                        children: vec![],
                    },
                ],
            },
            step_count_collapsed_edges: 1,
        };
        let report =
            audit_document_with_policy(&document, Some(&stock()), Some(&[]), AuditPolicy::Standard);
        serde_json::to_value(from_audit_report("renkin", None, None, &[], &report, None)).unwrap()
    }

    #[test]
    fn strict_reaudit_reconstructs_an_unambiguous_v1_export() {
        let value = exported_v1();
        let result = reauditable_import_v1(&value, &stock(), &[], AuditPolicy::Standard).unwrap();
        assert_eq!(result.imported_route_id, result.recomputed_route_id);
        assert_eq!(result.audit.source, RouteSource::Renkin);
        assert!(result.audit.route_tree_parseable);
    }

    #[test]
    fn strict_reaudit_rejects_route_hash_tampering() {
        let mut value = exported_v1();
        value["steps"][0]["precursors"][0] = Value::String("N".into());
        let error = reauditable_import_v1(&value, &stock(), &[], AuditPolicy::Standard)
            .unwrap_err()
            .to_string();
        assert!(error.contains("route_id mismatch"));
    }

    #[test]
    fn strict_reaudit_rejects_ambiguous_or_missing_v1_topology() {
        let mut duplicate = exported_v1();
        let step = duplicate["steps"][0].clone();
        duplicate["steps"].as_array_mut().unwrap().push(step);
        let error = reauditable_import_v1(&duplicate, &stock(), &[], AuditPolicy::Standard)
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate canonical_node_id"));

        let mut direct = exported_v1();
        direct["steps"] = Value::Array(vec![]);
        let error = reauditable_import_v1(&direct, &stock(), &[], AuditPolicy::Standard)
            .unwrap_err()
            .to_string();
        assert!(error.contains("root identity is absent"));
    }

    #[test]
    fn strict_import_rejects_unversioned_extension_fields() {
        let mut value = exported_v1();
        value["unrecorded_extension"] = Value::Bool(true);
        let error = validate_strict_import(&value).unwrap_err().to_string();
        assert!(error.contains("unknown canonical interchange v1 field"));
    }

    #[test]
    fn v2_preserves_direct_purchase_root_and_reaudits_it() {
        let root = RouteNode {
            canonical_smiles: canon("CC"),
            is_stock_leaf: Some(true),
            reaction_evidence: None,
            children: vec![],
        };
        let document = RouteDocument {
            source: RouteSource::Renkin,
            root,
            step_count_collapsed_edges: 0,
        };
        let report =
            audit_document_with_policy(&document, Some(&stock()), Some(&[]), AuditPolicy::Standard);
        let value = serde_json::to_value(from_document_v2(
            "renkin", None, None, &document, &report, None,
        ))
        .unwrap();
        validate_strict_import_v2(&value).unwrap();
        let reaudited =
            reauditable_import_v2(&value, &stock(), &[], AuditPolicy::Standard).unwrap();
        assert_eq!(reaudited.imported_route_id, reaudited.recomputed_route_id);
        assert!(value["root"]["children"].as_array().unwrap().is_empty());
    }

    #[test]
    fn v2_retains_repeated_precursor_occurrences_and_rejects_path_tampering() {
        let document = RouteDocument {
            source: RouteSource::Renkin,
            root: RouteNode {
                canonical_smiles: canon("CCO"),
                is_stock_leaf: Some(false),
                reaction_evidence: None,
                children: vec![
                    RouteNode {
                        canonical_smiles: canon("CC"),
                        is_stock_leaf: Some(true),
                        reaction_evidence: None,
                        children: vec![],
                    },
                    RouteNode {
                        canonical_smiles: canon("CC"),
                        is_stock_leaf: Some(true),
                        reaction_evidence: None,
                        children: vec![],
                    },
                ],
            },
            step_count_collapsed_edges: 1,
        };
        let report =
            audit_document_with_policy(&document, Some(&stock()), Some(&[]), AuditPolicy::Standard);
        let mut value = serde_json::to_value(from_document_v2(
            "renkin", None, None, &document, &report, None,
        ))
        .unwrap();
        assert_ne!(
            value["root"]["children"][0]["canonical_node_id"],
            value["root"]["children"][1]["canonical_node_id"]
        );
        value["root"]["children"][1]["canonical_node_id"] = Value::String("forged".into());
        assert!(reauditable_import_v2(&value, &stock(), &[], AuditPolicy::Standard).is_err());
    }
}
